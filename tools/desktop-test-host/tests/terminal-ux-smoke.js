import assert from 'node:assert/strict';
import {spawn} from 'node:child_process';
import {createHash} from 'node:crypto';
import {mkdir,readFile,writeFile} from 'node:fs/promises';
import {dirname,join,resolve} from 'node:path';
import {performance} from 'node:perf_hooks';
import {inspectReady,launchAutomation} from '../client.js';
import {resourceMetrics} from '../benchmark-report.js';

const [binaryArg,outputArg,powershellArg]=process.argv.slice(2);
assert(binaryArg&&outputArg&&powershellArg,'Usage: terminal-ux-smoke.js HOST NEW_OUT POWERSHELL_EXE');
const binary=resolve(binaryArg),output=resolve(outputArg);await mkdir(output);
const oldProfile=process.env.WEBVIEW2_USER_DATA_FOLDER;process.env.WEBVIEW2_USER_DATA_FOLDER=join(output,'profile');
const hash=async path=>createHash('sha256').update(await readFile(path)).digest('hex');
const evidence={started:new Date().toISOString(),binary_sha256:await hash(binary),helper_sha256:await hash(join(dirname(binary),'omni.exe')),
  platform:process.platform,node:process.version,checks:[],measurements:{history_navigation_ms:[],input_after_history_ms:[],live_snapshot_json_bytes:[]},
  clipboard:'Native selection checked; OS clipboard copy/contents NOT READ OR TESTED',human_ime_other_platforms:'NOT RUN'};
