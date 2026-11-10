<script lang="ts">
  import { onMount, tick } from 'svelte';
  import { zoomShortcut } from './camera';
  import { fitTerminal, terminalCommand } from './terminal';
  import type { Screen } from './terminal';
  import {historyKey, historyOffset, screenText, TERMINAL_LINE_HEIGHT} from './terminal-view';

  let { sessionId, parentSessionId, onparent, onstate }: {
    sessionId: string; parentSessionId: string | null; onparent: (id: string) => void; onstate: () => void
  } = $props();
  let screen = $state<Screen | null>(null);
  let error = $state('');
  let cellWidth = $state(8.5);
  let surface: HTMLDivElement;
  let measure: HTMLSpanElement;
  let input = $state<HTMLTextAreaElement>();
  let readingSurface = $state<HTMLDivElement>();
  let reading = $state(false);
  let focused = $state(false);
  let offset = $state(0);
  let query = $state(0);
  let applied = $state(-1);
  let sessionState = $state('connecting');
  let exitCode = $state<number | null>(null);
  let focusNext = false;
  let refresh = () => {};
  let composing = $state(false);
  let disposed = false;
  let chain = Promise.resolve();
  let queued = 0;

  function readPage(next = 0) {
    if (composing) return;
    reading = true;
    focused = false;
    offset = historyOffset(next,0,screen?.scrollback_rows ?? 0);
    query++;
    focusNext = true;
    refresh();
  }
  function liveInput() {
    reading = false;
    offset = 0;
    query++;
    focusNext = true;
    refresh();
  }
  function readingKey(event: KeyboardEvent) {
    if (event.isComposing) return;
    if (event.key === 'Escape') { event.preventDefault(); event.stopPropagation(); liveInput(); return; }
    if (event.ctrlKey && event.key.toLowerCase() === 'a') {
      event.preventDefault();
      if (readingSurface) window.getSelection()?.selectAllChildren(readingSurface);
      return;
    }
    if (event.ctrlKey || event.altKey || event.metaKey) return;
    const next = historyKey(event.key,screen?.rows ?? 2,offset,screen?.scrollback_rows ?? 0);
    if (next !== null) { event.preventDefault(); event.stopPropagation(); readPage(next); }
  }
  function wheel(event: WheelEvent) {
    if (!reading || event.ctrlKey || !event.deltaY) return;
    event.preventDefault();
    readPage(historyOffset(offset,event.deltaY < 0 ? 3 : -3,screen?.scrollback_rows ?? 0));
  }

  function send(command: Record<string, unknown>) {
    if (queued >= 32) { error = 'Input queue full; this input was not sent.'; return; }
    queued++;
    chain = chain.then(async () => {
      const reply = await terminalCommand({...command, session_id: sessionId});
      if (reply.status === 'error') error = reply.error.message;
    }).catch(() => { error = 'Input delivery could not be confirmed; it was not retried.'; })
      .finally(() => { queued--; });
  }
  function flushText() {
    if (reading || composing || !input?.value) return;
    const text = input.value;
    input.value = '';
    if (new TextEncoder().encode(text).length > 4096) { error = 'Input exceeds 4096 UTF-8 bytes.'; return; }
    send({op: 'text', text, paste: false});
  }
  function keyDown(event: KeyboardEvent) {
    if (event.isComposing || composing || event.keyCode === 229) return;
    if (zoomShortcut(event, false) !== null) return;
    event.stopPropagation();
    if (event.shiftKey && ['PageUp','PageDown'].includes(event.key)) {
      event.preventDefault();
      readPage(historyKey(event.key,screen?.rows ?? 2,0,screen?.scrollback_rows ?? 0) ?? 0);
      return;
    }
    if (event.key === 'Tab' && event.shiftKey) return; // Keyboard route back to workspace chrome.
    if (event.ctrlKey && ['v', 'V'].includes(event.key)) return; // Native paste event.
    const special = ['Enter', 'Backspace', 'Tab', 'Escape', 'Delete', 'Home', 'End', 'PageUp', 'PageDown',
      'ArrowUp', 'ArrowDown', 'ArrowLeft', 'ArrowRight'].includes(event.key);
    if (special || ((event.ctrlKey || event.altKey) && event.key.length === 1)) {
      event.preventDefault();
      flushText();
      send({op: 'key', key: event.key, ctrl: event.ctrlKey, alt: event.altKey, shift: event.shiftKey});
    }
  }
  function paste(event: ClipboardEvent) {
    event.preventDefault();
    const text = event.clipboardData?.getData('text/plain') ?? '';
    if (!text) return;
    if (new TextEncoder().encode(text).length > 4096) { error = 'Paste exceeds 4096 UTF-8 bytes; nothing sent.'; return; }
    send({op: 'text', text, paste: true});
  }

  onMount(() => {
    let timer: ReturnType<typeof setTimeout>;
    let fitTimer: ReturnType<typeof setTimeout>;
    let lastSize = '';
    let lastState = '';
    let fitting = false;
    let polling = false;
    async function fit() {
      if (disposed || fitting || !surface.clientWidth || !surface.clientHeight || sessionState !== 'running') return;
      cellWidth = measure.offsetWidth / 10;
      const size = fitTerminal(surface.clientWidth, surface.clientHeight, cellWidth);
      const key = `${size.rows}:${size.cols}`;
      if (key === lastSize) return;
      fitting = true;
      try {
        const reply = await terminalCommand({op: 'resize', session_id: sessionId, ...size});
        if (reply.status === 'error') error = reply.error.message;
        else lastSize = key;
      } catch { error = 'Terminal resize could not be confirmed.'; }
      finally { fitting = false; }
    }
    async function poll() {
      if (disposed || polling) return;
      polling = true;
      const requested = query;
      let failed = false;
      try {
        const metadataOnly = reading && applied === requested;
        const reply = await terminalCommand(metadataOnly ? {op:'list'} : {op:'screen',session_id:sessionId,scrollback:reading ? offset : 0,
          after:reading || applied !== requested ? null : screen?.revision ?? null});
        if (disposed || query !== requested) return;
        if (reply.status === 'error') { failed = true; error = reply.error.message; return; }
        applied = requested;
        if (reply.screen) {
          screen = reply.screen;
          offset = screen.scrollback_offset;
          sessionState = screen.state;
          exitCode = screen.exit_code;
          void fit();
        }
        if (metadataOnly) {
          const session = reply.sessions?.find(s => s.session_id === sessionId);
          if (session) { sessionState = session.state; exitCode = session.exit_code; }
        }
        if (lastState !== sessionState) { lastState = sessionState; onstate(); }
        if (focusNext) {
          focusNext = false;
          await tick();
          if (query === requested) (reading ? readingSurface : input)?.focus({preventScroll:true});
        }
      } catch { failed = true; if (!disposed && query === requested) error = 'Terminal view disconnected; reload the workspace to reconnect.'; }
      finally {
        polling = false;
        if (!disposed && query !== requested) timer = setTimeout(poll,0);
        else if (!disposed && !failed && reading && !['exited','failed'].includes(sessionState)) timer = setTimeout(poll,500);
        else if (!disposed && !failed && !reading && !(screen?.output_eof && ['exited','failed'].includes(screen.state))) timer = setTimeout(poll,125);
      }
    }
    refresh = () => { clearTimeout(timer); void poll(); };
    const observer = new ResizeObserver(() => { clearTimeout(fitTimer); fitTimer = setTimeout(fit, 100); });
    observer.observe(surface);
    surface.addEventListener('wheel',wheel,{passive:false});
    void poll();
    return () => { disposed = true; refresh = () => {}; clearTimeout(timer); clearTimeout(fitTimer); observer.disconnect(); surface.removeEventListener('wheel',wheel); };
  });
