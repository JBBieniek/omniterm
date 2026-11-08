import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { inspectReady, launchAutomation } from '../client.js';

const [binaryArg, outputArg] = process.argv.slice(2);
assert(binaryArg && outputArg, 'Usage: node tests/camera-smoke.js TEST_HOST NEW_OUTPUT_DIR');
const binary = resolve(binaryArg), output = resolve(outputArg);
await mkdir(output);
const evidence = {platform:process.platform, node:process.version, started:new Date().toISOString(),
  binary_sha256:createHash('sha256').update(await readFile(binary)).digest('hex'), checks:[]};
const near = (actual, expected, tolerance = .02) => assert(Math.abs(actual - expected) < tolerance, `${actual} != ${expected}`);
const check = (name, detail = {}) => evidence.checks.push({name, result:'PASS', ...detail});
let app, original, baseline;

function unchanged(state) {
  assert.deepEqual(state.layout, original, 'Camera must not change Rust snapshot/revision');
  assert.equal(state.rendered.rendered_revision, original.revision);
  assert.equal(state.rendered.viewport.scroll_x, 0);
  assert.equal(state.rendered.viewport.scroll_y, 0);
  near(state.rendered.page.dpr, baseline.rendered.page.dpr, 1e-9);
  near(state.rendered.page.scale, baseline.rendered.page.scale, 1e-9);
  const {origin,zoom} = state.rendered.camera, factor = zoom / 100;
  for (const block of original.blocks) {
    const node = state.rendered.blocks.find(node => node.id === block.id);
    assert.equal(node.left, block.rect.x * original.grid.cell_px);
    assert.equal(node.top, block.rect.y * original.grid.cell_px);
    near(node.viewport.x - state.rendered.viewport.x, (node.left - origin.x) * factor);
    near(node.viewport.y - state.rendered.viewport.y, (node.top - origin.y) * factor);
    near(node.viewport.width, node.width * factor);
    near(node.viewport.height, node.height * factor);
  }
}

async function state() {
  await new Promise(resolve => setTimeout(resolve, 30));
  const result = await inspectReady(app);
  unchanged(result);
  return result;
}
async function click(rect) {
  assert(rect && !rect.disabled);
  assert.deepEqual(await app.request({click:{x:Math.floor(rect.x + rect.width / 2),y:Math.floor(rect.y + rect.height / 2)}}), {result:{clicked:true}});
}
async function key(key) { assert.deepEqual(await app.request({key:{key}}), {result:{key_sent:true}}); }
async function pointer(phase, button, x, y) {
  assert.deepEqual(await app.request({pointer:{phase,button,x:Math.floor(x),y:Math.floor(y)}}), {result:{pointer_sent:true}});
}
const centerWorld = s => ({x:s.rendered.camera.origin.x + s.rendered.viewport.width / 2 / (s.rendered.camera.zoom / 100),
  y:s.rendered.camera.origin.y + s.rendered.viewport.height / 2 / (s.rendered.camera.zoom / 100)});
async function zoom(s, action, expected) {
  const anchor = centerWorld(s);
  await action();
  const next = await state();
  near(next.rendered.camera.zoom, expected);
  const after = centerWorld(next);
  near(after.x, anchor.x); near(after.y, anchor.y);
  assert.deepEqual(next.rendered.page.toolbar, s.rendered.page.toolbar, 'Toolbar is outside camera transform');
  return next;
}
async function shot(name) {
  const response = await app.request('screenshot');
  assert(!response.error, JSON.stringify(response));
  await writeFile(resolve(output, name + '.png'), Buffer.from(response.result.data, 'base64'));
  await writeFile(resolve(output, name + '.json'), JSON.stringify(await state(), null, 2));
}
async function reset(s) {
  await click(s.rendered.controls.reset);
  s = await state();
  near(s.rendered.camera.zoom, 100);
  near(s.rendered.camera.origin.x, 0); near(s.rendered.camera.origin.y, 0);
  return s;
}

