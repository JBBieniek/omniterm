import assert from 'node:assert/strict';
import {spawn,spawnSync} from 'node:child_process';
import {createHash} from 'node:crypto';
import {connect} from 'node:net';
import {mkdir,readFile,writeFile,stat} from 'node:fs/promises';
import {resolve,dirname,join} from 'node:path';
import {inspectReady,launchAutomation} from '../client.js';

const [binaryArg,outputArg,gitArg]=process.argv.slice(2);
assert(binaryArg&&outputArg&&gitArg,'Usage: node tests/git-smoke.js HOST NEW_OUTPUT_DIR GIT_EXE');
const binary=resolve(binaryArg),output=resolve(outputArg),git=resolve(gitArg),root=join(output,'root'),repo=join(root,'sub/repo');
await mkdir(repo,{recursive:true});
const originalPath=process.env.PATH;
process.env.PATH=dirname(git)+';'+originalPath;
const evidence={started:new Date().toISOString(),platform:process.platform,node:process.version,
 binary_sha256:createHash('sha256').update(await readFile(binary)).digest('hex'),
 helper_sha256:createHash('sha256').update(await readFile(join(dirname(binary),'omni.exe'))).digest('hex'),
 git_version:spawnSync(git,['--version'],{encoding:'utf8',windowsHide:true}).stdout.trim(),checks:[]};
