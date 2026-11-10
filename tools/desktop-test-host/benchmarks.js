import assert from 'node:assert/strict';
import {spawn} from 'node:child_process';
import {createHash} from 'node:crypto';
import {mkdir,readFile,writeFile} from 'node:fs/promises';
import os from 'node:os';
import {dirname,join,resolve} from 'node:path';
import {fileURLToPath} from 'node:url';
import {parseArgs} from 'node:util';
import {CASES,SUITE_REVISION,collectCases,compareReports} from './benchmark-report.js';

const root=resolve(dirname(fileURLToPath(import.meta.url)),'../..');
const {values,positionals}=parseArgs({allowPositionals:true,options:Object.fromEntries(
  ['group','mode','out','host','app','git','powershell','baseline','build-profile','current'].map(key=>[key,{type:'string'}]))});
const [command]=positionals;
assert.equal(positionals.length,1,'Use: benchmarks.js list | run --out NEW_DIR --git GIT_EXE [--group all|production|workflow|placement|terminal|child|idle] [--mode full|smoke] [--host EXE --app EXE --powershell EXE] [--baseline summary.json] | compare --current summary.json --baseline summary.json');
const json=async path=>JSON.parse((await readFile(path,'utf8')).replace(/^\uFEFF/,''));
if (command==='list') {console.log(JSON.stringify({suite_revision:SUITE_REVISION,cases:CASES},null,2));}
else if (command==='compare') {
  assert(values.current&&values.baseline,'Both --current and --baseline are required');
  const result=compareReports(await json(values.current),await json(values.baseline));
  console.log(JSON.stringify(result,null,2));if(result.status!=='COMPARABLE') process.exitCode=2;
} else if (command==='run') {await run();}
else throw new Error('Unknown command: '+command);

