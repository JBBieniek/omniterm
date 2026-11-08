<script lang="ts">
  import { onMount } from 'svelte';
  import { zoomShortcut } from './camera';
  import { fitTerminal, terminalCommand } from './terminal';
  import type { Screen } from './terminal';

  let { sessionId, onstate }: {sessionId: string; onstate: () => void} = $props();
  let screen = $state<Screen | null>(null);
  let error = $state('');
  let cellWidth = $state(8.5);
  let surface: HTMLDivElement;
  let measure: HTMLSpanElement;
  let input: HTMLTextAreaElement;
  let composing = false;
  let disposed = false;
  let chain = Promise.resolve();
  let queued = 0;

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
    if (composing || !input.value) return;
    const text = input.value;
    input.value = '';
    if (new TextEncoder().encode(text).length > 4096) { error = 'Input exceeds 4096 UTF-8 bytes.'; return; }
    send({op: 'text', text, paste: false});
  }
  function keyDown(event: KeyboardEvent) {
    if (event.isComposing || composing || event.keyCode === 229) return;
    if (zoomShortcut(event, false) !== null) return;
    event.stopPropagation();
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
    async function fit() {
      if (disposed || fitting || !surface.clientWidth || !surface.clientHeight || screen?.state !== 'running') return;
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
      try {
        const reply = await terminalCommand({op: 'screen', session_id: sessionId, after: screen?.revision ?? null});
        if (disposed) return;
        if (reply.status === 'error') { error = reply.error.message; return; }
        if (reply.screen) {
          screen = reply.screen;
          if (lastState !== screen.state) { lastState = screen.state; onstate(); }
          void fit();
        }
      } catch { error = 'Terminal view disconnected; reload the workspace to reconnect.'; return; }
      if (!disposed && !(screen?.output_eof && ['exited', 'failed'].includes(screen.state))) timer = setTimeout(poll, 125);
    }
    const observer = new ResizeObserver(() => { clearTimeout(fitTimer); fitTimer = setTimeout(fit, 100); });
    observer.observe(surface);
    void poll();
    return () => { disposed = true; clearTimeout(timer); clearTimeout(fitTimer); observer.disconnect(); };
  });
</script>

<div class="terminal" data-session-id={sessionId} data-terminal-state={screen?.state ?? 'loading'}
  data-terminal-rows={screen?.rows ?? ''} data-terminal-cols={screen?.cols ?? ''}>
  <div class="terminal-surface" bind:this={surface}>
    <span class="terminal-measure" bind:this={measure} aria-hidden="true">0000000000</span>
    <div class="terminal-screen" aria-hidden="true" data-testid="terminal-screen">
      {#each screen?.lines ?? [] as line, row}
        <div class="terminal-row" style:top={`${row * 20}px`}>
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
          style:top={`${screen.cursor_row * 20}px`} style:width={`${cellWidth}px`}></span>
      {/if}
    </div>
    <textarea bind:this={input} aria-label={`Terminal ${sessionId} input`} aria-describedby={`terminal-help-${sessionId}`}
      spellcheck="false" autocomplete="off" autocapitalize="off" disabled={screen?.state !== 'running'}
      onkeydown={keyDown} oninput={flushText} onpaste={paste}
      oncompositionstart={() => { composing = true; }}
      oncompositionend={() => { composing = false; queueMicrotask(flushText); }}></textarea>
  </div>
  <div class="terminal-status" role="status" data-testid="terminal-status">
    #{sessionId} · {screen?.state ?? 'connecting'}{screen?.exit_code != null ? ` (${screen.exit_code})` : ''}
    · {screen?.cols ?? '…'}×{screen?.rows ?? '…'} · {screen?.raw_bytes ?? 0} B retained
    {#if screen?.evicted_bytes} · {screen.evicted_bytes} B evicted{/if}
    {#if screen?.read_failed || screen?.reply_dropped} · I/O incomplete{/if}
  </div>
  <span id={`terminal-help-${sessionId}`} class="sr-only">Type to send input; Ctrl+C interrupts; Shift+Tab leaves the terminal.
    Output may include echoed input. Input recording is off.</span>
  <pre class="sr-only" aria-label={`Terminal ${sessionId} current output`}>{screen?.lines.map(line => line.map(run => run.text).join('')).join('\n') ?? ''}</pre>
  {#if error}<div class="terminal-error" role="alert">{error}</div>{/if}
</div>
