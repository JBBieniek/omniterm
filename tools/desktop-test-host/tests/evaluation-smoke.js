import assert from 'node:assert/strict';
import {spawn,spawnSync} from 'node:child_process';
import {createHash} from 'node:crypto';
import {mkdir,readFile,writeFile} from 'node:fs/promises';
import {resolve,dirname,join} from 'node:path';
import {performance} from 'node:perf_hooks';
import {inspectReady,launchAutomation} from '../client.js';
import {idleSignature,resourceMetrics} from '../benchmark-report.js';

const [binaryArg,outputArg,gitArg,mode='full',powershellArg]=process.argv.slice(2);
const smoke=mode==='smoke';
assert(binaryArg&&outputArg&&gitArg&&['full','smoke'].includes(mode),'Usage: node tests/evaluation-smoke.js HOST NEW_OUTPUT_DIR GIT_EXE [full|smoke] [POWERSHELL_EXE]');
const powershell=powershellArg?resolve(powershellArg):'C:/Windows/System32/WindowsPowerShell/v1.0/powershell.exe';
const binary=resolve(binaryArg),output=resolve(outputArg),git=resolve(gitArg),root=join(output,'repo');
await mkdir(output);await mkdir(root);
const oldPath=process.env.PATH,oldProfile=process.env.WEBVIEW2_USER_DATA_FOLDER;
process.env.PATH=dirname(git)+';'+oldPath;process.env.WEBVIEW2_USER_DATA_FOLDER=join(output,'profile');
const content='Omniterm combined workflow é界\n<img src=x onerror="throw 1">\nTerminal, text and Git stay independent.\n';
function gitRun(...args){const r=spawnSync(git,['-c','user.name=Omniterm Test','-c','user.email=omniterm-test@example.invalid',...args],{cwd:root,encoding:'utf8',windowsHide:true});assert.equal(r.status,0,r.stderr);return r.stdout;}
gitRun('init');await writeFile(join(root,'tracked.txt'),'baseline\n');await writeFile(join(root,'report.txt'),content);
await writeFile(join(root,'flood.ps1'),"$w=[Diagnostics.Stopwatch]::StartNew(); for($i=0;$i -lt 200000 -and $w.Elapsed.TotalSeconds -lt 12;$i++){[Console]::WriteLine(('FLOOD_'+$i+' '+('x'*80)))}");
gitRun('add','tracked.txt','report.txt','flood.ps1');gitRun('commit','-m','Evaluation baseline');
await writeFile(join(root,'tracked.txt'),'modified\n');await writeFile(join(root,'staged.txt'),'staged\n');gitRun('add','staged.txt');await writeFile(join(root,'new.txt'),'new\n');
const evidence={started:new Date().toISOString(),platform:process.platform,node:process.version,
 binary_sha256:createHash('sha256').update(await readFile(binary)).digest('hex'),helper_sha256:createHash('sha256').update(await readFile(join(dirname(binary),'omni.exe'))).digest('hex'),measurement_mode:smoke?'harness smoke':'full',checks:[],measurements:{},human:'NOT RUN',gpu_power_network_io:'NOT MEASURED'};
