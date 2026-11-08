import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { inspectReady, launchAutomation } from '../client.js';

const [binaryArg, outputArg] = process.argv.slice(2);
assert(binaryArg && outputArg, 'Usage: node tests/placement-smoke.js TEST_HOST NEW_OUTPUT_DIR');
const binary = resolve(binaryArg), output = resolve(outputArg);
await mkdir(output);
const evidence = {platform:process.platform, node:process.version, started:new Date().toISOString(),
  binary_sha256:createHash('sha256').update(await readFile(binary)).digest('hex'), checks:[]};
const check = (name, detail = {}) => evidence.checks.push({name,result:'PASS',...detail});
let app;
const pause = () => new Promise(resolve => setTimeout(resolve,25));
const block = (s,id='1') => s.layout.blocks.find(block => block.id === id);
const node = (s,id='1') => s.rendered.blocks.find(block => block.id === id);
const nextRevision = s => (BigInt(s.layout.revision) + 1n).toString();

function reconciled(s) {
  assert.equal(s.rendered.rendered_revision,s.layout.revision);
  assert.equal(s.rendered.blocks.length,s.layout.blocks.length);
  assert.equal(s.rendered.world.child_count,s.layout.blocks.length + Number(!!s.rendered.placement_preview));
  for (const b of s.layout.blocks) {
    const n = node(s,b.id), cell=s.layout.grid.cell_px;
    assert.equal(n.left,b.rect.x*cell); assert.equal(n.top,b.rect.y*cell);
    assert.equal(n.width,b.rect.width*cell); assert.equal(n.height,b.rect.height*cell);
  }
}
async function state() { await pause(); const s=await inspectReady(app); reconciled(s); return s; }
async function until(predicate) {
  for(let attempt=0;attempt<50;attempt++) {const s=await state(); if(!s.rendered.busy&&predicate(s)) return s;}
  throw new Error('Placement UI did not reach expected state');
}
async function click(r) {
  assert(r&&!r.disabled);
  const x=Math.floor(r.x+Math.min(r.width/2,40)), y=Math.floor(r.y+Math.min(r.height/2,16));
  assert.deepEqual(await app.request({click:{x,y}}),{result:{clicked:true}});
}
async function key(key) {assert.deepEqual(await app.request({key:{key}}),{result:{key_sent:true}});}
async function pointer(phase,x,y) {
  assert.deepEqual(await app.request({pointer:{phase,button:'left',x:Math.round(x),y:Math.round(y)}}),{result:{pointer_sent:true}});
}
async function close() {
  assert.deepEqual(await app.request('close'),{result:{closing:true}});
  assert.equal((await app.closed).code,0);
}
async function shot(name,s) {
  const response=await app.request('screenshot'); assert(!response.error,JSON.stringify(response));
  await writeFile(resolve(output,name+'.png'),Buffer.from(response.result.data,'base64'));
  await writeFile(resolve(output,name+'.json'),JSON.stringify(s??await state(),null,2));
}
async function keyboardChange(s,name,expected) {
  const revision=nextRevision(s), neighbor=block(s,'2');
  await key(name);
  s=await until(s=>s.layout.revision===revision);
  assert.deepEqual(block(s).rect,expected); assert.deepEqual(block(s,'2'),neighbor);
  assert.equal(s.rendered.active.block_id,'1');
  return s;
}

// Pan through ordinary controls/engine drag, never set the camera or model.
async function reveal(world,target) {
  let s=await state();
  if(!s.rendered.pan_tool) await click(s.rendered.controls.pan);
  for(let attempt=0;attempt<20;attempt++) {
    s=await state();
    const v=s.rendered.viewport, c=s.rendered.camera, factor=c.zoom/100;
    const dx=(c.origin.x-world.x)*factor+target.x, dy=(c.origin.y-world.y)*factor+target.y;
    if(Math.abs(dx)<1&&Math.abs(dy)<1) break;
    const start={x:Math.floor(v.width/2),y:Math.floor(v.y+v.height/2)};
    const sx=Math.round(Math.max(-v.width*.3,Math.min(v.width*.3,dx)));
    const sy=Math.round(Math.max(-v.height*.3,Math.min(v.height*.3,dy)));
    await pointer('down',start.x,start.y); await pointer('move',start.x+sx,start.y+sy); await pointer('up',start.x+sx,start.y+sy);
  }
  s=await state(); await click(s.rendered.controls.pan); return state();
}
async function startGesture(kind,dxCells,dyCells,target={x:240,y:160}) {
  let s=await state(); const r=block(s).rect, cell=s.layout.grid.cell_px;
  const point=kind==='move'?{x:r.x*cell+24,y:r.y*cell+16}:{x:(r.x+r.width)*cell-16,y:(r.y+r.height)*cell-16};
  s=await reveal(point,target);
  const rect=kind==='move'?node(s).header:node(s).resize;
  const start={x:Math.round(rect.x+Math.min(rect.width/2,24*s.rendered.camera.zoom/100)),y:Math.round(rect.y+rect.height/2)};
  const end={x:start.x+dxCells*cell*s.rendered.camera.zoom/100,y:start.y+dyCells*cell*s.rendered.camera.zoom/100};
  assert(end.x>=0&&end.x<s.rendered.viewport.width&&end.y>=0&&end.y<s.window.height,'Gesture must stay within bounded controller coordinates');
  const before=s.layout;
  await pointer('down',start.x,start.y); await pointer('move',end.x,end.y);
  const moving=await state(); assert.deepEqual(moving.layout,before);
  assert(moving.rendered.placement_preview,'Local ghost must appear before commit');
  assert.equal(moving.rendered.selected_id,'1');
  return {before,start,end,moving};
}

