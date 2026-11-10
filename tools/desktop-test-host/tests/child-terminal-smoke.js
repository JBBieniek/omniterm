import assert from 'node:assert/strict';
import {spawn} from 'node:child_process';
import {createHash} from 'node:crypto';
import {mkdir,readFile,writeFile} from 'node:fs/promises';
import {connect} from 'node:net';
import {dirname,join,resolve} from 'node:path';
import {inspectReady,launchAutomation} from '../client.js';
import {resourceMetrics} from '../benchmark-report.js';

const [binaryArg,outputArg,powershellArg]=process.argv.slice(2);
assert(binaryArg&&outputArg&&powershellArg,'Usage: child-terminal-smoke.js HOST NEW_DIR POWERSHELL');
const binary=resolve(binaryArg),output=resolve(outputArg),folder='child é space';
await mkdir(output);await mkdir(join(output,folder));
await writeFile(join(output,folder,'note.txt'),'Child helper result <img src=x> é界');
const previousProfile=process.env.WEBVIEW2_USER_DATA_FOLDER;process.env.WEBVIEW2_USER_DATA_FOLDER=join(output,'profile');
const hash=async path=>createHash('sha256').update(await readFile(path)).digest('hex');
const evidence={started:new Date().toISOString(),platform:process.platform,node:process.version,
  binary_sha256:await hash(binary),helper_sha256:await hash(join(dirname(binary),'omni.exe')),
  checks:[],measurements:{child_launch_ms:[],child_input_ms:[]},limits:'Fixed PowerShell child, no Python API/grammar; mixed sampled resources, not idle/scaling/power or a significance claim'};
