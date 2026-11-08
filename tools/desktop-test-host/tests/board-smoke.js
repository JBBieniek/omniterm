import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { inspectReady, launchAutomation } from '../client.js';

const [binaryArg, outputArg] = process.argv.slice(2);
assert(binaryArg && outputArg, 'Usage: node tests/board-smoke.js TEST_HOST NEW_OUTPUT_DIR');
const binary = resolve(binaryArg), output = resolve(outputArg);
await mkdir(output);
const evidence = { platform: process.platform, node: process.version, started: new Date().toISOString(),
  binary_sha256: createHash('sha256').update(await readFile(binary)).digest('hex'), checks: [] };
const check = (name, detail = {}) => evidence.checks.push({ name, result: 'PASS', ...detail });
let app;

function reconciled(state) {
  assert.equal(state.rendered.rendered_revision, state.layout.revision);
  assert.equal(state.rendered.blocks.length, state.layout.blocks.length);
  assert.equal(state.rendered.world.child_count, state.layout.blocks.length);
  for (const block of state.layout.blocks) {
    const node = state.rendered.blocks.find(node => node.id === block.id);
    assert(node, `Missing rendered block ${block.id}`);
    assert.equal(node.title, block.title);
    assert.equal(node.left, block.rect.x * state.layout.grid.cell_px);
    assert.equal(node.top, block.rect.y * state.layout.grid.cell_px);
    assert.equal(node.width, block.rect.width * state.layout.grid.cell_px);
    assert.equal(node.height, block.rect.height * state.layout.grid.cell_px);
    assert.equal(node.injected_elements, 0);
  }
}

async function waitFor(predicate) {
  for (let attempt = 0; attempt < 50; attempt++) {
    const state = await inspectReady(app);
    if (!state.rendered.busy && predicate(state)) { reconciled(state); return state; }
    await new Promise(resolve => setTimeout(resolve, 100));
  }
  throw new Error('UI did not reach the expected state');
}

async function click(rect) {
  assert(rect && !rect.disabled, 'Control must be present and enabled');
  assert.deepEqual(await app.request({click: {x:Math.floor(rect.x + Math.min(rect.width / 2, 80)), y:Math.floor(rect.y + Math.min(rect.height / 2, 20))}}), {result:{clicked:true}});
}

async function screenshot(name) {
  const response = await app.request('screenshot');
  assert(!response.error, JSON.stringify(response));
  const bytes = Buffer.from(response.result.data, 'base64');
  assert.equal(bytes.readUInt32BE(16), 1024);
  assert.equal(bytes.readUInt32BE(20), 700);
  await writeFile(resolve(output, name + '.png'), bytes);
}

