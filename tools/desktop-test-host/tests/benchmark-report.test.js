import assert from 'node:assert/strict';
import {test} from 'node:test';
import {CASES, collectCases, compareReports, distribution, frameMetrics, idleSignature, resourceMetrics, timingMetrics} from '../benchmark-report.js';

test('child terminal benchmarks require all launches, input samples and owned resources', async () => {
  const result={result:'PASS',checks:Array(10).fill({result:'PASS'}),measurements:{child_launch_ms:[100,110,120],child_input_ms:Array(10).fill(100)}};
  const read=async path=>path==='child/result.json'?result:counters();
  const cases=await collectCases('child','full',read);
  assert.deepEqual(cases.map(c=>c.id),['B-CHILD-01','B-CHILD-02','B-CHILD-03']);
  assert.equal(cases[0].metrics.latency_ms.n,3);
  result.measurements.child_launch_ms.pop();await assert.rejects(collectCases('child','full',read));
  result.measurements.child_launch_ms.push(120);result.measurements.child_input_ms.pop();
  await assert.rejects(collectCases('child','full',read));
});

function counters() {
  const processes = cpu => [
    {id:1,start_ticks:'100',name:'omniterm-test-host',file_version:'1',cpu_seconds:cpu,private_bytes:1048576,working_set_bytes:2097152},
    {id:2,start_ticks:'101',name:'msedgewebview2',file_version:'2',cpu_seconds:0,private_bytes:1048576,working_set_bytes:1048576},
  ];
  return {root_id:1,root_start_ticks:'100',logical_cpus:4,powershell_version:'7.6.5',duration_ms:60001,
    samples:[
      {elapsed_ms:1,cpu_one_core_percent:0,cpu_machine_percent:0,private_bytes:2097152,working_set_sum_bytes:3145728,processes:processes(0)},
      {elapsed_ms:60001,cpu_one_core_percent:0.025,cpu_machine_percent:0.00625,private_bytes:2097152,working_set_sum_bytes:3145728,processes:processes(0.015)},
    ]};
}

test('distributions use median and nearest-rank p95, retaining sample count', () => {
  assert.deepEqual(distribution([4,1,2,3]),{n:4,min:1,p50:2.5,p95:4,max:4});
  for (const input of [[],[NaN],[Infinity],[-1],['1']]) assert.throws(() => distribution(input));
});

test('fractional CPU uses actual elapsed time and memory retains distinct units', () => {
  const result = resourceMetrics(counters(),60);
  assert.equal(result.cpu_one_core_percent,0.025);
  assert.equal(result.cpu_machine_percent,0.00625);
  assert.equal(result.private_allocated_mib.p50,2);
  assert.equal(result.working_set_sum_mib.p50,3);
  assert.equal(result.interval_ms.max,60000);
});

test('invalid aggregates, missing root/WebView and short or incomplete counters cannot pass', () => {
  for (const mutate of [
    r => {r.samples[1].cpu_one_core_percent=0;},
    r => {r.samples[1].cpu_machine_percent=0;},
    r => {r.samples[1].private_bytes=0;},
    r => {r.samples[1].elapsed_ms=0;},
    r => {r.samples[1].processes[0].unavailable=true;},
    r => {r.samples.forEach(s => s.processes[1].name='other');},
    r => {r.duration_ms=59999;},
    r => {r.samples[1].processes[0].cpu_seconds=-1;},
    r => {r.samples[1].processes.push({...r.samples[1].processes[0]});},
  ]) {const raw=counters();mutate(raw);assert.throws(() => resourceMetrics(raw,60));}
});

test('PID reuse and unavailable short-lived children do not add old CPU', () => {
  const raw=counters();
  raw.samples[0].processes.push({id:3,start_ticks:'102',name:'child',cpu_seconds:100,private_bytes:0,working_set_bytes:0});
  raw.samples[1].processes.push({id:3,start_ticks:'103',name:'child',cpu_seconds:200,private_bytes:0,working_set_bytes:0},{id:4,unavailable:true});
  assert.equal(resourceMetrics(raw,60).cpu_one_core_percent,0.025);
});

test('timing and frame targets describe observations and reject partial collections', () => {
  assert.equal(timingMetrics([80,90,100,120,251],5,250).target.status,'OBSERVED_EXCEEDED');
  assert.equal(timingMetrics([80,90,100,120,150],5,250).target.status,'OBSERVED_WITHIN');
  assert.throws(() => timingMetrics([80],5,250));
  assert.throws(() => frameMetrics({intervals_ms:[16,16],elapsed_ms:32,visibility:'visible'}));
  const frame=frameMetrics({intervals_ms:Array(10).fill(16),elapsed_ms:161,visibility:'visible'});
  assert.equal(frame.target.status,'OBSERVED_WITHIN');
  assert.throws(() => frameMetrics({intervals_ms:Array(10).fill(16),elapsed_ms:20,visibility:'visible'}));
  assert.throws(() => frameMetrics({intervals_ms:Array(10).fill(16),elapsed_ms:161,visibility:'hidden'}));
});

