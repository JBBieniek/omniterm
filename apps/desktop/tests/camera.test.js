import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import { ZOOM_LEVELS, worldToScreen, screenToWorld, viewportPoint, zoomAt, panScreen, stepZoom, zoomShortcut } from '../src/camera.ts';

const cases = JSON.parse(await readFile(new URL('./camera-cases.json', import.meta.url), 'utf8'));
const point = ([x, y]) => ({x, y});
const near = (actual, expected) => {
  assert(Math.abs(actual.x - expected.x) < 1e-9, `${actual.x} != ${expected.x}`);
  assert(Math.abs(actual.y - expected.y) < 1e-9, `${actual.y} != ${expected.y}`);
};

test('same transform and anchored zoom fixtures as Rust', () => {
  for (const c of cases.world_to_screen) {
    const camera = {origin:point(c.origin), zoom:c.zoom};
    near(worldToScreen(camera, point(c.world)), point(c.screen));
    near(screenToWorld(camera, point(c.screen)), point(c.world));
  }
  for (const c of cases.zoom_at) {
    near(zoomAt({origin:point(c.origin), zoom:c.old_zoom}, c.new_zoom, point(c.anchor)).origin, point(c.new_origin));
  }
});

test('all levels round trip and keep viewport center anchored without changing inputs', () => {
  assert.deepEqual(ZOOM_LEVELS, [25,33,50,67,100,125,150,175,200]);
  for (const zoom of ZOOM_LEVELS) {
    const camera = {origin:{x:-123.25,y:89.75}, zoom};
    const before = structuredClone(camera);
    for (const world of [{x:0,y:0}, {x:3072,y:2048}, {x:-64.5,y:100.25}]) {
      near(screenToWorld(camera, worldToScreen(camera, world)), world);
    }
    for (const next of ZOOM_LEVELS) {
      const anchor = {x:491.5,y:277.25};
      near(worldToScreen(zoomAt(camera, next, anchor), screenToWorld(camera, anchor)), anchor);
    }
    assert.deepEqual(camera, before);
  }
  assert.equal(stepZoom(25, -1), 25);
  assert.equal(stepZoom(200, 1), 200);
  assert.equal(stepZoom(100, -1), 67);
});

test('viewport offset is CSS-local and pan uses inverse scale', () => {
  near(viewportPoint({x:420,y:280}, {left:20,top:130}), {x:400,y:150});
  for (const zoom of ZOOM_LEVELS) {
    const camera = {origin:{x:-20,y:50},zoom};
    const panned = panScreen(camera, 80, -40);
    near(worldToScreen(panned, camera.origin), {x:-80,y:40});
  }
  assert.throws(() => panScreen({origin:{x:0,y:0},zoom:100}, Infinity, 0), RangeError);
  assert.throws(() => screenToWorld({origin:{x:0,y:0},zoom:100}, {x:NaN,y:0}), RangeError);
});

test('only app Ctrl zoom shortcuts, with composition and modal guards', () => {
  const key = (key, code = '', extra = {}) => ({key,code,ctrlKey:true,altKey:false,metaKey:false,isComposing:false,keyCode:0,...extra});
  for (const event of [key('+'),key('='),key('', 'Equal'),key('Add','NumpadAdd')]) assert.equal(zoomShortcut(event, false), 1);
  for (const event of [key('-'),key('', 'Minus'),key('Subtract','NumpadSubtract')]) assert.equal(zoomShortcut(event, false), -1);
  assert.equal(zoomShortcut(key('0', 'Digit0'), false), 0);
  for (const event of [key('c'),key('+','',{ctrlKey:false}),key('+','',{altKey:true}),key('+','',{metaKey:true}),key('+','',{isComposing:true}),key('+','',{keyCode:229})]) assert.equal(zoomShortcut(event, false), null);
  assert.equal(zoomShortcut(key('+'), true), null);
});
