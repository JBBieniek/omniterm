import assert from 'node:assert/strict';

export const SUITE_REVISION = 1;
export const CASES = [
  ['B-STARTUP-01','production','Six ordinary launches: fresh/warm WebView profiles; native window-ready proxy'],
  ['B-IDLE-01','production','Ordinary empty app: >=60s owned process-tree CPU/private allocation/working sets'],
  ['B-STARTUP-02','workflow','Same-factory rendered startup: one controller-observed sample'],
  ['B-IDLE-02','workflow','Empty test host: >=60s process-tree counters'],
  ['B-IDLE-03','workflow','One visible terminal: >=60s process-tree counters'],
  ['B-IDLE-04','workflow','Terminal + 104-byte text + 1/1/1 Git status: >=60s process-tree counters'],
  ['B-INPUT-01','workflow','20 Enter requests to observed output, including IPC/inspection'],
  ['B-INTERRUPT-01','workflow','Five finite output bursts: interrupt request to observed prompt'],
  ['B-OUTPUT-01','workflow','20s window containing finite bursts, recovery and idle; not continuous output CPU'],
  ['B-FRAMES-01','workflow','Three-view zoom: bounded rAF callback intervals, not display presentation'],
  ['B-FRAMES-02','workflow','Three-view output: bounded rAF callback intervals, not display presentation'],
  ['B-HELPER-01','workflow','Small real Git fixture: one helper request to observed counts sample'],
  ['B-PLACE-01','placement','first_fit: empty default 48x32 grid, 12x8 request'],
  ['B-PLACE-02','placement','first_fit: three 12x8 blocks on default grid, earliest free position'],
  ['B-PLACE-03','placement','first_fit: full default grid, 16 blocks, no fit'],
  ['B-PLACE-04','placement','first_fit: fragmented 32x32 grid, 256 separated cells, no 2x2 fit'],
  ['B-PLACE-05','placement','first_fit: maximum 512x512 grid, 256 full-height strips, no fit'],
  ['B-PLACE-06','placement','place/validate_rect: accepted self-placement among 256 blocks'],
  ['B-PLACE-07','placement','place/validate_rect: collision with last of 256 blocks'],
  ['B-TERM-01','terminal','20 older/newer captured-page transitions, controller request to observed page'],
  ['B-TERM-02','terminal','10 input/output round trips after history selection and background output'],
  ['B-TERM-03','terminal','15s owned tree window containing history/input/resize/recovery and idle; not continuous-load CPU'],
  ['B-TERM-04','terminal','20 UTF-8 sizes of returned live ScreenSnapshot JSON representations; excludes invoke envelope/framing'],
  ['B-CHILD-01','child','Three fixed-shell helper launch workflows to observed linked terminal: two, three, four retained Sessions; controller input/IPC/inspection included'],
  ['B-CHILD-02','child','10 Enter requests to observed output in the separate child Session'],
  ['B-CHILD-03','child','15s owned tree window starting with two Sessions, input/resize/helper/view recovery and idle; not idle-only or a scaling rate'],
  ['B-IDLE-05','idle','Empty same-factory app, >=60s owned-process counters; no controller inspection during sampling'],
  ['B-IDLE-06','idle','One visible unfocused quiet Session, >=60s counters and stable source/view signatures'],
  ['B-IDLE-07','idle','Same live Session with its Block removed, >=60s owned-process counters'],
  ['B-IDLE-08','idle','Same Session with frozen selected reading page, >=60s owned-process counters'],
  ['B-IDLE-09','idle','Same quiet Session focused with cursor animation, >=60s owned-process counters'],
  ['B-UPDATES-01','idle','Test-host-only screen/list/other invoke deltas and rates for all five >=60s idle controls; no request contents retained'],
  ['B-COLD-01','idle','Two independent epoch-ms observations after >=60s quiet: external visible output and frozen-page exit; includes file/output/render/inspection'],
  ['B-INPUT-02','idle','10 Enter-to-rendered-output samples after the long-idle visible wake'],
  ['B-LAUNCH-01','launch','One JSON-selected native fixture helper launch to rendered ready; argv/cwd/stdio/IPC included'],
  ['B-LAUNCH-02','launch','One explicitly selected Python helper launch to rendered interactive prompt; different workload, not a language comparison'],
  ['B-LAUNCH-03','launch','10 native fixture input requests to rendered reply observations'],
  ['B-LAUNCH-04','launch','15s owned-tree mixed fixture/input/resize/helper/view/source-exit and idle window; no per-child or idle-only inference'],
].map(([id,group,workload]) => ({id,revision:1,group,workload}));