let app,probe,expected;
const pause=ms=>new Promise(done=>setTimeout(done,ms));
const terminal=(s,id)=>s.rendered.terminals.find(t=>t.session_id===id);
const text=(s,id)=>s.screens.find(t=>t.session_id===id)?.lines.map(l=>l.map(r=>r.text).join('')).join('\n')??'';
const state=()=>inspectReady(app);
function counts(s){assert.equal(s.rendered.input_events.clicks,expected.clicks,'Unrequested click disturbed workload');assert.equal(s.rendered.input_events.key_downs,expected.keys,'Unrequested key disturbed workload');}
async function until(test,timeout=15000){let s;const end=Date.now()+timeout;do{s=await state();counts(s);if(!s.rendered.busy&&test(s))return s;await pause(25);}while(Date.now()<end);await writeFile(join(output,'timeout.json'),JSON.stringify(s,null,2));throw Error('Child terminal state timeout');}
async function action(command,clicks=0,keys=0,closing=false){counts(await state());const reply=await app.request(command);assert(!reply.error,JSON.stringify(reply));expected.clicks+=clicks;expected.keys+=keys;if(!closing)return until(s=>s.rendered.input_events.clicks===expected.clicks&&s.rendered.input_events.key_downs===expected.keys);}
const click=(r,closing=false)=>{assert(r&&!r.disabled,'Control unavailable');return action({click:{x:Math.floor(r.x+Math.min(r.width/2,35)),y:Math.floor(r.y+Math.min(r.height/2,12))}},1,0,closing);};
const key=name=>action({key:{key:name}},0,1);
async function show(id){const s=await state();if(s.rendered.selected_id!==s.layout.blocks.find(b=>b.session_id===id)?.id){await click(s.rendered.session_views.find(v=>v.session_id===id).control);await until(s=>s.rendered.selected_id===s.layout.blocks.find(b=>b.session_id===id)?.id);}}
async function command(id,code,needle){await show(id);await click(terminal(await state(),id).input);await action({text:{text:code}});const start=performance.now();await key('enter');if(needle)await until(s=>terminal(s,id)?.text.includes(needle));return performance.now()-start;}
async function shot(name){const reply=await app.request('screenshot');assert(!reply.error);await writeFile(join(output,name+'.png'),Buffer.from(reply.result.data,'base64'));await writeFile(join(output,name+'.json'),JSON.stringify(await state(),null,2));}
function check(name){evidence.checks.push({name,result:'PASS'});}
async function launch(){expected={clicks:0,keys:0};app=await launchAutomation(binary,{cwd:output});await state();await action({resize:{width:1280,height:960}});await click((await state()).rendered.controls.new_terminal);await until(s=>terminal(s,'1')?.state==='running'&&terminal(s,'1').cols>50);}
async function child(parent,id){await show(parent);await click(terminal(await state(),parent).input);const s=await state(),camera=s.rendered.camera,focus=s.rendered.active.block_id;
 const start=performance.now();await command(parent,'omni terminal');await until(s=>s.sessions.some(t=>t.session_id===id)&&terminal(s,id)?.parent_session_id===parent&&terminal(s,id).state==='running'&&s.rendered.controls.show_terminal);
 evidence.measurements.child_launch_ms.push(performance.now()-start);
 const after=await state();assert.deepEqual(after.rendered.camera,camera);assert.equal(after.rendered.active.block_id,focus);assert.equal(after.sessions.find(t=>t.session_id===id).parent_session_id,parent);
}
async function denied(id,code,label){const s=await state(),layout=s.layout,sessions=s.sessions.map(t=>t.session_id);await command(id,`${code}; 'DENY_'+'${label}='+$LASTEXITCODE`,'DENY_'+label+'=2');const after=await state();assert.deepEqual(after.layout,layout);assert.deepEqual(after.sessions.map(t=>t.session_id),sessions);}
async function remove(id){const s=await state(),block=s.layout.blocks.find(b=>b.session_id===id);await click(s.rendered.blocks.find(b=>b.id===block.id).header);await click((await state()).rendered.controls.remove);await until(s=>!s.layout.blocks.some(b=>b.id===block.id));}
function resources(){const child=spawn(resolve(powershellArg),['-NoProfile','-ExecutionPolicy','Bypass','-File',resolve('tools/desktop-test-host/tests/resource-probe.ps1'),'-RootProcessId',String(app.child.pid),'-Seconds','15'],{windowsHide:true,stdio:['ignore','pipe','pipe']});let stdout='',stderr='';child.stdout.on('data',b=>stdout+=b);child.stderr.on('data',b=>stderr+=b);const closed=new Promise((done,reject)=>{child.once('error',reject);child.once('close',async code=>{try{await writeFile(join(output,'resources.json'),stdout);assert.equal(code,0,stderr);const raw=JSON.parse(stdout);assert.equal(raw.samples.length,16);resourceMetrics(raw,15);done();}catch(e){reject(e);}});});closed.catch(()=>{});return {closed};}
async function outsider(endpoint){return new Promise((done,reject)=>{const socket=connect(endpoint);let data='';const timer=setTimeout(()=>{socket.destroy();reject(Error('Outside caller timeout'));},5000);socket.once('error',e=>{clearTimeout(timer);reject(e);});socket.once('connect',()=>socket.write(JSON.stringify({api_version:1,op:'terminal',cwd:join(output,folder)})));socket.on('data',bytes=>{data+=bytes;try{const r=JSON.parse(data);socket.write(Buffer.from([1]));clearTimeout(timer);socket.end();done(r);}catch{}});});}
async function watch(ids){const child=spawn(resolve(powershellArg),['-NoProfile','-Command',`$p=@(${ids.map(id=>`[Diagnostics.Process]::GetProcessById(${id})`).join(';')}); 'ARMED'; foreach ($v in $p) {if (!$v.WaitForExit(6000)) {exit 1}}; 'REAPED'`],{windowsHide:true,stdio:['ignore','pipe','pipe']});let stdout='',stderr='';const closed=new Promise((done,reject)=>{child.once('error',reject);child.once('close',code=>done({code,stdout,stderr}));});child.stdout.on('data',b=>stdout+=b);child.stderr.on('data',b=>stderr+=b);const end=Date.now()+5000;while(!stdout.includes('ARMED')){assert(Date.now()<end,stderr);await pause(25);}return {closed};}
async function pid(id,name){await command(id,`$PID | Set-Content ${name}; 'PID_'+'READY_${id}'`,'PID_READY_'+id);const p=(await readFile(join(output,folder,name),'utf8')).trim();assert(/^\d+$/.test(p));return p;}
try{
 await launch();await command('1',`Set-Location '${folder}'; $env:OMNITERM_OPEN_PIPE | Set-Content parent.pipe; 'SOURCE_'+'READY'`,'SOURCE_READY');
 await child('1','2');let s=await state();assert.equal(s.sessions[0].parent_session_id,null);check('fixed_child_from_verified_unicode_cwd_with_parent_link_and_preserved_focus');
 await click(s.rendered.controls.show_terminal);await until(s=>s.rendered.selected_id===s.layout.blocks.find(b=>b.session_id==='2').id);probe=resources();
 await command('2',"(Get-Location).Path | Set-Content cwd.txt -Encoding UTF8; 'CHILD_'+'ONLY'",'CHILD_ONLY');
 assert.equal((await readFile(join(output,folder,'cwd.txt'),'utf8')).trim(),join(output,folder));assert(!text(await state(),'1').includes('CHILD_ONLY'));
 for(let i=0;i<10;i++)evidence.measurements.child_input_ms.push(await command('2',`'CHILD_LAT_'+'${i}'`,'CHILD_LAT_'+i));
 await click(terminal(await state(),'2').parent);await command('1',"'PARENT_'+'ONLY'",'PARENT_ONLY');assert(!text(await state(),'2').includes('PARENT_ONLY'));await click((await state()).rendered.controls.show_terminal);check('separate_output_input_and_ten_child_round_trips');
 s=await state();const parentSize=s.screens.find(t=>t.session_id==='1').rows,childSize=s.screens.find(t=>t.session_id==='2').rows;
 await click(s.rendered.blocks.find(b=>b.id===s.layout.blocks.find(b=>b.session_id==='2').id).header);await key('resize_down');await until(s=>s.screens.find(t=>t.session_id==='2').rows>childSize);assert.equal((await state()).screens.find(t=>t.session_id==='1').rows,parentSize);check('resizing_child_preserves_parent_pty_size');
 await command('2','omni open note.txt');await until(s=>s.layout.blocks.some(b=>b.text?.source_session_id==='2'));assert((await state()).rendered.text_views.some(t=>t.content==='Child helper result <img src=x> é界'&&t.injected_elements===0));
 await denied('2',"$own=$env:OMNITERM_OPEN_PIPE; $env:OMNITERM_OPEN_PIPE=Get-Content parent.pipe; omni terminal",'cross');
 await command('2',"$env:OMNITERM_OPEN_PIPE=$own; Set-Location ..; omni terminal; 'OUTSIDE_'+'CODE='+$LASTEXITCODE",'OUTSIDE_CODE=2');
 await command('2',`Set-Location '${folder}'; 'BACK_'+'READY'`,'BACK_READY');check('own_child_helper_scope_cross_session_and_outside_cwd_denials');
 await remove('2');assert.equal((await state()).sessions.find(t=>t.session_id==='2').state,'running');await click((await state()).rendered.controls.reopen);await until(s=>terminal(s,'2')?.state==='running');
 await click(terminal(await state(),'2').parent);await remove('1');assert.equal((await state()).sessions.find(t=>t.session_id==='1').state,'running');await click(terminal(await state(),'2').parent);await until(s=>terminal(s,'1')?.state==='running');
 assert.equal((await state()).sessions.length,2);check('removing_and_reopening_either_view_preserves_both_session_identities');
 await probe.closed;
 await click(terminal(await state(),'2').parent);await click((await state()).rendered.controls.terminate);await until(s=>s.sessions.find(t=>t.session_id==='1').state==='exited');assert.equal((await state()).sessions.find(t=>t.session_id==='2').state,'running');
 // The child is now independent of its stopped source and can still cooperate.
 await command('2',"'SURVIVED_'+'PARENT'",'SURVIVED_PARENT');await command('2','omni open note.txt');await until(s=>s.layout.blocks.filter(b=>b.text?.source_session_id==='2').length===2);
 await child('2','3');await child('2','4');check('source_termination_preserves_child_and_nested_launch_provenance');
 await denied('2','omni terminal','cap');await denied('2','omni terminal extra','args');
 const childPipe=(await command('2',"$env:OMNITERM_OPEN_PIPE | Set-Content child.pipe; 'PIPE_'+'READY'",'PIPE_READY'),(await readFile(join(output,folder,'child.pipe'),'utf8')).trim());
 assert.match(childPipe,/^\\\\\.\\pipe\\omniterm-open-[a-f0-9]{64}$/);s=await state();const outside=await outsider(childPipe);assert.equal(outside.status,'error');assert.match(outside.error,/outside the authorized Session/);assert.deepEqual((await state()).layout,s.layout);check('four_session_cap_fixed_arguments_and_outside_pid_denial');
 // Wait for parent helper response before navigating so source input is never stolen.
 await click((await state()).rendered.controls.show_terminal);await until(s=>terminal(s,'4')?.parent_session_id==='2');await shot('attached-terminals');
 // All live Session shell handles are held before confirmed native quit.
 const ids=[];for(const id of ['2','3','4']){await command(id,`$PID | Set-Content pid-${id}.txt; 'HANDLE_'+'${id}'`,'HANDLE_'+id);ids.push((await readFile(join(output,folder,`pid-${id}.txt`),'utf8')).trim());}
 assert(ids.every(id=>/^\d+$/.test(id)));const quitting=await watch(ids);await action('request_close');await until(s=>s.rendered.quit_dialog);await click((await state()).rendered.controls.confirm_quit,true);assert.equal((await app.closed).code,0);const reaped=await quitting.closed;await writeFile(join(output,'quit-handles.json'),JSON.stringify(reaped,null,2));assert.equal(reaped.code,0,reaped.stderr);assert(reaped.stdout.includes('REAPED'));check('confirmed_quit_reaps_all_independently_owned_child_shells');
 // Fresh app for abrupt death: capture parent, child and a real descendant.
 await launch();await command('1',`Set-Location '${folder}'; 'CRASH_'+'READY'`,'CRASH_READY');await command('1','omni terminal');await until(s=>s.sessions.length===2&&terminal(s,'2')?.state==='running');
 const crashIds=[await pid('1','crash-parent.pid'),await pid('2','crash-child.pid')];
 await command('2',"$p=Start-Process powershell.exe -ArgumentList '-NoProfile','-Command','Start-Sleep 120' -NoNewWindow -PassThru; $p.Id | Set-Content crash-desc.pid; 'DESC_'+'READY'",'DESC_READY');crashIds.push((await readFile(join(output,folder,'crash-desc.pid'),'utf8')).trim());assert(crashIds.every(id=>/^\d+$/.test(id)));
 const crashing=await watch(crashIds);await app.stop();const dead=await crashing.closed;await writeFile(join(output,'crash-handles.json'),JSON.stringify(dead,null,2));assert.equal(dead.code,0,dead.stderr);assert(dead.stdout.includes('REAPED'));check('host_death_reaps_parent_child_and_childs_descendant');
 // Isolate the full-board case so it cannot consume the launch benchmark's slots.
 await launch();await command('1','omni terminal');await until(s=>s.sessions.length===2&&terminal(s,'2')?.state==='running');
 for(let i=0;i<20;i++){s=await state();const revision=s.layout.revision;await click(s.rendered.controls.add);s=await state();if(s.layout.revision===revision)break;}
 await denied('2','omni terminal','room');assert.equal((await state()).sessions.length,2);check('full_board_denial_creates_no_session_or_layout_change');await app.stop();
 assert.equal(evidence.checks.length,10);evidence.result='PASS';console.log('PASS: ten child terminal groups');
}catch(error){evidence.result='FAIL';evidence.failure=String(error);if(app?.child.exitCode===null){try{await shot('failure');}catch{}}throw error;}
finally{if(probe)await probe.closed.catch(()=>{});if(app?.child.exitCode===null)await app.stop();evidence.finished=new Date().toISOString();await writeFile(join(output,'result.json'),JSON.stringify(evidence,null,2));if(previousProfile===undefined)delete process.env.WEBVIEW2_USER_DATA_FOLDER;else process.env.WEBVIEW2_USER_DATA_FOLDER=previousProfile;}
