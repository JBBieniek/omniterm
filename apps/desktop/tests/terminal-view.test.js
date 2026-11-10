import assert from 'node:assert/strict';
import test from 'node:test';
import {fitTerminal, historyOffset, historyKey, screenText, TERMINAL_LINE_HEIGHT} from '../src/terminal-view.ts';

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
