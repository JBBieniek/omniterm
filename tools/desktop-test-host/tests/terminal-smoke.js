import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { spawn } from 'node:child_process';
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { inspectReady, launchAutomation } from '../client.js';

const [binaryArg, outputArg] = process.argv.slice(2);
assert(binaryArg && outputArg, 'Usage: node tests/terminal-smoke.js TEST_HOST NEW_OUTPUT_DIR');
const binary = resolve(binaryArg), output = resolve(outputArg);
await mkdir(output);
const evidence = {platform:process.platform,node:process.version,started:new Date().toISOString(),
  binary_sha256:createHash('sha256').update(await readFile(binary)).digest('hex'),checks:[]};
const check = (name, detail={}) => evidence.checks.push({name,result:'PASS',...detail});
const pause = ms => new Promise(resolve => setTimeout(resolve,ms));
let app;
async function state() { return inspectReady(app); }
async function until(predicate, timeout=15000) {
  const deadline = Date.now()+timeout;
  let s;
  do { s=await state(); if(!s.rendered.busy&&predicate(s)) return s; await pause(50); } while(Date.now()<deadline);
  await writeFile(resolve(output,'failure-state.json'),JSON.stringify(s,null,2));
  throw new Error('Terminal did not reach expected state; see failure-state.json');
}
async function click(r) {
  assert(r&&!r.disabled,'Control must be available');
  const response=await app.request({click:{x:Math.floor(r.x+Math.min(r.width/2,40)),y:Math.floor(r.y+Math.min(r.height/2,16))}});
  assert(!response.error,JSON.stringify(response));
}
async function key(key) { assert.deepEqual(await app.request({key:{key}}),{result:{key_sent:true}}); }
async function command(text) {
  const s=await state(); await click(s.rendered.terminals.findLast(t=>t.state==='running').input);
  assert.deepEqual(await app.request({text:{text}}),{result:{text_sent:true}}); await key('enter');
}
const text = s => s.rendered.terminals[0]?.text ?? '';
async function shot(name,s) {
  const r=await app.request('screenshot'); assert(!r.error,JSON.stringify(r));
  await writeFile(resolve(output,name+'.png'),Buffer.from(r.result.data,'base64'));
  await writeFile(resolve(output,name+'.json'),JSON.stringify(s,null,2));
}
async function start() {
  app=await launchAutomation(binary,{cwd:output});
  let s=await state(); assert.equal(s.sessions.length,0,'Startup must not execute a shell');
  await click(s.rendered.controls.new_terminal);
  s=await until(s=>s.rendered.terminals[0]?.state==='running'&&s.rendered.terminals[0].cols>50);
  return s;
}

// Hold real process handles before triggering quit/crash, avoiding a PID-reuse check.
async function watchProcesses() {
  await command("$PID | Set-Content shell.pid; $p=Start-Process powershell.exe -ArgumentList '-NoProfile','-Command','Start-Sleep 120' -NoNewWindow -PassThru; $p.Id | Set-Content child.pid; 'CHILD_'+'ARMED'");
  await until(s=>s.rendered.terminals.some(t=>t.text.includes('CHILD_ARMED')));
  const ids=await Promise.all(['shell.pid','child.pid'].map(async file=>{
    const id=(await readFile(resolve(output,file),'utf8')).trim(); assert(/^\d+$/.test(id)); return id;
  }));
  const watcher=spawn('C:/Windows/System32/WindowsPowerShell/v1.0/powershell.exe',[
    '-NoLogo','-NoProfile','-Command',
    `$a=[Diagnostics.Process]::GetProcessById(${ids[0]}); $b=[Diagnostics.Process]::GetProcessById(${ids[1]}); Write-Output ARMED; $ae=$a.WaitForExit(6000); $be=$b.WaitForExit(6000); Write-Output "shell=$ae child=$be"; if (!$ae -or !$be) { exit 1 }; Write-Output REAPED`
  ],{stdio:['ignore','pipe','pipe'],windowsHide:true});
  let stdout='',stderr='';
  const closed=new Promise((resolve,reject)=>{watcher.once('error',reject);watcher.once('close',code=>resolve({code,stdout,stderr}));});
  watcher.stdout.on('data',chunk=>{stdout+=chunk;}); watcher.stderr.on('data',chunk=>{stderr+=chunk;});
  const deadline=Date.now()+5000;
  while(!stdout.includes('ARMED')) { assert(Date.now()<deadline,'Process handle watcher did not arm: '+stderr); await pause(25); }
  return {closed}; // Do not let async return await the exit watcher before triggering exit.
}

