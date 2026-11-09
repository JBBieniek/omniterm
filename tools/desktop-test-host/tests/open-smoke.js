import assert from 'node:assert/strict';
import {createHash} from 'node:crypto';
import {connect} from 'node:net';
import {spawn} from 'node:child_process';
import {mkdir,readFile,writeFile,symlink,link} from 'node:fs/promises';
import {resolve,dirname} from 'node:path';
import {inspectReady,launchAutomation} from '../client.js';

const [binaryArg,outputArg]=process.argv.slice(2);
assert(binaryArg&&outputArg,'Usage: node tests/open-smoke.js TEST_HOST NEW_OUTPUT_DIR');
const binary=resolve(binaryArg),output=resolve(outputArg),root=resolve(output,'root');
await mkdir(output);await mkdir(root);await mkdir(resolve(root,'sub'));await mkdir(resolve(output,'outside'));
const content='Current cwd é界é\n<img src=x onerror="throw 1">\n<script>throw 2</script>\n';
await Promise.all([
  writeFile(resolve(root,'report.txt'),'INITIAL_ROOT_FILE'),writeFile(resolve(root,'sub/report.txt'),content),
  writeFile(resolve(root,'sub/bad.txt'),Buffer.from([255,0,3])),writeFile(resolve(root,'sub/large.txt'),'x'.repeat(1048577)),
  writeFile(resolve(output,'outside/secret.txt'),'OUTSIDE_MUST_NOT_OPEN'),writeFile(resolve(root,'sub/limit.txt'),'x'.repeat(1048576))
]);
await symlink(resolve(output,'outside'),resolve(root,'sub/escape'),'junction');
await link(resolve(output,'outside/secret.txt'),resolve(root,'sub/hard.txt'));
await writeFile(resolve(root,'raw.ps1'),String.raw`param([string]$Case)
$p=[IO.Pipes.NamedPipeClientStream]::new('.', $env:OMNITERM_OPEN_PIPE.Substring(9), [IO.Pipes.PipeDirection]::InOut)
try {
 $p.Connect(1500); $p.ReadMode=[IO.Pipes.PipeTransmissionMode]::Message
 if ($Case -eq 'idle') {Start-Sleep -Seconds 3; $s='{}'}
 elseif ($Case -eq 'oversized') {$s='x'*16385}
 elseif ($Case -eq 'claim') {$s='{"api_version":1,"op":"open","cwd":"x","path":"a","session_id":"1"}'}
 else {$s='{"api_version":1,"op":"execute","cwd":"x","path":"a"}'}
 $b=[Text.Encoding]::UTF8.GetBytes($s); $p.Write($b,0,$b.Length)
 $r=New-Object byte[] 1024; $n=$p.Read($r,0,$r.Length); Write-Output ([Text.Encoding]::UTF8.GetString($r,0,$n))
 $p.Write([byte[]]@(1),0,1)
} catch {Write-Output 'RAW_REJECTED'} finally {$p.Dispose()}
Write-Output ('RAW_' + $Case + '_DONE')
`);
const evidence={platform:process.platform,node:process.version,started:new Date().toISOString(),
 binary_sha256:createHash('sha256').update(await readFile(binary)).digest('hex'),
 helper_sha256:createHash('sha256').update(await readFile(resolve(dirname(binary),'omni.exe'))).digest('hex'),checks:[]};