function nonnegative(value) {assert(typeof value==='number'&&Number.isFinite(value)&&value>=0,'Expected a finite nonnegative number');return value;}
export function idleSignature(state) {
  return {layout:state.layout,camera:state.rendered.camera,clicks:state.rendered.input_events.clicks,
    keys:state.rendered.input_events.key_downs,sessions:state.sessions.map(s=>s.session_id),tasks:state.git_tasks.map(t=>t.task_id)};
}
export function distribution(values) {
  assert(Array.isArray(values)&&values.length>0,'Missing samples');
  const sorted=values.map(nonnegative).sort((a,b)=>a-b),n=sorted.length;
  return {n,min:sorted[0],p50:n%2?sorted[(n-1)/2]:(sorted[n/2-1]+sorted[n/2])/2,p95:sorted[Math.ceil(n*.95)-1],max:sorted[n-1]};
}

export function resourceMetrics(raw,minSeconds) {
  assert(raw.samples?.length>1&&Number.isInteger(raw.logical_cpus)&&raw.logical_cpus>0,'Incomplete process counters');
  assert(nonnegative(raw.duration_ms)>=minSeconds*1000,'Short resource window');
  const previous=new Map(),names=new Set(),versions=new Set(),intervals=[];
  let last=0,cpuSeconds=0,duration=0;
  for (const [index,sample] of raw.samples.entries()) {
    const identities=new Set();let cpu=0,privateBytes=0,workingSet=0,root=false;
    nonnegative(sample.elapsed_ms);
    for (const p of sample.processes) {
      if (p.unavailable) continue;
      assert(Number.isInteger(p.id)&&p.id>0&&typeof p.start_ticks==='string'&&/^\d+$/.test(p.start_ticks),'Invalid process identity');
      const identity=p.id+'/'+p.start_ticks;assert(!identities.has(identity),'Duplicate process identity');identities.add(identity);
      root ||= p.id===raw.root_id&&p.start_ticks===raw.root_start_ticks;
      nonnegative(p.cpu_seconds);nonnegative(p.private_bytes);nonnegative(p.working_set_bytes);
      if (previous.has(identity)) {assert(p.cpu_seconds>=previous.get(identity),'CPU counter went backwards');cpu+=p.cpu_seconds-previous.get(identity);}
      previous.set(identity,p.cpu_seconds);names.add(p.name);
      if (p.name==='msedgewebview2'&&p.file_version) versions.add(p.file_version);
      privateBytes+=p.private_bytes;workingSet+=p.working_set_bytes;
    }
    assert(root,'Root counters unavailable');
    assert(privateBytes>0&&workingSet>0,'Missing memory counters');
    assert.equal(sample.private_bytes,privateBytes);assert.equal(sample.working_set_sum_bytes,workingSet);
    const interval=sample.elapsed_ms-last;assert(!index||interval>0,'Nonmonotonic timestamps');
    const expected=index?100000*cpu/interval:0;
    assert(Math.abs(nonnegative(sample.cpu_one_core_percent)-expected)<1e-6,'CPU aggregate disagrees with raw counters');
    assert(Math.abs(nonnegative(sample.cpu_machine_percent)-expected/raw.logical_cpus)<1e-6,'Machine CPU aggregate disagrees');
    if (index) {intervals.push(interval);cpuSeconds+=cpu;duration+=interval;}
    last=sample.elapsed_ms;
  }
  assert(names.has('msedgewebview2'),'WebView child counters missing');
  assert(raw.duration_ms>=last&&duration>=minSeconds*1000,'Incomplete elapsed window');
  return {duration_seconds:raw.duration_ms/1000,samples:raw.samples.length,
    cpu_one_core_percent:100000*cpuSeconds/duration,cpu_machine_percent:100000*cpuSeconds/duration/raw.logical_cpus,
    private_allocated_mib:distribution(raw.samples.map(s=>s.private_bytes/1048576)),
    working_set_sum_mib:distribution(raw.samples.map(s=>s.working_set_sum_bytes/1048576)),
    interval_ms:distribution(intervals),process_names:[...names].sort(),webview_versions:[...versions].sort()};
}

export function timingMetrics(values,count,budget) {
  assert.equal(values?.length,count,'Partial timing workload');
  const latency_ms=distribution(values),result={latency_ms};
  if (budget!==undefined) result.target={metric:'latency_ms.max',limit:budget,unit:'ms',status:latency_ms.max<=budget?'OBSERVED_WITHIN':'OBSERVED_EXCEEDED'};
  return result;
}