const check=(name,detail={})=>evidence.checks.push({name,result:'PASS',...detail});
const pause=ms=>new Promise(r=>setTimeout(r,ms));let app;
function gitRun(...args){const r=spawnSync(git,['-c','user.name=Omniterm Test','-c','user.email=omniterm-test@example.invalid',...args],{cwd:repo,encoding:'utf8',windowsHide:true});assert.equal(r.status,0,r.stderr);return r.stdout;}
gitRun('init');await writeFile(join(repo,'tracked.txt'),'baseline\n');gitRun('add','tracked.txt');gitRun('commit','-m','Test baseline');
await writeFile(join(repo,'tracked.txt'),'modified\n');await writeFile(join(repo,'staged.txt'),'staged\n');gitRun('add','staged.txt');await writeFile(join(repo,'new.txt'),'new\n');
const hookMarker=join(output,'HOOK_MUST_NOT_RUN'),hook=join(output,'monitor.cmd');
await writeFile(hook,`@echo off\r\necho unsafe>"${hookMarker}"\r\n`);gitRun('config','core.fsmonitor',hook);
const indexBefore=createHash('sha256').update(await readFile(join(repo,'.git/index'))).digest('hex');
async function state(){return inspectReady(app);}
async function until(test,timeout=18000){let s;const deadline=Date.now()+timeout;do{s=await state();if(!s.rendered.busy&&test(s))return s;await pause(50);}while(Date.now()<deadline);await writeFile(join(output,'failure-state.json'),JSON.stringify(s,null,2));throw Error('Git native state timeout');}
async function click(r){assert(r&&!r.disabled,JSON.stringify(r));const reply=await app.request({click:{x:Math.floor(r.x+Math.min(r.width/2,40)),y:Math.floor(r.y+Math.min(r.height/2,16))}});assert(!reply.error,JSON.stringify(reply));}
const terminal=s=>s.rendered.terminals.findLast(t=>t.state==='running');
const view=(s,id)=>s.rendered.git_views.find(v=>v.task_id===String(id));
async function command(text,needle){const s=await until(s=>{const r=terminal(s)?.input,v=s.rendered.viewport;return r&&r.x>=v.x&&r.y>=v.y&&r.x<v.x+v.width&&r.y<v.y+v.height;});await click(terminal(s).input);assert(Buffer.byteLength(text)<=256);await app.request({text:{text}});await app.request({key:{key:'enter'}});if(needle)await until(s=>terminal(s)?.text.includes(needle));}
async function shell(){let s=await state();await click(s.rendered.controls.new_terminal);await until(s=>terminal(s)?.cols>50);}
async function show(id){let s=await until(s=>s.rendered.git_reopen.some(r=>r.task_id===String(id)));await click(s.rendered.git_reopen.find(r=>r.task_id===String(id)));return until(s=>view(s,id)&&view(s,id).state!=='loading');}
async function run(id){await command('omni git-status');return until(s=>s.git_tasks.find(t=>t.task_id===String(id))&&!['running','draining','loading'].includes(view(s,id)?.state??'loading'));}
async function revealRaw(id){let s=await show(id);await click(view(s,id).toggle_raw);return until(s=>view(s,id)?.raw!==null);}
async function shot(name){const r=await app.request('screenshot');assert(!r.error);await writeFile(join(output,name+'.png'),Buffer.from(r.result.data,'base64'));await writeFile(join(output,name+'.json'),JSON.stringify(await state(),null,2));}
async function close(){await app.request('close');assert.equal((await app.closed).code,0);app=null;}
async function watchGitProcesses() {
 const ids=await Promise.all(['parent.pid','child.pid'].map(async file=>{const id=(await readFile(join(root,file),'utf8')).trim();assert(/^\d+$/.test(id));return id;}));
 const watcher=spawn('C:/Windows/System32/WindowsPowerShell/v1.0/powershell.exe',['-NoProfile','-Command',
  `$a=[Diagnostics.Process]::GetProcessById(${ids[0]});$b=[Diagnostics.Process]::GetProcessById(${ids[1]});$null=$a.Handle;$null=$b.Handle;'ARMED';$ae=$a.WaitForExit(6000);$be=$b.WaitForExit(6000);"git=$ae child=$be";if (!$ae -or !$be){exit 1}`],{stdio:['ignore','pipe','pipe'],windowsHide:true});
 let stdout='',stderr='';const closed=new Promise((done,reject)=>{watcher.once('error',reject);watcher.once('close',code=>done({code,stdout,stderr}));});
 watcher.stdout.on('data',b=>stdout+=b);watcher.stderr.on('data',b=>stderr+=b);
 const deadline=Date.now()+5000;while(!stdout.includes('ARMED')){assert(Date.now()<deadline,stderr);await pause(25);}return {closed};
}
function outsider(endpoint,body){return new Promise((done,reject)=>{const socket=connect(endpoint);let data='';const timer=setTimeout(()=>{socket.destroy();reject(Error('Outsider timeout'));},5000);socket.on('connect',()=>socket.write(JSON.stringify(body)));socket.on('error',reject);socket.on('data',b=>{data+=b;try{const reply=JSON.parse(data);clearTimeout(timer);socket.write(Buffer.from([1]));socket.end();done(reply);}catch{}});});}
try {
 app=await launchAutomation(binary,{cwd:root});await shell();
 await command("Set-Location sub/repo; Write-Output ('CWD_'+'READY')",'CWD_READY');
 let s=await state();const camera=s.rendered.camera,active=s.rendered.active.block_id,begin=Date.now();
 await run(1);s=await state();assert.deepEqual(s.rendered.camera,camera);assert.equal(s.rendered.active.block_id,active);
 s=await revealRaw(1);assert.match(view(s,1).counts,/Staged\s*1\s*Unstaged\s*1\s*Untracked\s*1/);
 assert.match(view(s,1).raw,/ M tracked.txt/);assert.match(view(s,1).raw,/A  staged.txt/);assert.match(view(s,1).raw,/\?\? new.txt/);
 assert.match(view(s,1).outcome,/succeeded.*exit 0.*stdout EOF yes.*stderr EOF yes/s);
 assert.match(view(s,1).source,/sub\\repo/);assert.equal(view(s,1).injected_elements,0);
 assert.equal(createHash('sha256').update(await readFile(join(repo,'.git/index'))).digest('hex'),indexBefore);
 assert.equal(await stat(hookMarker).then(()=>true,()=>false),false);
 check('real_git_actual_cwd_1_1_1_raw_eofs_no_hook_no_index_write_no_focus_theft',{request_to_observed_ms:Date.now()-begin,metric:'one end-to-end controller sample, not budget'});
 await shot('git-1-1-1');
 const block=s.layout.blocks.find(b=>b.git_task_id==='1');await click(s.rendered.blocks.find(b=>b.id===block.id).header);
 await click((await state()).rendered.controls.remove);s=await until(s=>!s.layout.blocks.some(b=>b.git_task_id==='1'));
 assert(s.git_tasks.some(t=>t.task_id==='1'));s=await revealRaw(1);assert.match(view(s,1).raw,/tracked.txt/);check('hide_and_reopen_preserves_task_and_raw_without_rerun');
 await writeFile(join(repo,'staged.txt'),'changed after staging\n');await click(view(s,1).refresh);
 s=await until(s=>view(s,2)?.state==='succeeded');assert.match(view(s,2).counts,/Staged\s*1\s*Unstaged\s*2\s*Untracked\s*1/);assert.match(view(s,1).counts,/Unstaged\s*1/);
 check('explicit_refresh_new_task_overlap_counts_old_snapshot_retained');
 gitRun('add','tracked.txt','staged.txt','new.txt');gitRun('commit','-m','Clean fixture');
 await click(view(s,2).refresh);s=await until(s=>view(s,3)?.state==='succeeded');assert.match(view(s,3).counts,/Staged\s*0\s*Unstaged\s*0\s*Untracked\s*0/);check('empty_stdout_is_zero_only_after_success_and_both_eofs');
 await click(s.rendered.controls.reopen);await command("Set-Location ../..; Write-Output ('ROOT_'+'READY')",'ROOT_READY');
 await run(4);s=await revealRaw(4);assert.equal(view(s,4).counts,null);assert.match(view(s,4).raw,/not a git repository/);assert.match(view(s,4).outcome,/failed.*exit 128/);check('nonrepository_keeps_stderr_exit_and_unavailable_counts');
 await click(s.rendered.controls.reopen);await command("$env:OMNITERM_OPEN_PIPE | Set-Content route.txt; Write-Output ('ROUTE_'+'READY')",'ROUTE_READY');
 const endpoint=(await readFile(join(root,'route.txt'),'utf8')).trim();s=await state();const layout=s.layout;
 const reply=await outsider(endpoint,{api_version:1,op:'git_status',cwd:repo});assert.equal(reply.status,'error');assert.match(reply.error,/outside the authorized Session/);assert.deepEqual((await state()).layout,layout);
 await command("omni git-status extra; Write-Output ('ARGS_'+'EXIT='+$LASTEXITCODE)",'ARGS_EXIT=2');assert.deepEqual((await state()).layout,layout);check('outside_process_and_extra_argv_cannot_launch_git');
 await close();

 // Adversarial process behavior uses an explicitly separate test executable, never real Git claims.
 const fake=join(output,'fake');await mkdir(fake);const build=spawnSync('rustc',[resolve('tools/desktop-test-host/tests/fixtures/git.rs'),'-o',join(fake,'git.exe')],{encoding:'utf8',windowsHide:true});assert.equal(build.status,0,build.stderr);
 process.env.PATH=fake+';'+originalPath;await writeFile(join(root,'mode'),'split');app=await launchAutomation(binary,{cwd:root});await shell();
 await run(1);s=await revealRaw(1);assert.match(view(s,1).counts,/Staged\s*1\s*Unstaged\s*1\s*Untracked\s*1/);assert.match(view(s,1).raw,/é界.txt/);assert.match(view(s,1).raw,/fixture stderr stays separate/);check('fixture_arbitrary_byte_splits_and_separate_streams');
 for(const [id,mode,pattern] of [[2,'malformed',/UU conflict/],[3,'invalid',/bad/],[4,'nonzero',/explicit failure/],[5,'flood',/stdout/]]) {
   await writeFile(join(root,'mode'),mode);await click(s.rendered.controls.reopen);await run(id);s=await revealRaw(id);assert.equal(view(s,id).counts,null);assert.match(view(s,id).raw,pattern);
   if(mode==='flood')assert.match(view(s,id).gap,/Raw gap: [1-9]/);
   if(mode==='nonzero')assert.match(view(s,id).outcome,/failed.*exit 7/);
   check('fixture_'+mode+'_retains_raw_and_never_reports_clean');
 }
 await writeFile(join(root,'mode'),'slow');await click(s.rendered.controls.reopen);await command('omni git-status');
 s=await until(s=>s.git_tasks.some(t=>t.task_id==='6'&&t.state==='running'));await stat(join(root,'child.pid')).catch(()=>pause(200));
 const childPid=Number(await readFile(join(root,'child.pid'),'utf8'));s=await show(6);
 const activeBlock=s.layout.blocks.find(b=>b.git_task_id==='6');await click(s.rendered.blocks.find(b=>b.id===activeBlock.id).header);await click((await state()).rendered.controls.remove);
 s=await until(s=>!view(s,6));assert.equal(s.git_tasks.find(t=>t.task_id==='6').state,'running');s=await show(6);await click(view(s,6).cancel);
 s=await until(s=>view(s,6)?.state==='cancelled');assert.equal(view(s,6).counts,null);await pause(150);assert.throws(()=>process.kill(childPid,0));check('live_hide_reopen_cancel_confirms_exit_and_owned_child_cleanup');
 await click(s.rendered.controls.reopen);await command('omni git-status');s=await until(s=>view(s,7)?.state==='running');s=await show(7);
 const deadlineBegan=Date.now();s=await until(s=>view(s,7)?.state==='timed_out',35000);assert.equal(view(s,7).counts,null);check('thirty_second_deadline_confirms_exit_without_false_counts',{observed_wait_ms:Date.now()-deadlineBegan});
 await shot('git-fallback');
 await writeFile(join(root,'mode'),'split');await click(s.rendered.controls.reopen);await run(8);s=await state();const capped=s.layout;
 await command("omni git-status; Write-Output ('CAP_'+'EXIT='+$LASTEXITCODE)",'CAP_EXIT=2');assert.deepEqual((await state()).layout,capped);assert.equal((await state()).git_tasks.length,8);
 check('eight_run_retention_cap_rejects_without_new_task_or_layout_change');
 await close();

 for(const mode of ['quit','crash']) {
   await writeFile(join(root,'mode'),'slow');app=await launchAutomation(binary,{cwd:root});await shell();await command('omni git-status');
   await until(s=>s.git_tasks.some(t=>t.state==='running'));await pause(150);const watcher=await watchGitProcesses();
   if(mode==='quit') {
     await app.request('request_close');s=await until(s=>s.rendered.quit_dialog);await click(s.rendered.controls.cancel_quit);
     s=await until(s=>!s.rendered.quit_dialog);assert.equal(s.git_tasks[0].state,'running');
     await app.request('request_close');s=await until(s=>s.rendered.quit_dialog);await click(s.rendered.controls.confirm_quit);assert.equal((await app.closed).code,0);app=null;
   } else {await app.stop();app=null;}
   const result=await watcher.closed;await writeFile(join(output,mode+'-git-process-handles.json'),JSON.stringify(result,null,2));assert.equal(result.code,0,result.stdout+result.stderr);
   check('live_git_'+mode+'_owns_and_reaps_process_and_descendant_handles');
 }

 process.env.PATH=process.env.SystemRoot+'\\System32';app=await launchAutomation(binary,{cwd:root});await shell();await run(1);s=await revealRaw(1);assert.equal(view(s,1).counts,null);assert.match(view(s,1).outcome,/failed.*exit unavailable/);assert.match(view(s,1).interpretation,/Unavailable/);check('missing_git_is_visible_failure_without_false_clean');await close();
 evidence.result='PASS';console.log(`PASS: ${evidence.checks.length} native Git checks`);
} catch(error) { evidence.result='FAIL';evidence.failure=String(error);throw error; }
finally { process.env.PATH=originalPath;if(app&&app.child.exitCode===null)await app.stop();evidence.finished=new Date().toISOString();await writeFile(join(output,'result.json'),JSON.stringify(evidence,null,2)); }