const report = value => ({schema:1,suite_revision:1,status:'COLLECTED',mode:'full',group:'workflow',
  conditions:{machine:{id:'host',logical_cpus:4},build_profile:'release',display:{scale:1},tools:{node:'24',webview:['2']}},
  source:{commit:'abc',dirty:false},binaries:{host:'abc'},
  cases:[{id:'B-INPUT-01',revision:1,metrics:{latency_ms:{n:20,p50:value,p95:value+1,max:value+2}}}]});

test('compatible baselines allow implementation changes but report observations, never significance', () => {
  const before=report(100),after=report(80);after.source.commit='def';after.binaries.host='def';
  const result=compareReports(after,before);
  assert.equal(result.status,'COMPARABLE');
  assert.deepEqual(result.deltas.find(d => d.metric==='latency_ms.p50'),{id:'B-INPUT-01',metric:'latency_ms.p50',before:100,after:80,absolute:-20,percent:-20});
  assert.match(result.limit,/significance/);
});

test('smoke, changed conditions, failed reports and changed case revisions cannot produce baseline deltas', () => {
  for (const mutate of [
    r => {r.mode='smoke';},r => {r.status='FAILED';},r => {r.schema=2;},
    r => {r.conditions.machine.id='other';},r => {r.conditions.display.scale=2;},
    r => {r.conditions.tools.webview=['3'];},r => {r.cases[0].revision=2;},
    r => {r.cases=[];},r => {r.cases[0].metrics.latency_ms.p50=NaN;},
    r => {delete r.cases[0].metrics.latency_ms.p95;},
  ]) {
    const before=report(100),after=report(80);mutate(after);
    const result=compareReports(after,before);
    assert.equal(result.status,'INCOMPARABLE');assert.deepEqual(result.deltas,[]);
  }
});

test('zero baselines have no invented percentage and repeated n is not a performance delta', () => {
  const before=report(0),after=report(1);
  const result=compareReports(after,before);
  assert.equal(result.deltas.find(d => d.metric==='latency_ms.p50').percent,null);
  assert(!result.deltas.some(d => d.metric.endsWith('.n')));
});

test('placement catalog collects exactly the seven runnable revisions, rejecting partial output', async () => {
  const raw=Array.from({length:7},(_,i)=>({id:'B-PLACE-0'+(i+1),revision:1,blocks:256,iterations_per_batch:2,batch_mean_ns:Array(12).fill(10)}));
  const cases=await collectCases('placement','full',async path=>{assert.equal(path,'placement.json');return raw;});
  assert.equal(cases.length,7);assert.equal(new Set(CASES.map(c=>c.id)).size,CASES.length);
  assert.equal(cases[0].metrics.batch_mean_ns.p50,10);
  raw[0].batch_mean_ns.pop();await assert.rejects(collectCases('placement','full',async()=>raw));
});

test('idle workload signature detects unsolicited clicks, keys, camera, layout and task changes', () => {
  const state={layout:{revision:'1'},rendered:{camera:{zoom:67},input_events:{clicks:39,key_downs:34}},sessions:[{session_id:'1'}],git_tasks:[{task_id:'1'}]};
  const before=idleSignature(state);
  assert.deepEqual(idleSignature(structuredClone(state)),before);
  for(const mutate of [s=>s.rendered.input_events.clicks++,s=>s.rendered.input_events.key_downs++,s=>s.rendered.camera.zoom=125,s=>s.layout.revision='2',s=>s.git_tasks.push({task_id:'2'}),s=>s.sessions=[]]) {
    const after=structuredClone(state);mutate(after);assert.notDeepEqual(idleSignature(after),before);
  }
});

test('terminal benchmark rejects partial navigation/input and short resource windows', async () => {
  const result={result:'PASS',checks:Array(9).fill({result:'PASS'}),measurements:{history_navigation_ms:Array(20).fill(20),input_after_history_ms:Array(10).fill(100),live_snapshot_json_bytes:Array(20).fill(4096)}};
  const read=async path=>path==='terminal/result.json'?result:counters();
  const cases=await collectCases('terminal','full',read);
  assert.deepEqual(cases.map(c=>c.id),['B-TERM-01','B-TERM-02','B-TERM-03','B-TERM-04']);
  assert.equal(cases[0].metrics.latency_ms.n,20);
  result.measurements.history_navigation_ms.pop();await assert.rejects(collectCases('terminal','full',read));
  result.measurements.history_navigation_ms.push(20);
  await assert.rejects(collectCases('terminal','full',async path=>path==='terminal/result.json'?result:{...counters(),duration_ms:14000}));
});