export function frameMetrics(raw) {
  assert(raw.intervals_ms?.length>5&&raw.intervals_ms.length<=512,'Incomplete frame observation');
  assert.equal(raw.visibility,'visible','Frame window was hidden at end');
  const callback_interval_ms=distribution(raw.intervals_ms),captured_ms=raw.intervals_ms.reduce((a,b)=>a+b,0);
  assert(captured_ms<=10050&&nonnegative(raw.elapsed_ms)+1>=captured_ms,'Invalid bounded frame window');
  return {callback_interval_ms,captured_ms,total_begin_end_ms:raw.elapsed_ms,
    target:{metric:'callback_interval_ms.p95',limit:33,unit:'ms',status:callback_interval_ms.p95<=33?'OBSERVED_WITHIN':'OBSERVED_EXCEEDED',meaning:'three-view diagnostic only; 100-block/display target NOT RUN'}};
}

function numericMetrics(object,prefix='',result={}) {
  assert(object&&typeof object==='object','Missing metrics');
  for (const [key,value] of Object.entries(object)) {
    const path=prefix?prefix+'.'+key:key;
    if (['target','n','samples','iterations_per_batch','blocks'].includes(key)) continue;
    if (typeof value==='number') result[path]=nonnegative(value);
    else if (value&&typeof value==='object'&&!Array.isArray(value)) numericMetrics(value,path,result);
  }
  return result;
}

export function compareReports(current,baseline) {
  const limit='Single-run deltas are observations, not statistical significance; background load, thermal/power state and observer effects remain uncontrolled.';
  try {
    for (const r of [current,baseline]) {
      assert.equal(r.schema,1);assert.equal(r.status,'COLLECTED');assert.equal(r.mode,'full');assert(r.cases?.length>0);
    }
    assert.equal(current.suite_revision,baseline.suite_revision);assert.equal(current.group,baseline.group);
    assert.deepEqual(current.conditions,baseline.conditions,'Machine/display/tool/build/instrumentation conditions differ');
    assert.deepEqual(current.cases.map(c=>[c.id,c.revision]),baseline.cases.map(c=>[c.id,c.revision]),'Case inventory/revisions differ');
    const deltas=[];
    for (const [index,c] of current.cases.entries()) {
      const after=numericMetrics(c.metrics),before=numericMetrics(baseline.cases[index].metrics);
      assert.deepEqual(Object.keys(after),Object.keys(before),'Metric coverage differs');
      for (const [metric,value] of Object.entries(after)) deltas.push({id:c.id,metric,before:before[metric],after:value,absolute:value-before[metric],percent:before[metric]===0?null:100*(value-before[metric])/before[metric]});
    }
    return {status:'COMPARABLE',deltas,limit};
  } catch (error) {return {status:'INCOMPARABLE',reason:error.message,deltas:[],limit};}
}