const check=(name,details={})=>evidence.checks.push({name,result:'PASS',...details});
const pause=ms=>new Promise(r=>setTimeout(r,ms));let app;
const texts=s=>s.layout.blocks.filter(b=>b.text);
const terminal=s=>s.rendered.terminals.findLast(t=>t.state==='running');
async function state(){return inspectReady(app);}
async function until(test,timeout=15000){let last;const deadline=Date.now()+timeout;do{last=await state();if(!last.rendered.busy&&test(last))return last;await pause(40);}while(Date.now()<deadline);await writeFile(resolve(output,'failure-state.json'),JSON.stringify(last,null,2));throw new Error('Open test did not reach expected state');}
async function click(r){assert(r&&!r.disabled);const reply=await app.request({click:{x:Math.floor(r.x+Math.min(r.width/2,40)),y:Math.floor(r.y+Math.min(r.height/2,16))}});assert(!reply.error,JSON.stringify(reply));}
async function command(text,needle){const s=await state();await click(terminal(s).input);assert(Buffer.byteLength(text)<=256);assert.deepEqual(await app.request({text:{text}}),{result:{text_sent:true}});assert.deepEqual(await app.request({key:{key:'enter'}}),{result:{key_sent:true}});if(needle)return until(s=>terminal(s)?.text.includes(needle));}
async function shot(name){const s=await state(),r=await app.request('screenshot');assert(!r.error,JSON.stringify(r));await writeFile(resolve(output,name+'.png'),Buffer.from(r.result.data,'base64'));await writeFile(resolve(output,name+'.json'),JSON.stringify(s,null,2));}
async function denied(text,index){const before=(await state()).layout;await command(`${text}; Write-Output ('DENY_'+'${index}='+$LASTEXITCODE)`,`DENY_${index}=2`);assert.deepEqual((await state()).layout,before);}
function outsider(endpoint,body){return new Promise((resolveResult,reject)=>{
 const socket=connect(endpoint);let data='';const timer=setTimeout(()=>{socket.destroy();reject(new Error('Outsider response timeout'));},5000);
 socket.on('connect',()=>socket.write(JSON.stringify(body)));socket.on('error',reject);
 socket.on('data',chunk=>{data+=chunk;try{const reply=JSON.parse(data);socket.write(Buffer.from([1]));clearTimeout(timer);socket.end();resolveResult(reply);}catch{}});
});}
function externalHelper(env){return new Promise((resolveResult,reject)=>{const child=spawn(resolve(dirname(binary),'omni.exe'),['open','report.txt'],{cwd:resolve(root,'sub'),env:{...process.env,...env},windowsHide:true});let stderr='';child.stderr.on('data',b=>stderr+=b);child.once('error',reject);child.once('close',code=>resolveResult({code,stderr}));});}