</script>

<div class="terminal" data-session-id={sessionId} data-terminal-state={sessionState}
  data-parent-session-id={parentSessionId ?? ''}
  data-terminal-rows={screen?.rows ?? ''} data-terminal-cols={screen?.cols ?? ''}
  data-terminal-reading={reading} data-terminal-focused={focused} data-terminal-view-ready={applied === query}
  data-scrollback-offset={screen?.scrollback_offset ?? 0} data-scrollback-rows={screen?.scrollback_rows ?? 0}
  style:--terminal-row-height={`${TERMINAL_LINE_HEIGHT}px`}>
  <div class="terminal-tools" aria-label="Terminal view controls">
    {#if parentSessionId}<button type="button" data-testid="parent-terminal" aria-label={`Show parent terminal ${parentSessionId}`} onclick={() => parentSessionId && onparent(parentSessionId)}>↖ #{parentSessionId}</button>{/if}
    <span class="terminal-mode"><span class:active={focused && !reading} class="terminal-dot"></span>{reading ? 'READING' : 'TERMINAL'}</span>
    <button type="button" data-testid="read-terminal" aria-pressed={reading} onclick={() => readPage()} disabled={!screen || composing}>Read / select</button>
    <button type="button" data-testid="older-terminal" aria-label="Older terminal output" disabled={!reading || offset >= (screen?.scrollback_rows ?? 0)} onclick={() => readPage(historyOffset(offset,(screen?.rows ?? 2)-1,screen?.scrollback_rows ?? 0))}>↑ Older</button>
    <button type="button" data-testid="newer-terminal" aria-label="Newer terminal output" disabled={!reading || offset === 0} onclick={() => readPage(historyOffset(offset,-((screen?.rows ?? 2)-1),screen?.scrollback_rows ?? 0))}>↓ Newer</button>
    <button type="button" data-testid="live-terminal" disabled={!reading} onclick={liveInput}>Live input ↵</button>
  </div>
  <div class="terminal-surface" bind:this={surface}>
    <span class="terminal-measure" bind:this={measure} aria-hidden="true">0000000000</span>
    {#if reading}
    <div class="terminal-reading" data-testid="terminal-reading" role="textbox" aria-readonly="true" aria-multiline="true" aria-label={`Terminal ${sessionId} captured output`}
      bind:this={readingSurface} tabindex="0" onkeydown={readingKey} aria-busy={applied !== query}>{screenText(screen?.lines ?? [])}</div>
    {:else}
    <div class="terminal-screen" aria-hidden="true" data-testid="terminal-screen">
      {#each screen?.lines ?? [] as line, row}
        <div class="terminal-row" style:top={`${row * TERMINAL_LINE_HEIGHT}px`}>
          {#each line as run}
            <span style:left={`${run.column * cellWidth}px`} style:width={`${run.width * cellWidth}px`}
              style:color={run.fg} style:background={run.bg} style:font-weight={run.bold ? 'bold' : 'normal'}
              style:font-style={run.italic ? 'italic' : 'normal'} style:opacity={run.dim ? '.6' : '1'}
              style:text-decoration={run.underline ? 'underline' : 'none'}>{run.text}</span>
          {/each}
        </div>
      {/each}
      {#if screen?.cursor_visible && screen.state === 'running'}
        <span class="terminal-cursor" style:left={`${screen.cursor_col * cellWidth}px`}
          style:top={`${screen.cursor_row * TERMINAL_LINE_HEIGHT}px`} style:width={`${cellWidth}px`}></span>
      {/if}
    </div>
    <textarea bind:this={input} aria-label={`Terminal ${sessionId} input`} aria-describedby={`terminal-help-${sessionId}`}
      spellcheck="false" autocomplete="off" autocapitalize="off" disabled={screen?.state !== 'running' || applied !== query}
      onfocus={() => focused = true} onblur={() => focused = false}
      onkeydown={keyDown} oninput={flushText} onpaste={paste}
      oncompositionstart={() => { composing = true; }}
      oncompositionend={() => { composing = false; queueMicrotask(flushText); }}></textarea>
    {/if}
  </div>
  <div class="terminal-status" role="status" data-testid="terminal-status">
    <span>#{sessionId} · {reading ? 'Captured page · ' : ''}{sessionState}{exitCode != null ? ` (${exitCode})` : ''}
    · {screen?.cols ?? '…'}×{screen?.rows ?? '…'}{reading ? ` · ${offset}/${screen?.scrollback_rows ?? 0} rows back` : focused ? ' · Input active' : ' · Click to type'}</span>
    <span class="terminal-hint">{reading ? 'Drag to select · Ctrl+C to copy · Esc to type' : 'Shift+PgUp to read · Ctrl+C to interrupt'}</span>
    {#if screen?.evicted_bytes}<span class="terminal-gap" title={`${screen.evicted_bytes} raw bytes evicted; recent history is limited to 200 rows`}>Older output discarded</span>{/if}
    {#if screen?.read_failed || screen?.reply_dropped} · I/O incomplete{/if}
  </div>
  <span id={`terminal-help-${sessionId}`} class="sr-only">Type to send input; Ctrl+C interrupts; Shift+Tab leaves the terminal.
    Output may include echoed input. Input recording is off.</span>
  {#if !reading}<pre class="sr-only" aria-label={`Terminal ${sessionId} current output`}>{screenText(screen?.lines ?? [])}</pre>{/if}
  {#if error}<div class="terminal-error" role="alert">{error}</div>{/if}
</div>