try {
  let s=await start();
  const session=s.sessions[0].session_id, initialBlock=s.layout.blocks.find(b=>b.session_id===session).id;
  await command("Write-Output ('NATIVE_'+'READY')");
  s=await until(s=>text(s).includes('NATIVE_READY'));
  check('native_engine_input_and_rust_cell_projection');
  await command("Write-Output ('é界é <img src=x> ' + [char]27 + '[31mRED' + [char]27 + '[0m')");
  s=await until(s=>text(s).includes('é界é <img src=x> RED'));
  assert(s.screens[0].lines.flat().some(r=>r.fg==='#cd3131'&&r.text.includes('RED')));
  assert(s.rendered.blocks.every(b=>b.injected_elements===0));
  check('unicode_combining_wide_color_and_inert_markup'); await shot('terminal-unicode',s);
  await click(s.rendered.terminals[0].input);
  assert.deepEqual(await app.request({paste:{text:"Write-Output ('PASTE_'+'é界')"}}),{result:{paste_sent:true}});
  await key('enter'); await until(s=>text(s).includes('PASTE_é界'));
  check('synthetic_clipboard_payload_through_normal_paste_handler');
  const layout=(await state()).layout;
  await key('arrow_up'); await key('arrow_down');
  assert.deepEqual((await state()).layout,layout,'Terminal arrows must not move blocks');
  await key('interrupt'); await pause(150); // Clear any console history line before the next command.
  await command('Start-Sleep -Seconds 30'); await pause(200);
  const interrupted=Date.now(); await key('interrupt');
  await until(s=>text(s).trimEnd().endsWith('>'));
  const interruptPromptMs=Date.now()-interrupted;
  await pause(100); // ConPTY's cancellation can discard input already buffered behind Ctrl+C.
  await command("Write-Output ('AFTER_'+'INTERRUPT')");
  await until(s=>text(s).includes('AFTER_INTERRUPT'));
  check('terminal_keys_keep_layout_and_ctrl_c_interrupts',{prompt_sample_ms:interruptPromptMs,next_command_ms:Date.now()-interrupted,metric:'controller request to observed rendered prompt; single sample, not a latency budget'});
  s=await state(); const size={rows:s.screens[0].rows,cols:s.screens[0].cols};
  await click(s.rendered.controls.zoom_out); await pause(350);
  s=await state(); assert.equal(s.screens[0].rows,size.rows); assert.equal(s.screens[0].cols,size.cols);
  await key('zoom_reset'); s=await state();
  // Reset zoom preserves the camera anchor; reveal the terminal via its normal Show control.
  await click(s.rendered.controls.reopen); s=await state();
  await click(s.rendered.blocks.find(b=>b.id===initialBlock).header); await key('resize_down');
  s=await until(s=>s.screens[0].rows>size.rows);
  const fitted=s.screens[0];
  await command("Write-Output ('SIZE=' + [Console]::WindowWidth + 'x' + [Console]::WindowHeight)");
  await until(s=>text(s).includes(`SIZE=${fitted.cols}x${fitted.rows}`));
  check('camera_does_not_resize_pty_and_block_resize_reaches_shell');
  await command("Start-Sleep -Seconds 2; Write-Output ('WHILE_'+'HIDDEN')");
  s=await state(); await click(s.rendered.controls.remove);
  s=await until(s=>!s.rendered.terminals.length);
  assert.equal(s.sessions[0].session_id,session); assert.equal(s.sessions[0].state,'running');
  s=await until(s=>s.screens[0].lines.flat().map(r=>r.text).join('').includes('WHILE_HIDDEN'));
  await click(s.rendered.controls.reopen);
  s=await until(s=>text(s).includes('WHILE_HIDDEN'));
  assert.equal(s.rendered.terminals[0].session_id,session);
  assert.notEqual(s.layout.blocks.find(b=>b.session_id===session).id,initialBlock);
  check('hide_continues_reading_and_reopen_reuses_same_session');
  await command("1..8000 | ForEach-Object { 'x' * 64 }; Write-Output ('FLOOD_'+'DONE')");
  s=await until(s=>text(s).includes('FLOOD_DONE'),30000);
  assert(s.screens[0].evicted_bytes>0); assert(s.screens[0].raw_bytes<=262144);
  check('native_flood_has_bounded_visible_capture_eviction',{retained:s.screens[0].raw_bytes,evicted:s.screens[0].evicted_bytes});
  await shot('terminal-flood',s);
  await command("while ($true) { 'F' * 64 }"); await pause(250);
  const floodInterrupt=Date.now(); await key('interrupt');
  await until(s=>text(s).trimEnd().endsWith('>'),15000);
  const floodPromptMs=Date.now()-floodInterrupt; await pause(100);
  await command("Write-Output ('FLOOD_'+'INTERRUPTED')");
  await until(s=>text(s).includes('FLOOD_INTERRUPTED'));
  check('ctrl_c_remains_usable_during_unbounded_output',{prompt_sample_ms:floodPromptMs,next_command_ms:Date.now()-floodInterrupt,metric:'controller request to observed rendered prompt; single sample, no general performance claim'});
  await click(s.rendered.controls.terminate);
  s=await until(s=>s.sessions[0].state==='exited'&&s.screens[0].output_eof);
  check('explicit_termination_confirms_exit_and_eof');
  await click(s.rendered.controls.new_terminal);
  await until(s=>s.sessions.length===2&&s.rendered.terminals.some(t=>t.session_id==='2'&&t.state==='running'));
  s=await state(); const second=s.layout.blocks.find(b=>b.session_id==='2');
  await click(s.rendered.blocks.find(b=>b.id===second.id).header);
  const quitProcesses=await watchProcesses();
  assert.deepEqual(await app.request('request_close'),{result:{close_requested:true}});
  s=await until(s=>s.rendered.quit_dialog);
  await click(s.rendered.controls.cancel_quit);
  s=await until(s=>!s.rendered.quit_dialog); assert.equal(s.sessions[1].state,'running');
  check('native_window_close_requires_confirmation_and_cancel_keeps_session');
  await app.request('request_close'); s=await until(s=>s.rendered.quit_dialog);
  await click(s.rendered.controls.confirm_quit); assert.equal((await app.closed).code,0);
  const quitReaped=await quitProcesses.closed; assert.equal(quitReaped.code,0,quitReaped.stdout+quitReaped.stderr);
  await writeFile(resolve(output,'quit-process-handles.json'),JSON.stringify(quitReaped,null,2));
  check('confirmed_quit_stops_live_session');

  s=await start();
  const processes=await watchProcesses();
  await app.stop();
  const reaped=await processes.closed; await writeFile(resolve(output,'crash-process-handles.json'),JSON.stringify(reaped,null,2));
  assert.equal(reaped.code,0,reaped.stdout+reaped.stderr); assert(reaped.stdout.includes('REAPED'));
  check('abrupt_host_death_closes_job_and_owned_shell_descendant_handles');
  evidence.result='PASS'; evidence.finished=new Date().toISOString();
  console.log(`PASS: ${evidence.checks.length} native terminal checks`);
} catch(error) { evidence.result='FAIL'; evidence.failure=String(error); throw error; }
finally { if(app&&app.child.exitCode===null) await app.stop(); await writeFile(resolve(output,'result.json'),JSON.stringify(evidence,null,2)); }
