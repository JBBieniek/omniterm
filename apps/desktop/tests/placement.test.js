import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import { screenToWorld, ZOOM_LEVELS } from '../src/camera.ts';
import { snapDelta, placementCandidate, placementError, keyboardPlacement, sameRect } from '../src/placement.ts';

const cases = JSON.parse(await readFile(new URL('./camera-cases.json', import.meta.url), 'utf8'));
const grid = {columns:48,rows:32,cell_px:64};

test('negative half-cell ties match Rust rounding fixtures', () => {
  for (const c of cases.snap_delta) assert.equal(snapDelta(c.delta_cells), c.expected);
  assert.equal(snapDelta(-.49), 0);
  assert.throws(() => snapDelta(Infinity), RangeError);
});

test('original world grab drives identical move and resize at every zoom', () => {
  const original = {x:4,y:9,width:12,height:8};
  const before = structuredClone(original);
  for (const zoom of ZOOM_LEVELS) {
    const camera = {origin:{x:-63.25,y:121.5},zoom};
    const start = {x:225.5,y:180.25};
    const grab = screenToWorld(camera, start);
    const current = screenToWorld(camera, {x:start.x + 128 * zoom / 100,y:start.y - 96 * zoom / 100});
    assert.deepEqual(placementCandidate(original,grab,current,64,'move'), {x:6,y:7,width:12,height:8});
    assert.deepEqual(placementCandidate(original,grab,current,64,'resize'), {x:4,y:9,width:14,height:6});
    assert(sameRect(placementCandidate(original,grab,grab,64,'move'),original));
  }
  assert.deepEqual(original,before);
  const grab = {x:0,y:0};
  assert.equal(placementCandidate(original,grab,{x:31.99999,y:0},64,'move').x,4);
  assert.equal(placementCandidate(original,grab,{x:32.00001,y:0},64,'move').x,5);
});

test('preview uses minimum 1x1, finite board and half-open collision bounds', () => {
  const blocks = [{id:'1',rect:{x:0,y:0,width:12,height:8}}, {id:'2',rect:{x:12,y:0,width:8,height:8}}];
  assert.equal(placementError(blocks[0].rect,grid,blocks,'1'), null);
  assert.equal(placementError({x:47,y:31,width:1,height:1},grid,blocks,'1'), null);
  assert.match(placementError({x:1,y:0,width:12,height:8},grid,blocks,'1'), /overlap/i);
  for (const rect of [{x:-1,y:0,width:1,height:1},{x:48,y:0,width:1,height:1}, {x:0,y:32,width:1,height:1}, {x:0,y:0,width:49,height:1}]) {
    assert.match(placementError(rect,grid,blocks,'1'), /outside/i);
  }
  for (const width of [0,-1,1.5,Infinity,NaN]) {
    assert(placementError({x:0,y:0,width,height:1},grid,blocks,'1'));
  }
  assert.match(placementError({x:0,y:0,width:3,height:3},grid,[{...blocks[0],session_id:'42'}],'1'), /4 × 3/);
});

test('keyboard moves one cell, Shift resizes from fixed top-left', () => {
  const rect = {x:3,y:4,width:12,height:8};
  assert.deepEqual(keyboardPlacement(rect,'ArrowLeft',false),{...rect,x:2});
  assert.deepEqual(keyboardPlacement(rect,'ArrowDown',false),{...rect,y:5});
  assert.deepEqual(keyboardPlacement(rect,'ArrowRight',true),{...rect,width:13});
  assert.deepEqual(keyboardPlacement(rect,'ArrowUp',true),{...rect,height:7});
  assert.equal(keyboardPlacement(rect,'c',false),null);
  assert.deepEqual(rect,{x:3,y:4,width:12,height:8});
});