try{
 app=await launchAutomation(binary,{cwd:root});let s=await state();assert.equal(s.sessions.length,0);
 await click(s.rendered.controls.new_terminal);s=await until(s=>terminal(s)?.state==='running'&&terminal(s).cols>50);
 await command("Set-Location sub; Write-Output ('CWD_'+'READY')",'CWD_READY');
 const camera=s.rendered.camera,begin=Date.now();await command('omni open ./report.txt');
 s=await until(s=>texts(s).length===1&&s.rendered.text_views[0]?.content===content);
 assert.equal(texts(s)[0].text.path,'sub\\report.txt');assert.equal(texts(s)[0].text.source_session_id,'1');
 assert.deepEqual(s.rendered.camera,camera);assert.equal(s.rendered.active.block_id,s.layout.blocks.find(b=>b.session_id==='1').id);
 assert.equal(s.rendered.text_views[0].injected_elements,0);check('real_helper_actual_changed_cwd_unicode_inert_text_and_focus',{request_to_observed_ms:Date.now()-begin,metric:'one controller/helper/IPC/layout/read/render sample, not a performance budget'});
 await click(s.rendered.controls.show_text);await shot('opened-text');s=await state();await click(s.rendered.controls.reopen);
 for(const [i,name] of ['missing.txt','large.txt','bad.txt','../report.txt','escape/secret.txt','hard.txt','report.txt:stream'].entries())await denied(`omni open ${name}`,i);
 check('missing_oversized_invalid_utf8_traversal_junction_hardlink_and_ads_deny_atomically');
 for(const name of ['claim','unknown','oversized','idle']){const before=(await state()).layout;await command(`& ../raw.ps1 ${name}`,`RAW_${name}_DONE`);assert.deepEqual((await state()).layout,before);}
 check('authenticated_forged_unknown_oversized_and_idle_requests_deny_without_mutation');
 await command("$env:OMNITERM_OPEN_PIPE | Set-Content ../endpoint.txt; Write-Output ('ROUTE_'+'READY')",'ROUTE_READY');
 const endpoint=(await readFile(resolve(root,'endpoint.txt'),'utf8')).trim();assert(/^\\\\\.\\pipe\\omniterm-open-[a-f0-9]{64}$/.test(endpoint));
 const body={api_version:1,op:'open',cwd:resolve(root,'sub'),path:'report.txt'};
 const deniedReply=await outsider(endpoint,body);assert.equal(deniedReply.status,'error');assert.match(deniedReply.error,/outside the authorized Session/);
 const outside=await externalHelper({OMNITERM_OPEN_PIPE:endpoint,OMNITERM_HOST_PID:String(app.child.pid)});assert.equal(outside.code,2);assert.match(outside.stderr,/outside the authorized Session/);
 const wrongHost=await externalHelper({OMNITERM_OPEN_PIPE:endpoint,OMNITERM_HOST_PID:String(app.child.pid+1)});assert.equal(wrongHost.code,2);assert.match(wrongHost.stderr,/identity mismatch/);
 check('known_endpoint_and_host_identity_do_not_authorize_outside_process');
 await command('omni open limit.txt');s=await until(s=>texts(s).length===2&&s.rendered.text_views.some(v=>v.length===1048576));
 check('exact_one_mib_boundary_is_complete_without_truncation');
 for(let count=3;count<=8;count++){await command('omni open report.txt');await until(s=>texts(s).length===count);}
 await denied('omni open report.txt','storage');check('eight_document_retention_cap_rejects_without_layout_change');
 // Fill the remaining board through ordinary UI controls, then attempt a helper placement.
 for(let attempt=0;attempt<20;attempt++){s=await state();const revision=s.layout.revision;await click(s.rendered.controls.add);s=await until(s=>s.layout.revision!==revision||!!s.rendered.error);if(s.layout.revision===revision)break;}
 await denied('omni open report.txt','space');check('full_board_denial_is_atomic');
 s=await state();await click(s.rendered.controls.terminate);await until(s=>s.sessions[0].state==='exited');
 const retained=(await state()).rendered.text_views[0].content;assert.equal(retained,content);
 const revoked=await externalHelper({OMNITERM_OPEN_PIPE:endpoint,OMNITERM_HOST_PID:String(app.child.pid)});assert.equal(revoked.code,2);assert.match(revoked.stderr,/unavailable|revoked/);
 check('session_exit_revokes_endpoint_and_text_snapshot_survives');
 assert.deepEqual(await app.request('close'),{result:{closing:true}});assert.equal((await app.closed).code,0);
 // Isolate the second-Session case from native dropdown-popup steering.
 app=await launchAutomation(binary,{cwd:root});s=await state();await click(s.rendered.controls.new_terminal);await until(s=>terminal(s)?.state==='running');
 await command("$env:OMNITERM_OPEN_PIPE | Set-Content endpoint.txt; Write-Output ('FIRST_'+'ROUTE')",'FIRST_ROUTE');
 s=await state();await click(s.rendered.controls.new_terminal);await until(s=>s.sessions.length===2&&s.rendered.terminals.some(t=>t.session_id==='2'&&t.state==='running'));
 await command("$env:OMNITERM_OPEN_PIPE=Get-Content endpoint.txt; Set-Location sub");
 await denied('omni open report.txt','cross_session');
 check('another_live_session_cannot_reuse_the_first_sessions_endpoint');
 assert.deepEqual(await app.request('close'),{result:{closing:true}});assert.equal((await app.closed).code,0);
 evidence.result='PASS';console.log(`PASS: ${evidence.checks.length} native text-open checks`);
}catch(error){evidence.result='FAIL';evidence.failure=String(error);throw error;}
finally{if(app&&app.child.exitCode===null)await app.stop();evidence.finished=new Date().toISOString();await writeFile(resolve(output,'result.json'),JSON.stringify(evidence,null,2));}
