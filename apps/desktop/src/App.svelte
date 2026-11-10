<script lang="ts">
  import { onMount, tick } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import { listen } from '@tauri-apps/api/event';
  import TerminalView from './TerminalView.svelte';
  import TextView from './TextView.svelte';
  import GitView from './GitView.svelte';
  import { gitCommand } from './git';
  import type { GitSummary } from './git';
  import { terminalCommand } from './terminal';
  import type { Session } from './terminal';
  import type { Layout as LayoutSnapshotV1 } from './workspace';
  import { panScreen, screenToWorld, stepZoom, viewportPoint, zoomAt, zoomShortcut } from './camera';
  import type { Camera, Point } from './camera';
  import { keyboardPlacement, placementCandidate, placementError, sameRect } from './placement';
  import type { Rect } from './placement';

  type MutationResult =
    | { status: 'ok'; snapshot: LayoutSnapshotV1; block_id?: string }
    | { status: 'error'; snapshot: LayoutSnapshotV1; error: { code: string; message: string } };

  let snapshot = $state<LayoutSnapshotV1 | null>(null);
  let selectedId = $state<string | null>(null);
  let title = $state('Placeholder');
  let loading = $state(true);
  let pending = $state(false);
  let needsReload = $state(false);
  let error = $state('');
  let addButton: HTMLButtonElement;
  let sessions = $state<Session[]>([]);
  let sessionId = $state('');
  let quitDialog: HTMLDialogElement;
  let openedId = $state<string | null>(null);
  let gitRuns = $state<GitSummary[]>([]);
  let viewport: HTMLElement;
  let camera = $state<Camera>({origin: {x: 0, y: 0}, zoom: 100});
  let panTool = $state(false);
  let pan = $state<{pointerId: number; start: Point; camera: Camera} | null>(null);
  let placement = $state<{
    pointerId: number; blockId: string; original: Rect; candidate: Rect; grab: Point;
    camera: Camera; bounds: {left: number; top: number}; cellPx: number;
    revision: string; kind: 'move' | 'resize';
  } | null>(null);
  let composing = false;
  const unavailable = $derived(loading || pending || needsReload || !snapshot || !!placement);
  const previewError = $derived(placement && snapshot
    ? placementError(placement.candidate, snapshot.grid, snapshot.blocks, placement.blockId) : null);

  function cancelPlacement() {
    if (!placement) return;
    const pointerId = placement.pointerId;
    placement = null;
    if (viewport.hasPointerCapture(pointerId)) viewport.releasePointerCapture(pointerId);
  }

  function cancelGestures() {
    finishPan(true);
    cancelPlacement();
  }

  function finishPan(cancel = false) {
    if (!pan) return;
    const previous = pan;
    pan = null;
    if (cancel) camera = previous.camera;
    if (viewport.hasPointerCapture(previous.pointerId)) viewport.releasePointerCapture(previous.pointerId);
  }

  function pointerPoint(event: PointerEvent) {
    return viewportPoint({x: event.clientX, y: event.clientY}, viewport.getBoundingClientRect());
  }

  function startPan(event: PointerEvent) {
    if (!snapshot || pan || !event.isPrimary) return;
    const onBlock = event.target instanceof Element && event.target.closest('[data-block-id]');
    if (!((event.button === 0 && panTool) || (event.button === 1 && !onBlock))) return;
    event.preventDefault();
    viewport.setPointerCapture(event.pointerId);
    pan = {pointerId: event.pointerId, start: pointerPoint(event), camera};
  }

  function startPointer(event: PointerEvent) {
    if (pan || placement) return;
    if (panTool || event.button === 1) { startPan(event); return; }
    if (unavailable || !snapshot || event.button !== 0 || !event.isPrimary || !(event.target instanceof Element)) return;
    const handle = event.target.closest<HTMLElement>('[data-resize-block], [data-drag-block]');
    const id = handle?.dataset.resizeBlock ?? handle?.dataset.dragBlock;
    const block = snapshot.blocks.find(block => block.id === id);
    if (!block) return;
    event.preventDefault();
    selectBlock(block.id);
    event.target.closest('button')?.focus({preventScroll: true});
    const bounds = viewport.getBoundingClientRect();
    viewport.setPointerCapture(event.pointerId);
    placement = {pointerId: event.pointerId, blockId: block.id, original: block.rect, candidate: block.rect,
      grab: screenToWorld(camera, viewportPoint({x: event.clientX, y: event.clientY}, bounds)), camera,
      bounds: {left: bounds.left, top: bounds.top}, cellPx: snapshot.grid.cell_px,
      revision: snapshot.revision, kind: handle?.dataset.resizeBlock ? 'resize' : 'move'};
  }

  function movePan(event: PointerEvent) {
    if (pan?.pointerId !== event.pointerId) return;
    const point = pointerPoint(event);
    camera = panScreen(pan.camera, pan.start.x - point.x, pan.start.y - point.y);
  }

  function movePointer(event: PointerEvent) {
    movePan(event);
    if (placement?.pointerId !== event.pointerId) return;
    const current = screenToWorld(placement.camera,
      viewportPoint({x: event.clientX, y: event.clientY}, placement.bounds));
    placement.candidate = placementCandidate(placement.original, placement.grab, current, placement.cellPx, placement.kind);
  }

  async function submitPlacement(blockId: string, rect: Rect, revision: string) {
    if (!snapshot) return;
    const original = snapshot.blocks.find(block => block.id === blockId)?.rect;
    if (!original || sameRect(rect, original)) return;
    const reason = placementError(rect, snapshot.grid, snapshot.blocks, blockId);
    if (reason) { error = `${reason} Placement unchanged.`; return; }
    await mutate('place_block', {blockId, rect, revision});
  }

  function finishPointer(event: PointerEvent) {
    if (pan?.pointerId === event.pointerId) { finishPan(); return; }
    if (placement?.pointerId !== event.pointerId) return;
    movePointer(event);
    const finished = placement;
    cancelPlacement();
    void submitPlacement(finished.blockId, finished.candidate, finished.revision);
  }

  function changeZoom(direction: -1 | 0 | 1) {
    if (!snapshot) return;
    cancelGestures();
    camera = zoomAt(camera, direction === 0 ? 100 : stepZoom(camera.zoom, direction),
      {x: viewport.clientWidth / 2, y: viewport.clientHeight / 2});
  }

  function resetView() {
    cancelGestures();
    camera = {origin: {x: 0, y: 0}, zoom: 100};
  }

  function keyDown(event: KeyboardEvent) {
    const blocked = composing || !document.hasFocus() || !!document.querySelector('dialog[open], [aria-modal="true"]');
    if (event.defaultPrevented || blocked || event.isComposing || event.keyCode === 229) return;
    if (event.key === 'Escape' && (pan || placement)) {
      event.preventDefault();
      cancelGestures();
      return;
    }
    const direction = zoomShortcut(event, blocked);
    if (direction !== null) {
      event.preventDefault(); // Stop WebView page zoom, including at our endpoints.
      changeZoom(direction);
      return;
    }
    if (panTool || pan || unavailable || !snapshot || event.ctrlKey || event.altKey || event.metaKey) return;
    const handle = event.target instanceof Element ? event.target.closest('.block-select, .resize-handle') : null;
    const frame = handle?.closest<HTMLElement>('[data-block-id]');
    const block = snapshot.blocks.find(block => block.id === frame?.dataset.blockId && block.id === selectedId);
    if (!block) return;
    const rect = keyboardPlacement(block.rect, event.key, event.shiftKey);
    if (!rect) return;
    event.preventDefault();
    void submitPlacement(block.id, rect, snapshot.revision);
  }

  function reconcile(next: LayoutSnapshotV1) {
    if (next.api_version !== 1) throw new Error('Unsupported workspace API version.');
    if (snapshot && BigInt(next.revision) < BigInt(snapshot.revision)) return;
    snapshot = next;
    if (!next.blocks.some(block => block.id === selectedId)) selectedId = null;
  }

  async function loadWorkspace() {
    loading = true;
    error = '';
    try {
      reconcile(await invoke<LayoutSnapshotV1>('get_layout_snapshot'));
      needsReload = false;
    } catch {
      needsReload = true;
      error = 'Could not load the workspace. Please retry or restart Omniterm.';
    } finally {
      loading = false;
    }
  }

  function selectBlock(id: string) {
    if (snapshot?.blocks.some(block => block.id === id)) selectedId = id;
    const session = snapshot?.blocks.find(block => block.id === id)?.session_id;
    if (session) sessionId = session;
  }

  async function mutate(command: 'add_placeholder' | 'remove_placement' | 'place_block',
    target?: {blockId: string; rect: Rect; revision: string}) {
    if (unavailable || !snapshot || (command === 'remove_placement' && !selectedId)) return;
    if (command === 'place_block' && !target) return;
    pending = true;
    error = '';
    let removed = false;
    try {
      const body = {
        api_version: 1,
        expected_revision: target?.revision ?? snapshot.revision,
        ...(command === 'add_placeholder' ? { title: title.trim() }
          : command === 'place_block' && target ? {block_id: target.blockId, rect: target.rect} : { block_id: selectedId }),
      };
      const result = await invoke<MutationResult>(command, new TextEncoder().encode(JSON.stringify(body)));
      reconcile(result.snapshot);
      if (result.status === 'error') error = result.error.message;
      else if (command === 'add_placeholder' && result.block_id) selectBlock(result.block_id);
      else if (command === 'remove_placement') removed = true;
    } catch {
      needsReload = true;
      error = 'Could not confirm this change. Reload the workspace before trying again.';
    } finally {
      pending = false;
    }
    if (removed) {
      await tick();
      addButton?.focus();
    }
  }

  async function refreshSessions() {
    try {
      const result = await terminalCommand({op: 'list'});
      if (result.status === 'ok' && result.sessions) {
        sessions = result.sessions;
        if (!sessions.some(s => s.session_id === sessionId)) sessionId = sessions[0]?.session_id ?? '';
      }
    } catch { error = 'Could not refresh Session status.'; }
  }

  async function openedText(id: string) {
    try {
      reconcile(await invoke<LayoutSnapshotV1>('get_layout_snapshot'));
      if (snapshot?.blocks.some(block => block.id === id && (block.text || block.git_task_id || block.session_id))) openedId = id;
      void refreshSessions();
      void refreshGit();
    } catch { error = 'A file was opened, but the board could not refresh; reload the workspace.'; needsReload = true; }
  }
  function showOpened() {
    const block = snapshot?.blocks.find(block => block.id === openedId);
    if (!block || !snapshot) return;
    cancelGestures();
    selectBlock(block.id);
    camera = {...camera, origin: {x: block.rect.x * snapshot.grid.cell_px, y: block.rect.y * snapshot.grid.cell_px}};
  }

  async function refreshGit() {
    try { gitRuns = (await gitCommand<{tasks: GitSummary[]}>({op: 'list'})).tasks; }
    catch { error = 'Could not inspect retained Git runs.'; }
  }
  async function gitAction(op: 'refresh' | 'reopen', taskId: string) {
    if (!snapshot || pending || needsReload) return;
    pending = true; error = '';
    try {
      const result = await gitCommand<{layout: LayoutSnapshotV1; block_id: string}>({op, task_id: taskId, expected_revision: snapshot.revision});
      reconcile(result.layout); openedId = result.block_id; showOpened(); await refreshGit();
    } catch (reason) {
      needsReload = true;
      error = `Could not confirm the Git action: ${String(reason)} Inspect retained runs and reload before another action; nothing was retried.`;
      void refreshGit();
    }
    finally { pending = false; }
  }

  async function terminalAction(op: 'start' | 'reopen' | 'terminate' | 'quit', target = sessionId) {
    if (pending || !snapshot || (needsReload && op !== 'quit')) return;
    pending = true;
    error = '';
    try {
      const result = await terminalCommand({op, ...(op === 'start' || op === 'reopen'
        ? {expected_revision: snapshot.revision} : {}),
        ...(op === 'reopen' || op === 'terminate' ? {session_id: target} : {})});
      if (result.status === 'error') {
        error = result.error.message;
        // Reconcile a rejected placement without ever retrying a process start.
        if (op === 'start' || op === 'reopen') reconcile(await invoke<LayoutSnapshotV1>('get_layout_snapshot'));
      } else {
        if (result.layout) reconcile(result.layout);
        if (result.sessions) sessions = result.sessions;
        if (result.session_id) sessionId = result.session_id;
        if (result.block_id) {
          selectBlock(result.block_id);
          const block = snapshot.blocks.find(b => b.id === result.block_id);
          if (block) camera = {...camera, origin: {x: block.rect.x * snapshot.grid.cell_px, y: block.rect.y * snapshot.grid.cell_px}};
        }
      }
    } catch {
      needsReload = true;
      error = 'Could not confirm the terminal action; it was not retried. Reload before another action.';
    } finally { pending = false; }
  }

  onMount(() => {
    void loadWorkspace();
    void refreshSessions();
    void refreshGit();
    const listeners = [listen('request-quit', () => { if (!quitDialog.open) quitDialog.showModal(); }),
      listen<string>('quit-failed', event => { error = event.payload; }),
      listen<string>('layout-changed', event => { void openedText(event.payload); })];
    void Promise.all(listeners).catch(() => { error = 'Could not subscribe to app quit confirmation; restart Omniterm.'; needsReload = true; });
    return () => { for (const listener of listeners) void listener.then(unlisten => unlisten(), () => {}); };
  });
