<script lang="ts">
  import { onMount, tick } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';

  type LayoutSnapshotV1 = {
    api_version: 1;
    revision: string;
    grid: { columns: number; rows: number; cell_px: number };
    blocks: Array<{
      id: string;
      title: string;
      rect: { x: number; y: number; width: number; height: number };
    }>;
  };
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
  const unavailable = $derived(loading || pending || needsReload || !snapshot);

  function reconcile(next: LayoutSnapshotV1) {
    if (next.api_version !== 1) throw new Error('Unsupported workspace API version.');
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
  }

  async function mutate(command: 'add_placeholder' | 'remove_placement') {
    if (unavailable || !snapshot || (command === 'remove_placement' && !selectedId)) return;
    pending = true;
    error = '';
    let removed = false;
    try {
      const body = {
        api_version: 1,
        expected_revision: snapshot.revision,
        ...(command === 'add_placeholder' ? { title: title.trim() } : { block_id: selectedId }),
      };
      const result = await invoke<MutationResult>(command, new TextEncoder().encode(JSON.stringify(body)));
      reconcile(result.snapshot);
      if (result.status === 'error') error = result.error.message;
      else if (command === 'add_placeholder' && result.block_id) selectBlock(result.block_id);
      else removed = true;
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

  onMount(loadWorkspace);
</script>

<div class="application" data-rendered-revision={snapshot?.revision ?? ''}
  data-selected-block-id={selectedId ?? ''} data-busy={loading || pending}>
  <header class="toolbar">
    <div class="workspace-heading">
      <span class="workspace-mark" aria-hidden="true">▦</span>
      <h1>Workspace</h1>
    </div>
    <div class="view-state"><span class="zoom-label">Zoom 100%</span></div>
    <div class="placement-controls" aria-label="Placement controls">
      <label for="placeholder-title">Title</label>
      <input id="placeholder-title" data-testid="placeholder-title" bind:value={title}
        maxlength="256" disabled={unavailable} autocomplete="off" />
      <button type="button" data-testid="add-placeholder" bind:this={addButton}
        disabled={unavailable || !title.trim()} onclick={() => mutate('add_placeholder')}>Add placeholder</button>
      <button type="button" data-testid="remove-placement" disabled={unavailable || !selectedId}
        onclick={() => mutate('remove_placement')}>Remove placement</button>
    </div>
  </header>

  {#if error && snapshot}
    <div class="error-banner" role="alert" data-testid="command-error">
      <span>{error}</span>
      {#if needsReload}<button type="button" disabled={loading} onclick={loadWorkspace}>Reload workspace</button>{/if}
    </div>
  {/if}

  <main class="board-viewport" aria-label="Workspace viewport" aria-busy={loading || pending}>
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
        style:--cell-size={`${snapshot.grid.cell_px}px`}>
        {#each snapshot.blocks as block (block.id)}
          <button type="button" class="block" data-block-id={block.id}
            aria-label={`Select ${block.title}`} aria-pressed={selectedId === block.id}
            onclick={() => selectBlock(block.id)}
            style:left={`${block.rect.x * snapshot.grid.cell_px}px`}
            style:top={`${block.rect.y * snapshot.grid.cell_px}px`}
            style:width={`${block.rect.width * snapshot.grid.cell_px}px`}
            style:height={`${block.rect.height * snapshot.grid.cell_px}px`}>
            <span class="block-header"><span class="block-title" data-testid="block-title">{block.title}</span>
              <span class="block-id">#{block.id}</span></span>
            <span class="block-content"><span class="placeholder-symbol" aria-hidden="true">▦</span>
              <span>Placeholder</span><span class="block-dimensions">{block.rect.width} × {block.rect.height} cells</span></span>
          </button>
        {/each}
      </div>
    {/if}
  </main>

  <footer class="statusbar">
    <span role="status" aria-live="polite" data-testid="workspace-status">
      {#if loading}Loading workspace…{:else if needsReload}Workspace needs reload
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
