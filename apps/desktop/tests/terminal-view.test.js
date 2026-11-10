import assert from 'node:assert/strict';
import test from 'node:test';
import {fitTerminal, historyOffset, historyKey, screenText, terminalRefresh, TERMINAL_LINE_HEIGHT} from '../src/terminal-view.ts';

test('terminal fit uses the same row height and respects host limits', () => {
  assert.equal(TERMINAL_LINE_HEIGHT,22);
  assert.deepEqual(fitTerminal(720,440,9),{cols:80,rows:20});
  assert.deepEqual(fitTerminal(0,0,9),{cols:10,rows:2});
  assert.deepEqual(fitTerminal(9000,9000,9),{cols:120,rows:60});
  for(const [w,h,c] of [[NaN,20,9],[20,Infinity,9],[20,20,0],[-1,20,9]]) assert.throws(()=>fitTerminal(w,h,c),RangeError);
});

test('history navigation stays within retained rows and the Rust limit', () => {
  assert.equal(historyOffset(0,3,12),3);
  assert.equal(historyOffset(8,20,12),12);
  assert.equal(historyOffset(8,-20,12),0);
  assert.equal(historyOffset(198,20,400),200);
  assert.equal(historyOffset(10,0,0),0);
  assert.throws(()=>historyOffset(0,NaN,200),RangeError);
});

test('reading keys page history without producing terminal keys', () => {
  assert.equal(historyKey('PageUp',20,0,200),19);
  assert.equal(historyKey('PageDown',20,19,200),0);
  assert.equal(historyKey('Home',20,19,200),200);
  assert.equal(historyKey('End',20,19,200),0);
  for(const key of ['c','Enter','ArrowUp','Escape']) assert.equal(historyKey(key,20,19,200),null);
});

test('selectable reading text preserves Unicode and line breaks and strips only empty trailing cells', () => {
  assert.equal(screenText([[{text:'é界é '},{text:'literal <img src=x>  '}],[{text:'  indented\u00a0 '}],[]]),'é界é literal <img src=x>\n  indented\u00a0\n');
});

const turn=()=>new Promise(done=>setImmediate(done));
test('terminal refresh coalesces bursts and preserves a change arriving during a read', async()=>{
  const completions=[];let reads=0,active=0,maxActive=0;
  const refresh=terminalRefresh(()=>new Promise(done=>{reads++;active++;maxActive=Math.max(maxActive,active);completions.push(()=>{active--;done();});}),error=>{throw error;});
  for(let i=0;i<100;i++)refresh.request();await turn();assert.equal(reads,1);
  for(let i=0;i<100;i++)refresh.request();assert.equal(reads,1);
  completions.shift()();await turn();assert.equal(reads,2);assert.equal(maxActive,1);
  completions.shift()();await turn();assert.equal(reads,2);refresh.dispose();
});

test('terminal refresh disposal drops queued work and never replays an in-flight request', async()=>{
  let reads=0,finish;const refresh=terminalRefresh(()=>{reads++;return new Promise(done=>{finish=done;});},error=>{throw error;});
  refresh.request();await turn();refresh.request();refresh.dispose();finish();await turn();refresh.request();await turn();assert.equal(reads,1);
  const stopped=terminalRefresh(async()=>{reads++;},error=>{throw error;});stopped.request();stopped.dispose();await turn();assert.equal(reads,1);
});

test('terminal refresh reports a failed read once and waits for another explicit request', async()=>{
  let reads=0,failures=0;const refresh=terminalRefresh(async()=>{reads++;throw Error('transport');},()=>{failures++;});
  refresh.request();await turn();await turn();assert.equal(reads,1);assert.equal(failures,1);
  refresh.request();await turn();assert.equal(reads,2);assert.equal(failures,2);refresh.dispose();
});