try {
  app = await launchAutomation(binary);
  let state = await waitFor(state => state.layout.revision === '0');
  assert.deepEqual(state.layout.grid, {columns:48,rows:32,cell_px:64});
  assert.deepEqual(state.layout.blocks.map(b => b.id), ['1','2']);
  assert.equal(state.rendered.world.width, 3072);
  assert.equal(state.rendered.world.height, 2048);
  assert.deepEqual(state.rendered.world.background_size.split(',').map(size => size.trim()), ['64px 64px', '64px 64px']);
  assert.match(state.rendered.world.background_image, /linear-gradient/);
  assert.equal(state.rendered.selected_id, null);
  assert.equal(state.rendered.controls.remove.disabled, true);
  const seed = state.layout.blocks;
  await writeFile(resolve(output, 'initial.json'), JSON.stringify(state, null, 2));
  await screenshot('initial');
  check('finite_css_grid_and_two_authoritative_frames', {world:state.rendered.world, window:state.window});

  await click(state.rendered.blocks[0].viewport);
  state = await waitFor(state => state.rendered.selected_id === '1');
  assert.equal(state.layout.revision, '0');
  assert.equal(state.rendered.blocks[0].selected, true);
  assert.equal(state.rendered.controls.remove.disabled, false);
  check('selection_is_local_and_does_not_mutate_layout');

  await click(state.rendered.controls.title);
  const literal = '<img src=x onerror=alert(1)>';
  assert.deepEqual(await app.request({text:{text:literal}}), {result:{text_sent:true}});
  // Native insertion keeps existing text; verify the supplied literal stays inert.
  state = await inspectReady(app);
  await click(state.rendered.controls.add);
  state = await waitFor(state => state.layout.revision === '1');
  assert.equal(state.layout.blocks.length, 3);
  assert.deepEqual(state.layout.blocks.slice(0, 2), seed);
  const added = state.layout.blocks[2];
  assert.equal(added.id, '3');
  assert(added.title.includes(literal));
  assert.deepEqual(added.rect, {x:20,y:0,width:12,height:8});
  assert.equal(state.rendered.selected_id, '3');
  await writeFile(resolve(output, 'added.json'), JSON.stringify(state, null, 2));
  check('native_title_add_inert_text_and_stable_ids', {id:added.id, rect:added.rect});

  await click(state.rendered.controls.remove);
  state = await waitFor(state => state.layout.revision === '2');
  assert.deepEqual(state.layout.blocks, seed);
  assert.equal(state.rendered.selected_id, null);
  assert.equal(state.rendered.controls.remove.disabled, true);
  assert.equal(state.rendered.active.test_id, 'add-placeholder');
  assert.deepEqual(await app.request({key:{key:'tab'}}), {result:{key_sent:true}});
  state = await waitFor(state => state.rendered.active.block_id === '1');
  assert.equal(state.rendered.active.focus_visible, true);
  await screenshot('keyboard-focus');
  check('remove_placement_and_visible_keyboard_focus');

  await click(state.rendered.controls.add);
  state = await waitFor(state => state.layout.revision === '3');
  assert.equal(state.layout.blocks[2].id, '4');
  assert.deepEqual(state.layout.blocks[2].rect, added.rect);
  check('removed_ids_are_not_reused_and_gaps_are_filled');

  for (let attempt = 0; attempt < 32 && !state.rendered.error.includes('No room'); attempt++) {
    const revision = state.layout.revision;
    await click(state.rendered.controls.add);
    state = await waitFor(state => state.layout.revision !== revision || state.rendered.error.includes('No room'));
  }
  assert.match(state.rendered.error, /No room/);
  const full = state.layout;
  await click(state.rendered.controls.add);
  state = await waitFor(state => state.rendered.error.includes('No room'));
  assert.deepEqual(state.layout, full);
  assert.deepEqual(state.layout.blocks.slice(0, 2), seed);
  await writeFile(resolve(output, 'full.json'), JSON.stringify(state, null, 2));
  await screenshot('no-room');
  check('no_room_is_visible_and_preserves_layout', {blocks:full.blocks.length, revision:full.revision});
  await click(state.rendered.controls.title);
  assert.deepEqual(await app.request({text:{text:'é'.repeat(128)}}), {result:{text_sent:true}});
  state = await inspectReady(app);
  await click(state.rendered.controls.add);
  state = await waitFor(state => state.rendered.error.includes('256 UTF-8 bytes'));
  assert.deepEqual(state.layout, full);
  await writeFile(resolve(output, 'invalid-title.json'), JSON.stringify(state, null, 2));
  check('utf8_title_limit_is_visible_and_preserves_layout');
  assert.deepEqual(await app.request('close'), {result:{closing:true}});
  assert.equal((await app.closed).code, 0);
  check('test_owned_host_closes_cleanly');
  evidence.result = 'PASS';
} catch (error) {
  evidence.result = 'FAIL';
  evidence.error = String(error);
  process.exitCode = 1;
} finally {
  if (app?.child.exitCode === null && app?.child.signalCode === null) await app.stop();
  evidence.finished = new Date().toISOString();
  await writeFile(resolve(output, 'result.json'), JSON.stringify(evidence, null, 2));
  console.log(JSON.stringify(evidence, null, 2));
}