</script>

<svelte:window onkeydown={keyDown} onblur={() => { composing = false; cancelGestures(); }} onresize={cancelGestures}
  oncompositionstart={() => { composing = true; }} oncompositionend={() => { composing = false; }} />

<div class="application" data-rendered-revision={snapshot?.revision ?? ''}
  data-selected-block-id={selectedId ?? ''} data-busy={loading || pending}>
  <header class="toolbar">
    <div class="workspace-heading">
      <span class="workspace-mark" aria-hidden="true">▦</span>
      <h1>Workspace</h1>
    </div>
    <div class="view-state" aria-label="View controls">
      <button type="button" data-testid="pan-tool" aria-pressed={panTool}
        title="When off: drag block headers; arrows move, Shift+arrows resize a focused selected block"
        disabled={!snapshot} onclick={() => { cancelGestures(); panTool = !panTool; }}>Pan tool</button>
      <button type="button" data-testid="zoom-out" aria-label="Zoom out" disabled={!snapshot || camera.zoom === 25}
        onclick={() => changeZoom(-1)}>−</button>
      <span class="zoom-label" data-testid="zoom-label" role="status">Zoom {camera.zoom}%</span>
      <button type="button" data-testid="zoom-in" aria-label="Zoom in" disabled={!snapshot || camera.zoom === 200}
        onclick={() => changeZoom(1)}>+</button>
      <button type="button" data-testid="reset-view" disabled={!snapshot} onclick={resetView}>Reset view</button>
    </div>
    <div class="placement-controls" aria-label="Placement controls">
      <label for="placeholder-title">Title</label>
      <input id="placeholder-title" data-testid="placeholder-title" bind:value={title}
        maxlength="256" disabled={unavailable} autocomplete="off" />
      <button type="button" data-testid="new-terminal" disabled={unavailable}
        onclick={() => terminalAction('start')}>New terminal</button>
      <button type="button" data-testid="add-placeholder" bind:this={addButton}
        disabled={unavailable || !title.trim()} onclick={() => mutate('add_placeholder')}>Add placeholder</button>
      <button type="button" data-testid="remove-placement" disabled={unavailable || !selectedId}
        onclick={() => mutate('remove_placement')}>Remove placement</button>
    </div>
    {#if sessions.length}
      <div class="session-controls" aria-label="Session controls">
        <label for="session">Session</label>
        <select id="session" data-testid="session-select" bind:value={sessionId} disabled={unavailable}>
          {#each sessions as session}<option value={session.session_id}>#{session.session_id}{session.parent_session_id ? ` ← #${session.parent_session_id}` : ''} · {session.state}</option>{/each}
        </select>
        <button type="button" data-testid="reopen-terminal" disabled={unavailable || !sessionId}
          onclick={() => terminalAction('reopen')}>Show terminal</button>
        <button type="button" data-testid="terminate-session" disabled={unavailable || !sessionId || sessions.find(s => s.session_id === sessionId)?.state !== 'running'}
          onclick={() => terminalAction('terminate')}>Terminate Session</button>
        <span>Removing a placement keeps its Session running.</span>
      </div>
    {/if}
  </header>

  {#if error && snapshot}
    <div class="error-banner" role="alert" data-testid="command-error">
      <span>{error}</span>
      {#if needsReload}<button type="button" disabled={loading} onclick={loadWorkspace}>Reload workspace</button>{/if}
    </div>
  {/if}

  <main class="board-viewport" class:pan-enabled={panTool} class:panning={!!pan}
    bind:this={viewport} aria-label="Workspace viewport" aria-busy={loading || pending}
    onpointerdown={startPointer} onpointermove={movePointer} onpointerup={finishPointer}
    onpointercancel={cancelGestures} onlostpointercapture={cancelGestures}>
    {#if !snapshot}
      <section class="empty-state">
        {#if loading}<h2>Loading workspace…</h2>
        {:else}<h2>Workspace unavailable</h2><p role="alert">{error}</p>
          <button type="button" onclick={loadWorkspace}>Retry</button>{/if}
      </section>
    {:else}
      <div class="world" data-testid="world" aria-label="Workspace board"
        style:width={`${snapshot.grid.columns * snapshot.grid.cell_px}px`}
        style:height={`${snapshot.grid.rows * snapshot.grid.cell_px}px`}
        style:transform={`scale(${camera.zoom / 100}) translate(${-camera.origin.x}px, ${-camera.origin.y}px)`}
        style:--cell-size={`${snapshot.grid.cell_px}px`}>
        {#each snapshot.blocks as block (block.id)}
          <div class="block" class:selected={selectedId === block.id} data-block-id={block.id}
            class:terminal-block={!!block.session_id}
            class:text-block={!!block.text}
            class:git-block={!!block.git_task_id}
            style:left={`${block.rect.x * snapshot.grid.cell_px}px`}
            style:top={`${block.rect.y * snapshot.grid.cell_px}px`}
            style:width={`${block.rect.width * snapshot.grid.cell_px}px`}
            style:height={`${block.rect.height * snapshot.grid.cell_px}px`}>
            <button type="button" class="block-select" data-testid="select-block" aria-describedby="layout-instructions"
              aria-label={`Select ${block.title}`} aria-pressed={selectedId === block.id}
              onclick={() => { if (!panTool) selectBlock(block.id); }}>
              <span class="block-header" data-drag-block={block.id} title="Drag to move">
                <span class="block-title" data-testid="block-title">{block.title}</span><span class="block-id">#{block.id}</span></span>
              {#if !block.session_id && !block.text && !block.git_task_id}<span class="block-content"><span class="placeholder-symbol" aria-hidden="true">▦</span>
                <span>Placeholder</span><span class="block-dimensions">{block.rect.width} × {block.rect.height} cells</span></span>
              {/if}
            </button>
            {#if block.session_id}<TerminalView sessionId={block.session_id}
              parentSessionId={sessions.find(s => s.session_id === block.session_id)?.parent_session_id ?? null}
              onparent={(id) => terminalAction('reopen',id)} onstate={refreshSessions} />{/if}
            {#if block.text}<TextView blockId={block.id} />{/if}
            {#if block.git_task_id}<GitView taskId={block.git_task_id} onrefresh={id => gitAction('refresh', id)} />{/if}
            <button type="button" class="resize-handle" data-resize-block={block.id} aria-describedby="layout-instructions"
              aria-label={`Resize ${block.title} from bottom right`} title="Drag to resize; Shift+arrows with keyboard"
              onclick={() => { if (!panTool) selectBlock(block.id); }}>↘</button>
          </div>
        {/each}
        {#if placement}
          <div class="placement-preview" class:invalid={!!previewError} data-testid="placement-preview" aria-hidden="true"
            style:left={`${placement.candidate.x * snapshot.grid.cell_px}px`}
            style:top={`${placement.candidate.y * snapshot.grid.cell_px}px`}
            style:width={`${Math.max(1, placement.candidate.width) * snapshot.grid.cell_px}px`}
            style:height={`${Math.max(1, placement.candidate.height) * snapshot.grid.cell_px}px`}>
            <span>{previewError ?? `${placement.kind === 'move' ? 'Move' : 'Resize'}: ${placement.candidate.x}, ${placement.candidate.y} · ${placement.candidate.width} × ${placement.candidate.height} cells`}</span>
          </div>
        {/if}
      </div>
    {/if}
  </main>

  <footer class="statusbar">
    {#if sessions.length > 1}<div aria-label="Terminal shortcuts">
      {#each sessions as session}<button type="button" data-reopen-session={session.session_id} disabled={unavailable}
        onclick={() => terminalAction('reopen',session.session_id)}>Show #{session.session_id}{session.parent_session_id ? ` ← #${session.parent_session_id}` : ''}</button>{/each}
    </div>{/if}
    {#if snapshot?.blocks.some(block => block.id === openedId && block.session_id)}
      <button type="button" data-testid="show-opened-terminal" onclick={showOpened}>Show attached terminal</button>
    {/if}
    {#if gitRuns.length}<div class="git-retained" aria-label="Retained Git runs">Git runs:
      {#each gitRuns as run}<button type="button" data-reopen-git={run.task_id} disabled={unavailable}
        onclick={() => gitAction('reopen', run.task_id)}>Show #{run.task_id}</button>{/each}
    </div>{/if}
    {#if snapshot?.blocks.some(block => block.id === openedId && block.git_task_id)}
      <button type="button" data-testid="show-opened-git" onclick={showOpened}>Show Git run</button>
    {/if}
    {#if snapshot?.blocks.some(block => block.id === openedId && block.text)}
      <button type="button" data-testid="show-opened-text" onclick={showOpened}>Show opened text</button>
    {/if}
    <span id="layout-instructions" class="sr-only">With Pan tool off, drag a block header to move or its bottom-right handle to resize.
      Focus and select a block, then use arrows to move or Shift+arrows to resize; Escape cancels a preview.</span>
    <span role="status" aria-live="polite" data-testid="workspace-status">
      {#if placement}{previewError ?? 'Placement preview; release to apply, Escape to cancel.'}
      {:else if loading}Loading workspace…{:else if needsReload}Workspace needs reload
      {:else if pending}Updating workspace…{:else if snapshot}✓ Workspace loaded{:else}Workspace unavailable{/if}
    </span>
    {#if snapshot}
      <div class="layout-details">
        <span data-testid="block-count">{snapshot.blocks.length} blocks</span>
        <span data-testid="selection-status">{selectedId ? `Selected #${selectedId}` : 'No selection'}</span>
        <span data-testid="grid-dimensions">{snapshot.grid.columns} columns × {snapshot.grid.rows} rows</span>
        <span data-testid="cell-size">{snapshot.grid.cell_px} px / cell</span>
      </div>
    {/if}
  </footer>
</div>

<dialog bind:this={quitDialog} aria-labelledby="quit-title">
  <h2 id="quit-title">Stop live Sessions and Tasks and quit?</h2>
  <p>Quitting stops shells, Git runs and their owned child processes.</p>
  <button type="button" data-testid="cancel-quit" onclick={() => quitDialog.close()}>Keep working</button>
  <button type="button" data-testid="confirm-quit" disabled={pending}
    onclick={() => { quitDialog.close(); void terminalAction('quit'); }}>Stop Sessions and quit</button>
</dialog>