try {
  app = await launchAutomation(binary);
  baseline = await inspectReady(app);
  original = baseline.layout;
  let s = await state();
  await shot('initial');
  check('initial_camera_matches_rust_rectangles', {window:s.window, viewport:s.rendered.viewport});

  for (const level of [67,50,33,25]) s = await zoom(s, () => click(s.rendered.controls.zoom_out), level);
  assert.equal(s.rendered.controls.zoom_out.disabled, true);
  s = await zoom(s, () => key('zoom_out'), 25);
  await shot('zoom-25');
  for (const level of [33,50,67,100,125,150,175,200]) s = await zoom(s, () => click(s.rendered.controls.zoom_in), level);
  assert.equal(s.rendered.controls.zoom_in.disabled, true);
  s = await zoom(s, () => key('zoom_in'), 200);
  await shot('zoom-200');
  check('all_nine_button_zoom_levels_center_anchor_and_endpoint_clamps');
  for (const [name,expected] of [['zoom_out',175],['numpad_subtract',150],['zoom_reset',100],['zoom_in',125],['zoom_plus',150],['numpad_add',175]]) {
    s = await zoom(s, () => key(name), expected);
  }
  check('native_ctrl_equals_plus_minus_zero_and_numpad_keep_page_scale');

  s = await reset(s);
  await click(s.rendered.controls.pan);
  s = await state();
  assert.equal(s.rendered.pan_tool, true);
  // Pan tool works over a block, and captured movement can enter the toolbar.
  const start = {x:100,y:s.rendered.viewport.y + 90};
  await pointer('down','left',start.x,start.y);
  await pointer('move','left',start.x + 120,start.y + 60);
  s = await state();
  assert.equal(s.rendered.panning, true);
  near(s.rendered.camera.origin.x, -120); near(s.rendered.camera.origin.y, -60);
  assert.equal(s.rendered.selected_id, null);
  await pointer('up','left',start.x + 120,start.y + 60);
  s = await state();
  assert.equal(s.rendered.panning, false);
  await shot('panned');
  check('pan_tool_drag_over_block_has_no_layout_or_selection_effect');

  for (const cancellation of ['escape','cancel','lost']) {
    const before = s.rendered.camera;
    await pointer('down','left',start.x,start.y);
    await pointer('move','left',start.x + 60,30);
    const moving = await state();
    assert.equal(moving.rendered.panning, true);
    assert.notDeepEqual(moving.rendered.camera, before);
    if (cancellation === 'escape') await key('escape');
    else assert.deepEqual(await app.request({cancel_pan:{lost_capture:cancellation === 'lost'}}), {result:{synthetic:true}});
    s = await state();
    assert.equal(s.rendered.panning, false);
    near(s.rendered.camera.origin.x, before.origin.x); near(s.rendered.camera.origin.y, before.origin.y);
    await pointer('up','left',start.x + 60,30);
  }
  check('captured_pan_escape_and_synthetic_cancel_lost_capture_restore_view');

  s = await reset(await state());
  s = await zoom(s, () => key('zoom_out'), 67);
  s = await zoom(s, () => key('zoom_out'), 50);
  // Tool primary drag uses inverse scale at 50%.
  const beforePan = s.rendered.camera.origin;
  await pointer('down','left',300,s.rendered.viewport.y + 120);
  await pointer('move','left',380,s.rendered.viewport.y + 160);
  await pointer('up','left',380,s.rendered.viewport.y + 160);
  s = await state();
  near(s.rendered.camera.origin.x, beforePan.x - 160); near(s.rendered.camera.origin.y, beforePan.y - 80);
  await click(s.rendered.controls.pan);
  s = await state();
  assert.equal(s.rendered.pan_tool, false);
  // At 50% this bottom-right point is empty board, away from both seed frames.
  const x = s.rendered.viewport.width - 50, y = s.rendered.viewport.y + s.rendered.viewport.height - 50;
  const beforeMiddle = s.rendered.camera.origin;
  await pointer('down','middle',x,y);
  await pointer('move','middle',x - 80,y - 40);
  await pointer('up','middle',x - 80,y - 40);
  s = await state();
  near(s.rendered.camera.origin.x, beforeMiddle.x + 160); near(s.rendered.camera.origin.y, beforeMiddle.y + 80);
  const beforeBlocked = s.rendered.camera;
  const block = s.rendered.blocks[0].viewport;
  const bx = Math.max(20,block.x + 40), by = Math.max(s.rendered.viewport.y + 20,block.y + 40);
  await pointer('down','middle',bx,by); await pointer('move','middle',bx + 30,by + 30); await pointer('up','middle',bx + 30,by + 30);
  s = await state();
  assert.deepEqual(s.rendered.camera, beforeBlocked);
  check('half_scale_pan_and_middle_empty_board_only');

  const guards = await app.request('shortcut_guards');
  assert.deepEqual(guards, {result:{synthetic:true,composing:false,event_composing:false,modal:false,unchanged:true}});
  s = await state();
  check('synthetic_composition_and_modal_dom_guards', {physical_ime_os_dialog:'NOT TESTED'});
  const cameraBeforeResize = s.rendered.camera;
  assert.deepEqual(await app.request({resize:{width:640,height:480}}), {result:{resized:true}});
  for (let attempt = 0; attempt < 30; attempt++) {
    s = await state();
    if (s.rendered.viewport.width === 640) break;
  }
  assert.equal(s.rendered.viewport.width, 640);
  assert.deepEqual(s.rendered.camera, cameraBeforeResize);
  s = await zoom(s, () => key('zoom_in'), 67);
  await shot('resized');
  assert.deepEqual(await app.request({resize:{width:360,height:300}}), {result:{resized:true}});
  for (let attempt = 0; attempt < 30; attempt++) { s = await state(); if (s.rendered.viewport.width === 360) break; }
  assert.equal(s.rendered.viewport.width, 360);
  assert(s.rendered.viewport.height > 0);
  for (const name of ['pan','zoom_in','zoom_out','reset']) {
    const r = s.rendered.controls[name];
    assert(r.x >= 0 && r.x + r.width <= 360 && r.y >= 0 && r.y + r.height < 300);
  }
  s = await reset(s);
  await shot('narrow-reset');
  check('window_resize_keeps_camera_and_rectangles_toolbar_wraps_and_reset_recovers');
  assert.deepEqual(await app.request({resize:{width:1,height:1}}), {error:'invalid_window_size'});
  assert.deepEqual(await app.request({pointer:{phase:'move',button:'left',x:4000,y:0}}), {error:'point_outside_webview'});
  check('steering_bounds_reject');
  assert.deepEqual(await app.request('close'), {result:{closing:true}});
  assert.equal((await app.closed).code, 0);
  check('authenticated_clean_close');
  evidence.result = 'PASS';
} catch (error) {
  evidence.result = 'FAIL'; evidence.error = String(error); evidence.stack = error.stack; process.exitCode = 1;
  if (app) {
    try { await writeFile(resolve(output,'failure.json'), JSON.stringify(await inspectReady(app),null,2)); } catch {}
  }
} finally {
  if (app?.child.exitCode === null && app?.child.signalCode === null) await app.stop();
  evidence.finished = new Date().toISOString();
  await writeFile(resolve(output,'result.json'), JSON.stringify(evidence,null,2));
  console.log(JSON.stringify(evidence,null,2));
}