let app,probe,expected={clicks:0,keys:0};
const pause=ms=>new Promise(done=>setTimeout(done,ms));
const check=name=>evidence.checks.push({name,result:'PASS'});
const terminal=s=>s.rendered.terminals[0];
const liveText=s=>s.screens[0]?.lines.map(l=>l.map(r=>r.text).join('')).join('\n')??'';
const state=()=>inspectReady(app);
async function until(predicate,timeout=15000){let s;const end=Date.now()+timeout;do{s=await state();if(!s.rendered.busy&&predicate(s))return s;await pause(20);}while(Date.now()<end);await writeFile(join(output,'timeout.json'),JSON.stringify(s,null,2));throw Error('Terminal UX state timeout');}
function counts(s){assert.equal(s.rendered.input_events.clicks,expected.clicks,'Unrequested click: disturbed native workload');assert.equal(s.rendered.input_events.key_downs,expected.keys,'Unrequested key: disturbed native workload');}
async function action(command,clicks=0,keys=0){counts(await state());const reply=await app.request(command);assert(!reply.error,JSON.stringify(reply));expected.clicks+=clicks;expected.keys+=keys;const s=await until(s=>s.rendered.input_events.clicks>=expected.clicks&&s.rendered.input_events.key_downs>=expected.keys);counts(s);return s;}
async function click(r){assert(r&&!r.disabled,'Control unavailable');return action({click:{x:Math.floor(r.x+Math.min(r.width/2,40)),y:Math.floor(r.y+Math.min(r.height/2,12))}},1);}
const key=name=>action({key:{key:name}},0,1);
async function command(text,needle){let s=await state();await click(terminal(s).input);await until(s=>terminal(s).focused);await action({text:{text}});const began=performance.now();await key('enter');if(needle)await until(s=>terminal(s)?.text.includes(needle));return performance.now()-began;}
async function page(){return until(s=>terminal(s)?.reading&&terminal(s).view_ready&&s.rendered.active.test_id==='terminal-reading');}
async function live(){await click(terminal(await state()).live);return until(s=>!terminal(s).reading&&terminal(s).view_ready&&(terminal(s).focused||terminal(s).state==='exited'));}
async function shot(name){const r=await app.request('screenshot');assert(!r.error);await writeFile(join(output,name+'.png'),Buffer.from(r.result.data,'base64'));await writeFile(join(output,name+'.json'),JSON.stringify(await state(),null,2));}
function resources(){
 const child=spawn(resolve(powershellArg),['-NoProfile','-ExecutionPolicy','Bypass','-File',resolve('tools/desktop-test-host/tests/resource-probe.ps1'),'-RootProcessId',String(app.child.pid),'-Seconds','15'],{stdio:['ignore','pipe','pipe'],windowsHide:true});
 let stdout='',stderr='';child.stdout.on('data',b=>stdout+=b);child.stderr.on('data',b=>stderr+=b);
 const closed=new Promise((done,reject)=>{child.once('error',reject);child.once('close',async code=>{try{await writeFile(join(output,'resources.json'),stdout);assert.equal(code,0,stderr);const raw=JSON.parse(stdout);assert.equal(raw.samples.length,16);resourceMetrics(raw,15);done();}catch(e){reject(e);}});});closed.catch(()=>{});return {closed};
}
try {
 app=await launchAutomation(binary,{cwd:output});let s=await state();assert.equal(s.sessions.length,0);assert.equal(s.git_tasks.length,0);counts(s);
 await action({resize:{width:1280,height:960}});await until(s=>s.window.width===1280&&s.window.height===960);
 for(const id of ['1','2']){await click((await state()).rendered.blocks.find(b=>b.id===id).header);await click((await state()).rendered.controls.remove);await until(s=>!s.layout.blocks.some(b=>b.id===id));}
 await click((await state()).rendered.controls.new_terminal);s=await until(s=>terminal(s)?.state==='running'&&terminal(s).cols>60);
 const session=s.sessions[0].session_id,block=s.layout.blocks[0].id;
 await command("Write-Output ('TYPE_'+'READY é界é <img src=x>')",'TYPE_READY');assert(terminal(await state()).focused);
 await click(terminal(await state()).read);s=await page();assert(terminal(s).reading_text.includes('é界é <img src=x>'));assert(s.rendered.blocks.every(b=>b.injected_elements===0));await key('escape');await until(s=>terminal(s).focused&&!terminal(s).reading);
 await command("1..260 | ForEach-Object { 'ROW_' + $_.ToString('000') }; Write-Output ('HISTORY_'+'READY')",'HISTORY_READY');
 s=await until(s=>terminal(s).history_rows===200);const size={rows:s.screens[0].rows,cols:s.screens[0].cols},layout=s.layout;
 check('real_input_focus_and_bounded_history');probe=resources();
 await click(terminal(s).read);s=await page();assert(terminal(s).reading_text.includes('HISTORY_READY'));assert.equal(terminal(s).input,null);
 await key('home');s=await until(s=>terminal(s).history_offset===200&&terminal(s).view_ready);assert(!terminal(s).reading_text.includes('ROW_001'));
 await key('select_all');s=await state();assert.equal(terminal(s).selection_length,terminal(s).reading_text.length);
 await shot('history-selected');check('oldest_bounded_page_and_native_text_selection_without_clipboard_access');
 const surface=terminal(s).reading_surface;
 await action({wheel:{x:Math.floor(surface.x+40),y:Math.floor(surface.y+40),delta:120}});await until(s=>terminal(s).history_offset===197&&terminal(s).view_ready);
 await action({wheel:{x:Math.floor(surface.x+40),y:Math.floor(surface.y+40),delta:-120}});await until(s=>terminal(s).history_offset===200&&terminal(s).view_ready);
 await key('end');await until(s=>terminal(s).history_offset===0&&terminal(s).view_ready);
 for(let i=0;i<20;i++){const began=performance.now();await key(i%2===0?'page_up':'page_down');const observed=await until(s=>terminal(s).view_ready&&terminal(s).history_offset===(i%2===0?size.rows-1:0));evidence.measurements.history_navigation_ms.push(performance.now()-began);evidence.measurements.live_snapshot_json_bytes.push(Buffer.byteLength(JSON.stringify(observed.screens[0])));}
 s=await state();assert.deepEqual(s.layout,layout);assert.equal(s.screens[0].rows,size.rows);assert.equal(s.screens[0].cols,size.cols);check('wheel_keyboard_and_twenty_page_transitions_preserve_layout_and_pty_size');
 await live();await command("Start-Sleep -Seconds 2; Write-Output ('BACKGROUND_'+'DONE')");await click(terminal(await state()).read);s=await page();const frozen=terminal(s).reading_text;
 assert(!frozen.includes('BACKGROUND_DONE'));await key('select_all');const selected=terminal(await state()).selection_length;
 s=await until(s=>liveText(s).includes('BACKGROUND_DONE'));assert.equal(terminal(s).reading_text,frozen);assert.equal(terminal(s).selection_length,selected);counts(s);check('background_output_continues_while_selected_page_stays_frozen');
 await key('escape');await until(s=>terminal(s).focused&&!terminal(s).reading&&terminal(s).text.includes('BACKGROUND_DONE'));
 for(let i=0;i<10;i++){const n=String(i).padStart(3,'0');evidence.measurements.input_after_history_ms.push(await command(`Write-Output ('UX_LAT_'+'${n}')`,'UX_LAT_'+n));}
 await key('history_up');s=await page();assert(terminal(s).history_offset>0);check('escape_returns_to_live_input_and_ten_round_trips_then_shift_page_up');
 await click((await state()).rendered.blocks.find(b=>b.id===block).header);await key('zoom_out');await pause(200);s=await state();assert.equal(s.rendered.camera.zoom,67);assert.equal(s.screens[0].rows,size.rows);assert.equal(s.screens[0].cols,size.cols);
 await key('zoom_reset');await until(s=>s.rendered.camera.zoom===100);await click((await state()).rendered.controls.reopen);await click((await state()).rendered.blocks.find(b=>b.id===block).header);await key('resize_down');s=await until(s=>s.screens[0].rows>size.rows);assert(terminal(s).reading);check('camera_preserves_pty_and_real_resize_works_while_reading');
 await click((await state()).rendered.controls.remove);s=await until(s=>!s.rendered.terminals.length);assert.equal(s.sessions[0].session_id,session);assert.equal(s.sessions[0].state,'running');
 await click(s.rendered.controls.reopen);s=await until(s=>terminal(s)?.state==='running'&&terminal(s).view_ready);assert.equal(s.sessions.length,1);assert.equal(terminal(s).session_id,session);assert(!terminal(s).reading);check('removing_reading_view_preserves_session_and_reopen_returns_live');
 await click(terminal(s).input);await until(s=>terminal(s).focused);await shot('terminal-live');
 await command("Start-Sleep -Seconds 2; Write-Output ('FINAL_'+'OUTPUT'); exit");await click(terminal(await state()).read);s=await page();const beforeExit=terminal(s).reading_text;
 s=await until(s=>s.sessions[0].state==='exited'&&s.screens[0].output_eof&&terminal(s).state==='exited');assert.equal(terminal(s).reading_text,beforeExit);
 await key('end');s=await page();assert(terminal(s).reading_text.includes('FINAL_OUTPUT'));await key('page_up');await page();await shot('history-after-exit');check('exit_and_eof_update_status_while_selection_page_stays_frozen_and_history_remains_readable');
 await probe.closed;counts(await state());assert.equal((await state()).git_tasks.length,0);
 await app.request('close');assert.equal((await app.closed).code,0);app=null;check('owned_shutdown_after_finite_profile_workload');evidence.result='PASS';console.log('PASS: '+evidence.checks.length+' terminal UX checks');
} catch(error){evidence.result='FAIL';evidence.failure=String(error);if(app){try{await shot('failure');}catch{}}throw error;}
finally{if(app&&app.child.exitCode===null)await app.stop();if(probe)await probe.closed.catch(e=>{evidence.resource_error=String(e);});if(oldProfile===undefined)delete process.env.WEBVIEW2_USER_DATA_FOLDER;else process.env.WEBVIEW2_USER_DATA_FOLDER=oldProfile;evidence.finished=new Date().toISOString();await writeFile(join(output,'result.json'),JSON.stringify(evidence,null,2));}
