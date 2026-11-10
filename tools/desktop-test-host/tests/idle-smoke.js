import assert from 'node:assert/strict';
import {spawn} from 'node:child_process';
import {createHash} from 'node:crypto';
import {mkdir,readFile,writeFile} from 'node:fs/promises';
import {dirname,join,resolve} from 'node:path';
import {inspectReady,launchAutomation} from '../client.js';
import {idleSignature,resourceMetrics} from '../benchmark-report.js';

const [binaryArg,outputArg,powershellArg]=process.argv.slice(2);
assert(binaryArg&&outputArg&&powershellArg,'Usage: idle-smoke.js HOST NEW_DIR POWERSHELL');
const binary=resolve(binaryArg),output=resolve(outputArg);await mkdir(output);
const oldProfile=process.env.WEBVIEW2_USER_DATA_FOLDER;process.env.WEBVIEW2_USER_DATA_FOLDER=join(output,'profile');
await writeFile(join(output,'cold.ps1'),`param([string]$TaskName,[switch]$Finish)
$ErrorActionPreference='Stop'
[Console]::WriteLine('ARMED_'+$TaskName)
Start-Sleep -Seconds 90
[DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds() | Set-Content ($TaskName+'.time') -Encoding ASCII
[Console]::WriteLine('AWAKE_'+$TaskName)
if ($Finish) {exit}
`);
const hash=async path=>createHash('sha256').update(await readFile(path)).digest('hex');
const evidence={result:'RUNNING',started:new Date().toISOString(),platform:process.platform,node:process.version,
 binary_sha256:await hash(binary),helper_sha256:await hash(join(dirname(binary),'omni.exe')),checks:[],idle_observations:[],
 measurements:{cold_output_ms:[],input_ms:[]},focus_control:'WebView2 document focus emulation in test host; OS foreground/physical keyboard not measured',limits:'Five sequential >=60s controls, same Session across mounted/hidden/frozen; test-host custom-protocol fetch counts validated against known actions; observer and workload order effects; epoch-ms cold timings include file/output/render/inspection, not a precision hardware latency or power claim'};