export async function collectCases(group,mode,read) {
  const cases=[],seconds=mode==='smoke'?0:60;
  function add(id,source,metrics) {cases.push({...CASES.find(c=>c.id===id),source,metrics});}
  if (group==='production'||group==='all') {
    const result=await read('production/result.json');assert.equal(result.result,'PASS');assert.equal(result.startup.length,6);
    const fresh=result.startup.filter(s=>s.case.startsWith('fresh-')),warm=result.startup.filter(s=>s.case.startsWith('warm-'));
    assert.deepEqual(result.startup.map(s=>s.case),['fresh-1','fresh-2','fresh-3','warm-1','warm-2','warm-3']);
    add('B-STARTUP-01','production/result.json',{fresh_window_proxy_ms:distribution(fresh.map(s=>s.window_ready_proxy_ms)),warm_window_proxy_ms:distribution(warm.map(s=>s.window_ready_proxy_ms))});
    add('B-IDLE-01','production/idle.json',resourceMetrics(await read('production/idle.json'),60));
  }
  if (group==='workflow'||group==='all') {
    const result=await read('workflow/result.json');assert.equal(result.result,'PASS');assert.equal(result.measurement_mode,mode==='smoke'?'harness smoke':'full');
    assert.equal(result.checks.length,7);assert(result.checks.every(c=>c.result==='PASS'));
    assert.deepEqual(result.idle_observations?.map(o=>o.name),['empty-idle','terminal-idle','three-view-idle']);
    for(const observation of result.idle_observations) assert.deepEqual(observation.after,observation.before,'Idle workload changed');
    const m=result.measurements;
    add('B-STARTUP-02','workflow/result.json',timingMetrics([m.same_factory_rendered_startup_ms],1));
    for (const [id,name] of [['B-IDLE-02','empty-idle'],['B-IDLE-03','terminal-idle'],['B-IDLE-04','three-view-idle']]) {
      add(id,'workflow/'+name+'.json',resourceMetrics(await read('workflow/'+name+'.json'),seconds));
    }
    add('B-INPUT-01','workflow/result.json',timingMetrics(m.input_request_to_observed_ms,20));
    add('B-INTERRUPT-01','workflow/result.json',timingMetrics(m.interrupt_request_to_prompt_ms,5,250));
    add('B-OUTPUT-01','workflow/bounded-output.json',resourceMetrics(await read('workflow/bounded-output.json'),mode==='smoke'?0:20));
    for (const [id,name] of [['B-FRAMES-01','interaction-frames'],['B-FRAMES-02','output-frames']]) add(id,'workflow/'+name+'.json',frameMetrics(await read('workflow/'+name+'.json')));
    add('B-HELPER-01','workflow/result.json',timingMetrics([m.git_helper_to_observed_ms],1));
  }
  if (group==='placement'||group==='all') {
    const raw=await read('placement.json');assert.equal(raw.length,7);
    for (const [index,item] of raw.entries()) {
      assert.equal(item.id,'B-PLACE-0'+(index+1));assert.equal(item.revision,1);
      assert.equal(item.batch_mean_ns.length,12);assert(Number.isInteger(item.iterations_per_batch)&&item.iterations_per_batch>0);
      add(item.id,'placement.json',{batch_mean_ns:distribution(item.batch_mean_ns),iterations_per_batch:item.iterations_per_batch,blocks:item.blocks});
    }
  }
  if (group==='terminal'||group==='all') {
    const result=await read('terminal/result.json');assert.equal(result.result,'PASS');
    assert.equal(result.checks.length,9);assert(result.checks.every(c=>c.result==='PASS'));
    add('B-TERM-01','terminal/result.json',timingMetrics(result.measurements.history_navigation_ms,20));
    add('B-TERM-02','terminal/result.json',timingMetrics(result.measurements.input_after_history_ms,10));
    add('B-TERM-03','terminal/resources.json',resourceMetrics(await read('terminal/resources.json'),15));
    assert.equal(result.measurements.live_snapshot_json_bytes?.length,20);
    add('B-TERM-04','terminal/result.json',{serialized_snapshot_bytes:distribution(result.measurements.live_snapshot_json_bytes)});
  }
  if (group==='child'||group==='all') {
    const result=await read('child/result.json');assert.equal(result.result,'PASS');
    assert.equal(result.checks.length,10);assert(result.checks.every(c=>c.result==='PASS'));
    add('B-CHILD-01','child/result.json',timingMetrics(result.measurements.child_launch_ms,3));
    add('B-CHILD-02','child/result.json',timingMetrics(result.measurements.child_input_ms,10));
    add('B-CHILD-03','child/resources.json',resourceMetrics(await read('child/resources.json'),15));
  }
  if (group==='idle'||group==='all') {
    const result=await read('idle/result.json');assert.equal(result.result,'PASS');
    assert.equal(result.checks.length,6);assert(result.checks.every(c=>c.result==='PASS'));
    assert.deepEqual(result.idle_observations?.map(o=>o.name),['empty','visible','focused','hidden','reading']);
    for (const [id,name] of [['B-IDLE-05','empty'],['B-IDLE-06','visible'],['B-IDLE-07','hidden'],['B-IDLE-08','reading'],['B-IDLE-09','focused']]) {
      add(id,'idle/'+name+'.json',resourceMetrics(await read('idle/'+name+'.json'),60));
    }
    const requests={};
    for (const o of result.idle_observations) {
      assert.deepEqual(o.after,o.before,'Disturbed idle control');assert(nonnegative(o.elapsed_seconds)>=60);
      assert.deepEqual(Object.keys(o.requests).sort(),['list','other','screen']);
      for (const n of Object.values(o.requests)) assert(Number.isSafeInteger(n)&&nonnegative(n)>=0);
      requests[o.name]={...o.requests,reads_per_second:(o.requests.screen+o.requests.list)/o.elapsed_seconds};
    }
    add('B-UPDATES-01','idle/result.json',requests);
    assert.equal(result.measurements.cold_output_ms?.length,2);
    add('B-COLD-01','idle/result.json',{visible_output_ms:nonnegative(result.measurements.cold_output_ms[0]),frozen_exit_ms:nonnegative(result.measurements.cold_output_ms[1])});
    add('B-INPUT-02','idle/result.json',timingMetrics(result.measurements.input_ms,10));
  }
  if (group==='launch'||group==='all') {
    const result=await read('launch/result.json');assert.equal(result.result,'PASS');
    assert.equal(result.checks.length,11);assert(result.checks.every(c=>c.result==='PASS'));
    add('B-LAUNCH-01','launch/result.json',timingMetrics(result.measurements.fixture_launch_ms,1));
    add('B-LAUNCH-02','launch/result.json',timingMetrics(result.measurements.python_launch_ms,1));
    add('B-LAUNCH-03','launch/result.json',timingMetrics(result.measurements.fixture_input_ms,10));
    add('B-LAUNCH-04','launch/resources.json',resourceMetrics(await read('launch/resources.json'),15));
  }
  return cases;
}
