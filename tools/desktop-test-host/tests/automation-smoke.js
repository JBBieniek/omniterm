import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { createHash } from 'node:crypto';
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { inspectReady, launchAutomation } from '../client.js';

const [binaryArg, outputArg, mode = 'test'] = process.argv.slice(2);
assert(binaryArg && outputArg, 'Usage: node tests/automation-smoke.js BINARY NEW_OUTPUT_DIR [test|excluded|opt-in|no-pipes]');
const binary = resolve(binaryArg);
const output = resolve(outputArg);
await mkdir(output); // Preserve prior runs; never overwrite evidence.
const evidence = {
  mode, platform: process.platform, node: process.version,
  binary_sha256: createHash('sha256').update(await readFile(binary)).digest('hex'),
  started: new Date().toISOString(), checks: [],
};
const check = (name, detail = {}) => evidence.checks.push({ name, result: 'PASS', ...detail });
let app;
const cameraCommands = [
  {pointer:{phase:'move',button:'left',x:10,y:10}},
  {resize:{width:640,height:480}}, {key:{key:'zoom_in'}},
  'shortcut_guards', {cancel_pan:{lost_capture:true}},
  'advance_layout', {key:{key:'resize_right'}},
  'request_close', {key:{key:'interrupt'}}, {paste:{text:'unauthorized'}},
];
try {
  if (mode === 'excluded') {
    // Production has no automation entry point: even its former flag and a
    // correctly framed token cannot activate the test-host protocol.
    const result = await new Promise((resolveResult, reject) => {
      const child = spawn(binary, ['--automation'], { stdio: ['pipe', 'pipe', 'pipe'] });
      let stdout = '', stderr = '', stopping = false;
      const timeout = setTimeout(() => { stopping = true; child.kill(); }, 2000);
      child.on('error', error => { clearTimeout(timeout); reject(error); });
      child.stdout.on('data', chunk => { stdout += chunk; });
      child.stderr.on('data', chunk => { stderr += chunk; });
      child.on('close', code => {
        clearTimeout(timeout);
        if (!stopping) reject(new Error(`App exited before the exclusion check (${code}): ${stderr}`));
        else resolveResult({ stdout, stderr });
      });
      child.stdin.on('error', () => {});
      child.stdin.write(JSON.stringify({ hello: '0'.repeat(64) }) + '\n');
    });
    assert.equal(result.stdout, '');
    assert.equal(result.stderr, '');
    const bytes = await readFile(binary);
    for (const marker of ['private-stdio', '__omniterm_test_events', 'point_outside_webview']) {
      assert.equal(bytes.includes(Buffer.from(marker)), false, `Production contains ${marker}`);
      assert.equal(bytes.includes(Buffer.from(marker, 'utf16le')), false, `Production contains wide ${marker}`);
    }
    check('ordinary_build_has_no_channel_or_test_markers', { observation_ms: 2000 });
  } else if (mode === 'opt-in' || mode === 'no-pipes') {
    const result = await new Promise((resolveResult, reject) => {
      const child = spawn(binary, mode === 'opt-in' ? [] : ['--automation'], { stdio: mode === 'opt-in' ? ['pipe', 'pipe', 'pipe'] : ['ignore', 'ignore', 'pipe'] });
      let stdout = '', stderr = '';
      const timeout = setTimeout(() => { child.kill(); reject(new Error('Production exclusion timed out')); }, 10000);
      child.on('error', reject);
      child.stdout?.on('data', chunk => { stdout += chunk; });
      child.stderr.on('data', chunk => { stderr += chunk; });
      child.on('close', code => { clearTimeout(timeout); resolveResult({ code, stdout, stderr }); });
      child.stdin?.on('error', () => {});
      child.stdin?.end(JSON.stringify({ hello: '0'.repeat(64) }) + '\n');
    });
    assert.equal(result.code, 2);
    assert.equal(result.stdout, '');
    assert.match(result.stderr, mode === 'opt-in' ? /test host requires explicit --automation launch/ : /automation requires private pipes/);
    check(mode === 'opt-in' ? 'test_host_requires_opt_in' : 'test_launch_requires_private_pipes', result);
  } else {
    assert.equal(mode, 'test');
    app = await launchAutomation(binary);
    assert.deepEqual(app.hello, { result: { protocol: 1, transport: 'private-stdio' } });
    check('private_pipe_handshake');
    assert.deepEqual(await app.request('inspect', 'incorrect'), { error: 'unauthorized' });
    assert.deepEqual(await app.raw('{"command":"inspect"}'), { error: 'invalid_request' });
    assert.deepEqual(await app.raw(JSON.stringify({ token: app.token, command: 'inspect', path: 'forbidden' })), { error: 'invalid_request' });
    assert.deepEqual(await app.request('execute'), { error: 'invalid_request' });
    check('unauthorized_missing_extra_unknown_rejected');
    const state = await inspectReady(app);
    assert.deepEqual(state.layout, { api_version: 1, revision: '0', grid: { columns: 48, rows: 32, cell_px: 64 }, blocks: [
      {id: '1', title: 'Placeholder 1', rect: {x:0,y:0,width:12,height:8}},
      {id: '2', title: 'Placeholder 2', rect: {x:12,y:0,width:8,height:8}},
    ] });
    assert.equal(state.rendered.rendered_revision, state.layout.revision);
    assert.match(state.rendered.status, /Workspace loaded/);
    assert.equal(state.rendered.block_count, '2 blocks');
    assert.equal(state.rendered.grid, '48 columns × 32 rows');
    assert.equal(state.rendered.cell, '64 px / cell');
    assert.equal(state.rendered.test_command_rejected, true);
    assert.equal(state.window.title, 'Omniterm');
    assert.equal(state.window.visible, true);
    await writeFile(resolve(output, 'state.json'), JSON.stringify(state, null, 2));
    check('rust_rendered_revision_window_and_ipc_isolation', { window: state.window, origin: state.rendered.origin });
    const screenshot = await app.request('screenshot');
    assert(!screenshot.error, JSON.stringify(screenshot));
    const png = Buffer.from(screenshot.result.data, 'base64');
    assert.deepEqual(png.subarray(0, 8), Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]));
    assert.equal(png.readUInt32BE(16), state.window.width);
    assert.equal(png.readUInt32BE(20), state.window.height);
    await writeFile(resolve(output, 'webview.png'), png);
    check('native_webview_screenshot', { bytes: png.length, width: png.readUInt32BE(16), height: png.readUInt32BE(20) });
    assert.deepEqual(await app.request({ click: { x: state.window.width, y: 0 } }), { error: 'point_outside_webview' });
    assert.deepEqual(await app.request({ click: { x: 100, y: 100 } }, 'wrong'), { error: 'unauthorized' });
    assert.deepEqual(await app.request({ text: { text: 'unauthorized' } }, 'wrong'), { error: 'unauthorized' });
    for (const command of cameraCommands) assert.deepEqual(await app.request(command, 'wrong'), {error:'unauthorized'});
    assert.deepEqual(await app.request({ text: { text: 'x'.repeat(257) } }), { error: 'invalid_text' });
    assert.deepEqual(await app.request({ key: { key: 'host_shortcut' } }), { error: 'invalid_request' });
    assert.deepEqual(await app.request({ click: { x: 100, y: 100 } }), { result: { clicked: true } });
    assert.deepEqual(await app.request({ key: { key: 'tab' } }), { result: { key_sent: true } });
    const after = await app.request('inspect');
    assert(!after.error, JSON.stringify(after));
    assert.deepEqual(after.result.layout, state.layout);
    assert.equal(after.result.rendered.rendered_revision, state.layout.revision);
    assert.equal(after.result.rendered.input_events.clicks, state.rendered.input_events.clicks + 1);
    assert.equal(after.result.rendered.input_events.key_downs, state.rendered.input_events.key_downs + 1);
    assert.equal(after.result.rendered.input_events.click_trusted, true);
    assert.equal(after.result.rendered.input_events.key_trusted, true);
    await writeFile(resolve(output, 'after-steering.json'), JSON.stringify(after.result, null, 2));
    check('bounded_engine_click_key_and_rust_cross_check', { events: after.result.rendered.input_events });
    const oldToken = app.token;
    assert.deepEqual(await app.request('revoke'), { result: { revoked: true } });
    assert.deepEqual(await app.request('inspect'), { error: 'unauthorized' });
    assert.deepEqual(await app.request('close'), { error: 'unauthorized' });
    assert.deepEqual(await app.request({ click: { x: 1, y: 1 } }), { error: 'unauthorized' });
    assert.deepEqual(await app.request({ text: { text: 'revoked' } }), { error: 'unauthorized' });
    for (const command of cameraCommands) assert.deepEqual(await app.request(command), {error:'unauthorized'});
    check('revocation_denies_inspection_and_close');
    await app.stop(); // Revoked session cannot close; runner owns this child.
    app = await launchAutomation(binary);
    assert.notEqual(app.token, oldToken);
    assert.deepEqual(await app.request('inspect', oldToken), { error: 'unauthorized' });
    assert.deepEqual(await app.request('close'), { result: { closing: true } });
    assert.equal((await app.closed).code, 0);
    check('fresh_launch_token_and_authenticated_clean_close');
    app = await launchAutomation(binary, { hello: 'invalid' });
    assert.deepEqual(app.hello, { error: 'invalid_handshake' });
    await app.stop();
    check('invalid_handshake_rejected');
    app = await launchAutomation(binary);
    assert.deepEqual(await app.raw('x'.repeat(4097)), { error: 'request_too_large' });
    await assert.rejects(app.raw(JSON.stringify({ token: app.token, command: 'inspect' }), 500), /timeout/);
    await app.stop();
    check('oversized_request_closes_channel');
  }
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