async function run() {
  const group=values.group??'all',mode=values.mode??'full',buildProfile=values['build-profile']??'release';
  assert(['all','production','workflow','placement','terminal','child','idle'].includes(group)&&['full','smoke'].includes(mode),'Unknown group/mode');
  assert(mode!=='smoke'||group==='workflow','Smoke is only a shortened workflow harness check');
  assert.equal(buildProfile,'release','Native binaries must be built with --release; placement uses the Cargo bench profile');
  assert(values.out&&values.git,'--out and --git are required');
  const native=group!=='placement',workflow=group==='workflow'||group==='all',production=group==='production'||group==='all',terminal=group==='terminal'||group==='all',child=group==='child'||group==='all',idle=group==='idle'||group==='all';
  assert(!native||process.platform==='win32','Native probes currently require Windows');
  assert(!native||values.powershell,'--powershell is required for native sampling');
  assert(!(workflow||terminal||child||idle)||values.host,'--host is required');assert(!production||values.app,'--app is required');
  const out=resolve(values.out),git=resolve(values.git),powershell=values.powershell&&resolve(values.powershell);
  // Never reuse, overwrite or delete a previous run/profile, even after failure.
  await mkdir(out);
  const evidence={schema:1,suite_revision:SUITE_REVISION,status:'RUNNING',group,mode,started:new Date().toISOString(),
    conditions:{build_profile:buildProfile,machine:{id:createHash('sha256').update(os.hostname()).digest('hex'),platform:os.platform(),arch:os.arch(),os_release:os.release(),os_version:os.version(),cpu:os.cpus()[0]?.model,logical_cpus:os.cpus().length,total_memory_bytes:os.totalmem()},tools:{node:process.version}},
    cases:[],limits:[
      'Collection validity and observed targets are separate; no significance claim from a single run.',
      'Private allocated/committed bytes are not resident RAM; working-set sums may duplicate shared pages.',
      'Sampled descendants can miss brief/brokered processes; observer/background/thermal/power effects remain uncontrolled.',
      'Window-ready is a native proxy, fresh profile is not OS-cold boot; controller timings include IPC and inspection.',
      'rAF callback intervals are not compositor/display timings; 100-block target NOT RUN.',
      'GPU, power, network/storage I/O, 50MB retained log, multi-Session scaling, human/IME/other-platform checks NOT MEASURED by this suite.',
      'Placement timings are batch means after warmup/calibration, not individual-call tail latency or native UI responsiveness.',
    ]};
  const hash=async path=>createHash('sha256').update(await readFile(path)).digest('hex');
  async function execute(label,binary,args) {
    console.log('Running '+label);
    const child=spawn(binary,args,{cwd:root,windowsHide:true,stdio:['ignore','pipe','pipe']});
    let stdout='',stderr='';child.stdout.on('data',b=>stdout+=b);child.stderr.on('data',b=>stderr+=b);
    const code=await new Promise((done,reject)=>{child.once('error',reject);child.once('close',done);});
    await writeFile(join(out,label+'.stdout.txt'),stdout);await writeFile(join(out,label+'.stderr.txt'),stderr);
    assert.equal(code,0,label+' failed; see retained stdout/stderr');return stdout.trim();
  }
  try {
    evidence.conditions.tools.git=await execute('git-version',git,['--version']);
    const gitArgs=['-c','safe.directory='+root.replaceAll('\\','/')];
    evidence.source={commit:await execute('source-commit',git,[...gitArgs,'rev-parse','HEAD']),dirty:!!await execute('source-status',git,[...gitArgs,'status','--porcelain=v1','--untracked-files=all'])};
    // Fingerprint changed/untracked implementation too; ignored local records are deliberately excluded.
    const paths=(await execute('source-paths',git,[...gitArgs,'ls-files','--cached','--others','--exclude-standard'])).split(/\r?\n/).filter(Boolean).sort();
    const sourceHash=createHash('sha256');
    for (const path of paths) {sourceHash.update(path+'\0');try{sourceHash.update(await readFile(join(root,path)));}catch(e){if(e.code!=='ENOENT')throw e;sourceHash.update('<deleted>');}}
    evidence.source.implementation_sha256=sourceHash.digest('hex');
    const instruments=['benchmark-report.js','benchmarks.js'];
    if (native) instruments.push('client.js','src/automation.rs','tests/resource-probe.ps1');
    if (workflow) instruments.push('tests/evaluation-smoke.js');
    if (terminal) instruments.push('tests/terminal-ux-smoke.js');
    if (child) instruments.push('tests/child-terminal-smoke.js');
    if (idle) instruments.push('tests/idle-smoke.js');
    if (production) instruments.push('tests/production-probe.ps1');
    evidence.conditions.instrumentation={};
    for (const path of instruments) evidence.conditions.instrumentation[path]=await hash(join(root,'tools/desktop-test-host',path));
    evidence.binaries={};
    if (workflow||terminal||child||idle) {evidence.binaries.host=await hash(resolve(values.host));evidence.binaries.helper=await hash(join(dirname(resolve(values.host)),'omni.exe'));}
    if (production) evidence.binaries.app=await hash(resolve(values.app));
    if (native) {
      evidence.conditions.display=JSON.parse(await execute('display',powershell,['-NoProfile','-Command',
        "Add-Type -AssemblyName System.Windows.Forms; @{powershell=$PSVersionTable.PSVersion.ToString();screens=@([Windows.Forms.Screen]::AllScreens | ForEach-Object {@{primary=$_.Primary;bits=$_.BitsPerPixel;x=$_.Bounds.X;y=$_.Bounds.Y;width=$_.Bounds.Width;height=$_.Bounds.Height}})} | ConvertTo-Json -Depth 4 -Compress"]));
    }
    if (group==='placement'||group==='all') {
      evidence.conditions.tools.rustc=await execute('rustc-version','rustc',['--version']);
      evidence.conditions.instrumentation['placement.rs']=await hash(join(root,'crates/omniterm-core/benches/placement.rs'));
      const stdout=await execute('placement','cargo',['bench','-p','omniterm-core','--bench','placement','--locked','--offline']);
      await writeFile(join(out,'placement.json'),JSON.stringify(stdout.split(/\r?\n/).filter(s=>s.startsWith('OMNITERM_BENCH ')).map(s=>JSON.parse(s.slice(15))),null,2));
    }
    if (production) await execute('production',powershell,['-NoProfile','-ExecutionPolicy','Bypass','-File',join(root,'tools/desktop-test-host/tests/production-probe.ps1'),'-Binary',resolve(values.app),'-OutputDirectory',join(out,'production')]);
    if (workflow) await execute('workflow',process.execPath,[join(root,'tools/desktop-test-host/tests/evaluation-smoke.js'),resolve(values.host),join(out,'workflow'),git,mode,powershell]);
    if (terminal) await execute('terminal',process.execPath,[join(root,'tools/desktop-test-host/tests/terminal-ux-smoke.js'),resolve(values.host),join(out,'terminal'),powershell]);
    if (child) await execute('child',process.execPath,[join(root,'tools/desktop-test-host/tests/child-terminal-smoke.js'),resolve(values.host),join(out,'child'),powershell]);
    if (idle) await execute('idle',process.execPath,[join(root,'tools/desktop-test-host/tests/idle-smoke.js'),resolve(values.host),join(out,'idle'),powershell]);
    evidence.cases=await collectCases(group,mode,path=>json(join(out,path)));
    if (native) {
      const resources=evidence.cases.filter(c=>c.metrics.webview_versions);
      evidence.conditions.tools.webview=[...new Set(resources.flatMap(c=>c.metrics.webview_versions))].sort();
      assert(evidence.conditions.tools.webview.length>0&&!evidence.conditions.tools.webview.includes('unavailable'),'Missing WebView runtime version');
      assert(resources.every(c=>c.metrics.webview_versions.length>0),'Missing WebView version in a workload');
      const probe=await json(join(out,production?'production/idle.json':workflow?'workflow/empty-idle.json':terminal?'terminal/resources.json':child?'child/resources.json':'idle/empty.json'));
      evidence.conditions.tools.sampler=probe.powershell_version;
      if (workflow) {
        const {width,height,scale_factor}= (await json(join(out,'workflow/three-view.json'))).window;
        evidence.conditions.workflow_window={width,height,scale_factor};
      }
      if (terminal) {
        const {width,height,scale_factor}=(await json(join(out,'terminal/history-selected.json'))).window;
        evidence.conditions.terminal_window={width,height,scale_factor};
      }
      if (child) {
        const {width,height,scale_factor}=(await json(join(out,'child/attached-terminals.json'))).window;
        evidence.conditions.child_window={width,height,scale_factor};
      }
    }
    if (idle) {
      const {width,height,scale_factor}=(await json(join(out,'idle/idle-final.state.json'))).window;
      evidence.conditions.idle_window={width,height,scale_factor,focus_control:(await json(join(out,'idle/result.json'))).focus_control};
    }
    evidence.status='COLLECTED';
    evidence.target_observations=evidence.cases.filter(c=>c.metrics.target).map(c=>({id:c.id,...c.metrics.target}));
    if (values.baseline) {
      evidence.comparison=compareReports(evidence,await json(values.baseline));
      if(evidence.comparison.status!=='COMPARABLE') process.exitCode=2;
    }
    console.log(`${mode==='smoke'?'SMOKE ONLY':'COLLECTED'}: ${evidence.cases.length} cases; ${evidence.target_observations.filter(t=>t.status==='OBSERVED_EXCEEDED').length} observed target exceedances; ${join(out,'summary.json')}`);
  } catch (error) {evidence.status='FAILED';evidence.failure=String(error);process.exitCode=1;console.error(evidence.failure);}
  finally {evidence.finished=new Date().toISOString();await writeFile(join(out,'summary.json'),JSON.stringify(evidence,null,2)+'\n');}
}