let app,probe,expected={clicks:0,keys:0};
const pause=ms=>new Promise(done=>setTimeout(done,ms));
const state=()=>inspectReady(app);
const terminal=s=>s.rendered.terminals[0];
const save=()=>writeFile(join(output,'result.json'),JSON.stringify(evidence,null,2)+'\n');
const counts=s=>{assert.equal(s.rendered.input_events.clicks,expected.clicks,'Unrequested click');assert.equal(s.rendered.input_events.key_downs,expected.keys,'Unrequested key');};
async function until(test,timeout=15000){let s;const end=Date.now()+timeout;do{s=await state();counts(s);if(!s.rendered.busy&&test(s))return s;await pause(20);}while(Date.now()<end);await writeFile(join(output,'timeout.json'),JSON.stringify(s,null,2));throw Error('Idle workload state timeout');}
async function action(command,clicks=0,keys=0){counts(await state());const r=await app.request(command);assert(!r.error,JSON.stringify(r));expected.clicks+=clicks;expected.keys+=keys;return until(s=>s.rendered.input_events.clicks===expected.clicks&&s.rendered.input_events.key_downs===expected.keys);}
const click=r=>{assert(r&&!r.disabled);return action({click:{x:Math.floor(r.x+Math.min(r.width/2,35)),y:Math.floor(r.y+Math.min(r.height/2,12))}},1);};
const key=name=>action({key:{key:name}},0,1);
async function command(code,needle){const before=(await state()).rendered.terminal_requests.other;await click(terminal(await state()).input);await until(s=>terminal(s).focused);await action({text:{text:code}});const start=performance.now();await key('enter');if(needle)await until(s=>terminal(s)?.text.includes(needle));assert((await state()).rendered.terminal_requests.other>before,'Known input bypassed request counter');return performance.now()-start;}
function signature(s){return {...idleSignature(s),screens:s.screens.map(t=>({session:t.session_id,revision:t.revision,state:t.state,rows:t.rows,cols:t.cols})),views:s.rendered.terminals.map(t=>({session:t.session_id,focused:t.focused,reading:t.reading,offset:t.history_offset,text:t.reading?t.reading_text:t.text})),selected:s.rendered.selected_id,focused:s.rendered.focused,visibility:s.rendered.visibility};}
function sample(name){
 const child=spawn(resolve(powershellArg),['-NoProfile','-ExecutionPolicy','Bypass','-File',resolve('tools/desktop-test-host/tests/resource-probe.ps1'),'-RootProcessId',String(app.child.pid),'-Seconds','60'],{windowsHide:true,stdio:['ignore','pipe','pipe']});let stdout='',stderr='';child.stdout.on('data',b=>stdout+=b);child.stderr.on('data',b=>stderr+=b);
 const closed=new Promise((done,reject)=>{child.once('error',reject);child.once('close',async code=>{try{await writeFile(join(output,name+'.json'),stdout);assert.equal(code,0,stderr);const raw=JSON.parse(stdout);assert.equal(raw.samples.length,61);resourceMetrics(raw,60);done(raw);}catch(e){reject(e);}});});closed.catch(()=>{});return {closed};
}
async function idle(name){
 await pause(600);const first=await state();counts(first);const before=signature(first),requestStart=first.rendered.terminal_requests;
 assert(requestStart&&Object.keys(requestStart).length===3,'Test-host request counters unavailable');
 await writeFile(join(output,'progress.json'),JSON.stringify({phase:name,status:'SAMPLING',started:new Date().toISOString()}));
 probe=sample(name);const raw=await probe.closed;const last=await state();counts(last);const after=signature(last);assert.deepEqual(after,before,'Disturbed idle control');
 const requests=Object.fromEntries(Object.keys(requestStart).map(k=>[k,last.rendered.terminal_requests[k]-requestStart[k]]));assert(Object.values(requests).every(n=>Number.isInteger(n)&&n>=0));
 evidence.idle_observations.push({name,before,after,requests,elapsed_seconds:raw.duration_ms/1000});await save();return last;
}
async function cold(name,reading=false){
 let s=await until(s=>reading?terminal(s)?.state==='exited':terminal(s)?.text.includes('AWAKE_'+name),60000);
 const observed=Date.now(),emitted=Number((await readFile(join(output,name+'.time'),'utf8')).trim());assert(Number.isSafeInteger(emitted)&&emitted<=observed&&observed-emitted<15000,'Cold clock/observation invalid');evidence.measurements.cold_output_ms.push(observed-emitted);
 if(reading){assert.equal(terminal(s).reading_text,reading.text);assert.equal(terminal(s).selection_length,reading.selection);await click(terminal(s).live);s=await until(s=>terminal(s).text.includes('AWAKE_'+name)&&s.screens[0].output_eof);}
 return s;
}
async function shot(name){const r=await app.request('screenshot');assert(!r.error);await writeFile(join(output,name+'.png'),Buffer.from(r.result.data,'base64'));await writeFile(join(output,name+'.state.json'),JSON.stringify(await state(),null,2));}
function check(name){evidence.checks.push({name,result:'PASS'});}
try{
 app=await launchAutomation(binary,{cwd:output});await state();await action('focus_probe');await action({resize:{width:1280,height:960}});await until(s=>s.window.width===1280&&s.window.height===960&&s.rendered.focused&&s.rendered.visibility==='visible');
 // Let app launch/controller return settle before measuring foreground state.
 await pause(15000);await idle('empty');check('empty_control_has_no_session_or_activity');
 await click((await state()).rendered.controls.new_terminal);await until(s=>terminal(s)?.state==='running'&&terminal(s).cols>50);assert((await state()).rendered.terminal_requests.other>0,'Counter did not observe a known start/resize command');
 await command('.\\cold.ps1 live','ARMED_live');const source=await state();await click(source.rendered.blocks.find(b=>b.id===source.layout.blocks.find(b=>b.session_id==='1').id).header);
 await until(s=>!terminal(s).focused);await idle('visible');await cold('live');check('long_idle_visible_output_is_observed_without_input');
 for(let i=0;i<10;i++)evidence.measurements.input_ms.push(await command(`'IDLE_LAT_'+'${i}'`,'IDLE_LAT_'+i));
 await idle('focused');check('ten_input_round_trips_and_separate_focused_cursor_control');
 let s=await state();const id=s.layout.blocks.find(b=>b.session_id==='1').id;await click(s.rendered.blocks.find(b=>b.id===id).header);await click((await state()).rendered.controls.remove);await until(s=>!s.rendered.terminals.length);
 await idle('hidden');assert.equal((await state()).sessions[0].session_id,'1');assert.equal((await state()).sessions[0].state,'running');check('hidden_control_preserves_same_live_session');
 await click((await state()).rendered.controls.reopen);await until(s=>terminal(s)?.state==='running'&&terminal(s).view_ready);await command('.\\cold.ps1 frozen; exit','ARMED_frozen');await click(terminal(await state()).read);await until(s=>terminal(s).reading&&terminal(s).view_ready);
 await key('select_all');const frozen=terminal(await state()).reading_text,selection=terminal(await state()).selection_length;
 await idle('reading');s=await cold('frozen',{text:frozen,selection});assert.equal(s.sessions[0].state,'exited');assert.equal(s.sessions[0].session_id,'1');check('frozen_control_receives_exit_and_final_output_after_long_idle');
 assert(frozen.includes('ARMED_frozen'));assert(selection>0);await shot('idle-final');assert.equal(evidence.idle_observations.length,5);assert.equal(evidence.measurements.cold_output_ms.length,2);check('all_controls_kept_geometry_focus_input_output_and_authority_stable');
 await app.request('close');assert.equal((await app.closed).code,0);evidence.result='PASS';console.log('PASS: six idle/control groups');
}catch(error){evidence.result='FAIL';evidence.failure=String(error);if(app?.child.exitCode===null){try{await shot('failure');}catch{}}throw error;}
finally{if(probe)await probe.closed.catch(()=>{});if(app?.child.exitCode===null)await app.stop();evidence.finished=new Date().toISOString();await save();if(oldProfile===undefined)delete process.env.WEBVIEW2_USER_DATA_FOLDER;else process.env.WEBVIEW2_USER_DATA_FOLDER=oldProfile;}