const check=(name,detail={})=>evidence.checks.push({name,result:'PASS',...detail});
const pause=ms=>new Promise(r=>setTimeout(r,ms));let app;
async function state(){return inspectReady(app);}
async function until(test,timeout=15000){let s;const deadline=Date.now()+timeout;do{s=await state();if(!s.rendered.busy&&test(s))return s;await pause(20);}while(Date.now()<deadline);await writeFile(join(output,'failure-state.json'),JSON.stringify(s,null,2));throw Error('Combined workflow state timeout');}
async function click(r){assert(r&&!r.disabled,JSON.stringify(r));const reply=await app.request({click:{x:Math.floor(r.x+Math.min(r.width/2,40)),y:Math.floor(r.y+Math.min(r.height/2,16))}});assert(!reply.error,JSON.stringify(reply));}
async function key(key){const r=await app.request({key:{key}});assert(!r.error,JSON.stringify(r));}
const terminal=s=>s.rendered.terminals[0];
const screenText=s=>s.screens[0]?.lines.map(line=>line.map(run=>run.text).join('')).join('\n')??'';
async function command(text,needle){
 const s=await until(s=>{const r=terminal(s)?.input,v=s.rendered.viewport;return r&&r.x>=v.x&&r.y>=v.y&&r.x<v.x+v.width&&r.y<v.y+v.height;});
 await click(terminal(s).input);const focused=await state();
 const terminalId=focused.layout.blocks.find(b=>b.session_id===terminal(focused).session_id)?.id;
 assert.equal(focused.rendered.active.block_id,terminalId,'Terminal click did not focus its Block');
 assert.equal(focused.rendered.active.test_id,null,'Terminal click focused a workspace action');
 assert(Buffer.byteLength(text)<=256);const inserted=await app.request({text:{text}});assert(!inserted.error,JSON.stringify(inserted));
 const began=performance.now();await key('enter');const after=needle?await until(s=>terminal(s)?.text.includes(needle)):await state();
 const elapsed=performance.now()-began;
 (evidence.command_observations??=[]).push({index:evidence.command_observations?.length??0,git_tasks:after.git_tasks.map(t=>t.task_id),focused_block:focused.rendered.active.block_id});
 assert(after.git_tasks.length<=1,'Unexpected extra Git Task after terminal command');return elapsed;
}
async function layoutKey(id,which){let s=await state();const before=s.layout.revision;await click(s.rendered.blocks.find(b=>b.id===id).header);await key(which);return until(s=>s.layout.revision!==before);}
async function shot(name){const r=await app.request('screenshot');assert(!r.error);await writeFile(join(output,name+'.png'),Buffer.from(r.result.data,'base64'));await writeFile(join(output,name+'.json'),JSON.stringify(await state(),null,2));}
function sampleResources(name,seconds=60){
 const child=spawn(powershell,['-NoProfile','-ExecutionPolicy','Bypass','-File',resolve('tools/desktop-test-host/tests/resource-probe.ps1'),'-RootProcessId',String(app.child.pid),'-Seconds',String(seconds)],{stdio:['ignore','pipe','pipe'],windowsHide:true});
 let stdout='',stderr='';child.stdout.on('data',b=>stdout+=b);child.stderr.on('data',b=>stderr+=b);
 const closed=new Promise((done,reject)=>{child.once('error',reject);child.once('close',async code=>{try{await writeFile(join(output,name+'.json'),stdout);assert.equal(code,0,stderr);const result=JSON.parse(stdout);assert(result.samples.length===seconds+1);assert(result.samples.some(s=>s.processes.some(p=>p.name==='msedgewebview2')),'WebView children must be counted');validateCounters(result);done(result);}catch(e){reject(e);}});});
 return {closed};
}
function validateCounters(result){
 resourceMetrics(result,0);
}
async function idle(name){
 const seconds=smoke?1:60,before=idleSignature(await state());
 console.log('Measuring '+name+' for '+seconds+' seconds');const result=await sampleResources(name,seconds).closed;
 const after=idleSignature(await state());(evidence.idle_observations??=[]).push({name,before,after});
 assert.deepEqual(after,before,'Idle benchmark workload changed; reject this disturbed run');return result;
}
async function framesBegin(){assert.deepEqual((await app.request({begin_frames:{}})).result,{started:true,max_intervals:512,max_ms:10000});}
async function framesEnd(name){const r=await app.request({end_frames:{}});assert(!r.error);assert(r.result.intervals_ms.length>5&&r.result.intervals_ms.length<=512);await writeFile(join(output,name+'.json'),JSON.stringify(r.result,null,2));return r.result;}
try {
 const began=performance.now();app=await launchAutomation(binary,{cwd:root});let s=await state();evidence.measurements.same_factory_rendered_startup_ms=performance.now()-began;
 assert.equal(s.sessions.length,0);assert.equal(s.git_tasks.length,0);await idle('empty-idle');check('fresh_start_has_no_automatic_work');
 await app.request({resize:{width:1280,height:960}});await until(s=>s.window.width===1280&&s.window.height===960);
 for(const id of ['1','2']){s=await state();await click(s.rendered.blocks.find(b=>b.id===id).header);await click((await state()).rendered.controls.remove);await until(s=>!s.layout.blocks.some(b=>b.id===id));}
 await click((await state()).rendered.controls.new_terminal);s=await until(s=>terminal(s)?.state==='running'&&terminal(s).cols>50);
 const session=s.sessions[0].session_id,terminalId=s.layout.blocks.find(b=>b.session_id===session).id,initialRows=terminal(s).rows;
 await idle('terminal-idle');
 for(let i=0;i<6;i++)await layoutKey(terminalId,'resize_down');await until(s=>terminal(s).rows>initialRows);
 await command('omni open report.txt');s=await until(s=>s.rendered.text_views[0]?.content===content);const textId=s.layout.blocks.find(b=>b.text).id;
 for(let i=0;i<4;i++)await layoutKey(textId,'resize_left');await layoutKey(textId,'resize_up');
 await click((await state()).rendered.controls.reopen);await until(s=>s.rendered.camera.origin.x===0&&s.rendered.camera.origin.y===0);
 const helperBegin=performance.now();await command('omni git-status');s=await until(s=>s.rendered.git_views[0]?.state==='succeeded');evidence.measurements.git_helper_to_observed_ms=performance.now()-helperBegin;
 const gitId=s.layout.blocks.find(b=>b.git_task_id==='1').id;await key('zoom_out');await until(s=>s.rendered.camera.zoom===67);
 for(let i=0;i<4;i++)await layoutKey(gitId,'resize_left');await layoutKey(gitId,'resize_up');
 for(let i=0;i<7;i++)await layoutKey(gitId,'arrow_down');for(let i=0;i<8;i++)await layoutKey(gitId,'arrow_left');
 s=await state();assert.equal(s.layout.blocks.length,3);assert.match(s.rendered.git_views[0].counts,/Staged\s*1\s*Unstaged\s*1\s*Untracked\s*1/);
 const viewport=s.rendered.viewport;assert(s.rendered.blocks.every(b=>b.viewport.x>=viewport.x&&b.viewport.y>=viewport.y&&b.viewport.x+b.viewport.width<=viewport.x+viewport.width+1&&b.viewport.y+b.viewport.height<=viewport.y+viewport.height+1));
 assert(s.rendered.blocks.every(b=>b.injected_elements===0));check('real_terminal_text_git_placed_together_without_overlap_or_injection');await shot('three-view');
 await idle('three-view-idle');
 evidence.measurements.input_request_to_observed_ms=[];
 for(let i=0;i<20;i++){const n=String(i).padStart(3,'0');evidence.measurements.input_request_to_observed_ms.push(await command(`Write-Output ('LAT_'+'${n}')`,'LAT_'+n));}
 check('twenty_real_input_output_round_trips_with_three_views');
 const accepted=(await state()).layout,sizing={rows:terminal(await state()).rows,cols:terminal(await state()).cols};
 await framesBegin();for(let i=0;i<12;i++){await key(i%2===0?'zoom_in':'zoom_out');await state();await pause(40);}
 await framesEnd('interaction-frames');s=await state();assert.deepEqual(s.layout,accepted);assert.equal(terminal(s).rows,sizing.rows);assert.equal(terminal(s).cols,sizing.cols);check('camera_interaction_keeps_geometry_task_and_pty_dimensions');
 const activeProbe=sampleResources('bounded-output',smoke?1:20);await framesBegin();evidence.measurements.interrupt_request_to_prompt_ms=[];
 for(let i=0;i<5;i++){
  await command('.\\flood.ps1');await until(s=>terminal(s)?.text.includes('FLOOD_'));await pause(300);
  const began=performance.now();await key('interrupt');s=await until(s=>terminal(s)?.text.trimEnd().endsWith('>'),12000);evidence.measurements.interrupt_request_to_prompt_ms.push(performance.now()-began);
  await pause(150);await command(`Write-Output ('AFTER_'+'${i}')`,'AFTER_'+i);
 }
 await framesEnd('output-frames');await activeProbe.closed;s=await state();
 assert(s.screens[0].evicted_bytes>0);check('bounded_flood_visible_eviction_and_five_interrupt_recovery_runs',{evicted_bytes:s.screens[0].evicted_bytes});
 await command("Start-Sleep -Milliseconds 500; Write-Output ('HIDDEN_'+'DONE')");s=await state();await click(s.rendered.blocks.find(b=>b.id===terminalId).header);await click((await state()).rendered.controls.remove);
 await until(s=>!s.layout.blocks.some(b=>b.session_id===session)&&screenText(s).includes('HIDDEN_DONE'));await click((await state()).rendered.controls.reopen);s=await until(s=>terminal(s)?.text.includes('HIDDEN_DONE'));
 assert.equal(s.sessions.length,1);assert.equal(s.sessions[0].session_id,session);assert.equal(s.git_tasks.length,1);assert.equal(s.rendered.text_views[0].content,content);check('combined_view_removal_preserves_session_output_text_and_git_without_rerun');
 await app.request('request_close');s=await until(s=>s.rendered.quit_dialog);await click(s.rendered.controls.cancel_quit);await until(s=>!s.rendered.quit_dialog&&s.sessions[0].state==='running');
 await app.request('close');assert.equal((await app.closed).code,0);app=null;check('combined_demo_quit_cancel_and_owned_shutdown');
 evidence.result='PASS';console.log('PASS: '+evidence.checks.length+' combined workflow checks');
} catch(error){evidence.result='FAIL';evidence.failure=String(error);if(app){try{await shot('failure');}catch(snapshotError){evidence.failure_snapshot_error=String(snapshotError);}}throw error;}
finally{process.env.PATH=oldPath;if(oldProfile===undefined)delete process.env.WEBVIEW2_USER_DATA_FOLDER;else process.env.WEBVIEW2_USER_DATA_FOLDER=oldProfile;if(app&&app.child.exitCode===null)await app.stop();evidence.finished=new Date().toISOString();await writeFile(join(output,'result.json'),JSON.stringify(evidence,null,2));}