try {
  for(const zoom of [50,100,200]) {
    app=await launchAutomation(binary);
    let s=await state(); const seed=s.layout;
    await click(node(s).header); s=await state();
    assert.deepEqual(s.layout,seed,'Click without motion is not a placement commit');
    assert.equal(s.rendered.placement_preview,null);
    // Clear the neighbor vertically using focused layout-mode keyboard actions.
    for(let y=1;y<=8;y++) s=await keyboardChange(s,'arrow_down',{x:0,y,width:12,height:8});
    s=await keyboardChange(s,'arrow_right',{x:1,y:8,width:12,height:8});
    s=await keyboardChange(s,'arrow_left',{x:0,y:8,width:12,height:8});
    s=await keyboardChange(s,'arrow_up',{x:0,y:7,width:12,height:8});
    s=await keyboardChange(s,'arrow_down',{x:0,y:8,width:12,height:8});
    s=await keyboardChange(s,'resize_right',{x:0,y:8,width:13,height:8});
    s=await keyboardChange(s,'resize_left',{x:0,y:8,width:12,height:8});
    s=await keyboardChange(s,'resize_down',{x:0,y:8,width:12,height:9});
    s=await keyboardChange(s,'resize_up',{x:0,y:8,width:12,height:8});
    if(zoom===50) {await key('zoom_out'); await key('zoom_out');}
    if(zoom===200) for(let step=0;step<4;step++) await key('zoom_in');
    s=await state(); assert.equal(s.rendered.camera.zoom,zoom);
    check('focused_keyboard_move_resize_and_noop_'+zoom);

    const move=await startGesture('move',1,1);
    assert.equal(move.moving.rendered.placement_preview.invalid,false);
    assert.equal(move.moving.rendered.placement_preview.left,64);
    assert.equal(move.moving.rendered.placement_preview.top,9*64);
    // Extra intermediate moves never advance the Rust revision or accumulate snap.
    await pointer('move',move.end.x-8,move.end.y-8); await pointer('move',move.end.x,move.end.y);
    assert.deepEqual((await state()).layout,move.before);
    await shot('move-preview-'+zoom,await state());
    await pointer('up',move.end.x,move.end.y);
    s=await until(s=>s.layout.revision===(BigInt(move.before.revision)+1n).toString());
    assert.deepEqual(block(s).rect,{x:1,y:9,width:12,height:8});
    assert.deepEqual(block(s,'2'),move.before.blocks[1]); assert.equal(s.rendered.placement_preview,null);
    await shot('moved-'+zoom,s);
    check('header_preview_and_one_move_commit_'+zoom);

    const resize=await startGesture('resize',2,1);
    assert.equal(resize.moving.rendered.placement_preview.invalid,false);
    assert.equal(resize.moving.rendered.placement_preview.width,14*64);
    assert.equal(resize.moving.rendered.placement_preview.height,9*64);
    await shot('resize-preview-'+zoom,resize.moving);
    await pointer('up',resize.end.x,resize.end.y);
    s=await until(s=>s.layout.revision===(BigInt(resize.before.revision)+1n).toString());
    assert.deepEqual(block(s).rect,{x:1,y:9,width:14,height:9});
    assert.deepEqual(block(s,'2'),resize.before.blocks[1]);
    await shot('resized-'+zoom,s);
    check('bottom_right_preview_and_one_resize_commit_'+zoom);

    for(const cancellation of ['escape','cancel','lost']) {
      const gesture=await startGesture('move',1,1);
      if(cancellation==='escape') await key('escape');
      else assert.deepEqual(await app.request({cancel_pan:{lost_capture:cancellation==='lost'}}),{result:{synthetic:true}});
      s=await state(); assert.deepEqual(s.layout,gesture.before); assert.equal(s.rendered.placement_preview,null);
      await pointer('up',gesture.end.x,gesture.end.y);
      assert.deepEqual((await state()).layout,gesture.before);
    }
    check('escape_and_synthetic_cancel_lost_capture_no_commit_'+zoom);

    if(zoom===100) {
      const gesture=await startGesture('move',1,1);
      assert.deepEqual(await app.request({resize:{width:640,height:480}}),{result:{resized:true}});
      s=await until(s=>s.rendered.viewport.width===640);
      assert.deepEqual(s.layout,gesture.before); assert.equal(s.rendered.placement_preview,null);
      await pointer('up',gesture.end.x,gesture.end.y);
      assert.deepEqual(await app.request({resize:{width:1024,height:700}}),{result:{resized:true}});
      await until(s=>s.rendered.viewport.width===1024);
      check('native_window_resize_cancels_active_preview');
    }

    if(zoom===50) {
      for(const [name,dx,dy,target,reason] of [
        ['collision',11,-9,{x:150,y:350},/overlap/i],['negative',-2,0,{x:240,y:160},/outside/i],
      ]) {
        const gesture=await startGesture('move',dx,dy,target);
        assert.equal(gesture.moving.rendered.placement_preview.invalid,true);
        assert.match(gesture.moving.rendered.placement_preview.text,reason);
        await shot('invalid-'+name,gesture.moving);
        await pointer('up',gesture.end.x,gesture.end.y);
        s=await state(); assert.deepEqual(s.layout,gesture.before); assert.match(s.rendered.error,reason);
      }
      const shrink=await startGesture('resize',-13,-8,{x:700,y:400});
      assert.equal(shrink.moving.rendered.placement_preview.invalid,false);
      await pointer('up',shrink.end.x,shrink.end.y);
      s=await until(s=>s.layout.revision===(BigInt(shrink.before.revision)+1n).toString());
      assert.deepEqual(block(s).rect,{x:1,y:9,width:1,height:1});
      const minimum=await startGesture('resize',-1,0);
      assert.equal(minimum.moving.rendered.placement_preview.invalid,true);
      await pointer('up',minimum.end.x,minimum.end.y);
      s=await state(); assert.deepEqual(s.layout,minimum.before); assert.match(s.rendered.error,/at least 1/);
      await key('resize_left'); s=await state(); assert.deepEqual(s.layout,minimum.before);
      await click(s.rendered.controls.title); await key('arrow_right');
      s=await state(); assert.deepEqual(s.layout,minimum.before,'Title input arrows must not edit layout');
      await click(s.rendered.controls.pan);
      for(let attempt=0;attempt<12;attempt++) {s=await state(); if(s.rendered.active.block_id==='1') break; await key('tab');}
      assert.equal(s.rendered.active.block_id,'1'); assert.equal(s.rendered.pan_tool,true);
      await key('arrow_right'); assert.deepEqual((await state()).layout,minimum.before,'Pan tool must disable layout keyboard actions');
      check('local_invalid_collision_negative_minimum_input_focus_and_pan_tool_guards');
    }
    await close(); check('clean_close_'+zoom);
  }

  app=await launchAutomation(binary);
  let s=await state(); await click(node(s).header); s=await state();
  const before=s.layout;
  const header=node(s).header, start={x:40,y:Math.round(header.y+header.height/2)};
  await pointer('down',start.x,start.y); await pointer('move',start.x,start.y+128);
  s=await state(); assert.deepEqual(s.layout,before); assert.equal(s.rendered.placement_preview.invalid,false);
  const advanced=await app.request('advance_layout'); assert.equal(advanced.result?.status,'ok');
  const stale=await app.request('inspect');
  assert.equal(stale.result.layout.revision,'1'); assert.equal(stale.result.rendered.rendered_revision,'0');
  await pointer('up',start.x,start.y+128);
  s=await until(s=>s.rendered.error.includes('workspace changed'));
  assert.deepEqual(s.layout,advanced.result.snapshot); assert.deepEqual(block(s),before.blocks[0]);
  assert.equal(s.rendered.placement_preview,null);
  await shot('stale-rejected',s);
  check('competing_real_ipc_rejects_stale_gesture_and_reconciles_current_snapshot');
  await close(); check('clean_close_stale');
  evidence.result='PASS';
} catch(error) {
  evidence.result='FAIL'; evidence.error=String(error); evidence.stack=error.stack; process.exitCode=1;
  if(app) try {await writeFile(resolve(output,'failure.json'),JSON.stringify(await app.request('inspect'),null,2));} catch {}
} finally {
  if(app?.child.exitCode===null&&app?.child.signalCode===null) await app.stop();
  evidence.finished=new Date().toISOString();
  await writeFile(resolve(output,'result.json'),JSON.stringify(evidence,null,2));
  console.log(JSON.stringify(evidence,null,2));
}
