export const TERMINAL_LINE_HEIGHT = 22;

// One screen/status read at a time; keep one pending refresh across awaits.
export function terminalRefresh(read: () => Promise<void>, failed: (error: unknown) => void) {
  let stopped = false, busy = false, queued = false, pending = false;
  async function run() {
    queued = false;
    if (stopped || busy || !pending) return;
    pending = false;
    busy = true;
    try { await read(); } catch (error) { failed(error); }
    finally { busy = false; if (pending) request(); }
  }
  function request() {
    if (stopped) return;
    pending = true;
    if (!busy && !queued) { queued = true; queueMicrotask(() => { void run(); }); }
  }
  return {request, dispose() { stopped = true; pending = false; }};
}

export function fitTerminal(width: number, height: number, cellWidth: number) {
  if (![width,height,cellWidth].every(Number.isFinite) || width < 0 || height < 0 || cellWidth <= 0) {
    throw new RangeError('Invalid terminal dimensions');
  }
  return {cols:Math.max(10,Math.min(120,Math.floor(width/cellWidth))),
    rows:Math.max(2,Math.min(60,Math.floor(height/TERMINAL_LINE_HEIGHT)))};
}

export function historyOffset(current: number, delta: number, available: number) {
  if (![current,delta,available].every(Number.isFinite)) throw new RangeError('Invalid history offset');
  return Math.max(0,Math.min(200,Math.max(0,Math.trunc(available)),Math.trunc(current+delta)));
}

export function historyKey(key: string, rows: number, offset: number, available: number): number | null {
  const page = Math.max(1,rows-1);
  switch(key) {
    case 'PageUp': return historyOffset(offset,page,available);
    case 'PageDown': return historyOffset(offset,-page,available);
    case 'Home': return historyOffset(0,available,available);
    case 'End': return 0;
    default: return null;
  }
}

export function screenText(lines: {text: string}[][]) {
  return lines.map(line=>line.map(run=>run.text).join('').replace(/ +$/u,'')).join('\n');
}
