<script lang="ts">
  import { onMount } from 'svelte';
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

  let snapshot = $state<LayoutSnapshotV1 | null>(null);
  let loading = $state(true);
  let error = $state('');

  async function loadWorkspace() {
    loading = true;
    error = '';
    snapshot = null;
    try {
      const next = await invoke<LayoutSnapshotV1>('get_layout_snapshot');
      if (next.api_version !== 1) throw new Error('Unsupported workspace API version.');
      snapshot = next;
    } catch {
      error = 'Could not load the workspace. Please retry or restart Omniterm.';
    } finally {
      loading = false;
    }
  }

  onMount(loadWorkspace);
</script>

<div class="application" data-rendered-revision={snapshot?.revision ?? ''}>
  <header class="toolbar">
    <div class="workspace-heading">
      <span class="workspace-mark" aria-hidden="true">▦</span>
      <h1>Workspace</h1>
    </div>
    <div class="view-state"><span class="read-only">Read-only</span><span class="zoom-label">Zoom 100%</span></div>
  </header>

  <main class="board-viewport" aria-label="Workspace viewport" aria-busy={loading}>
    {#if error}
      <section class="empty-state">
        <div class="workspace-symbol" aria-hidden="true">!</div>
        <h2>Workspace unavailable</h2>
        <p role="alert">{error}</p>
        <button type="button" onclick={loadWorkspace}>Retry</button>
      </section>
    {:else if loading}
      <section class="empty-state"><h2>Loading workspace…</h2></section>
    {:else if snapshot}
      <section class="empty-state">
        <div class="workspace-symbol" aria-hidden="true">▦</div>
        <h2>Your workspace</h2>
        <p data-testid="block-count">{snapshot.blocks.length} blocks</p>
      </section>
    {/if}
  </main>

  <footer class="statusbar">
    <span role="status" aria-live="polite" data-testid="workspace-status">
      {#if snapshot}<span aria-hidden="true">✓</span> Workspace loaded
      {:else if loading}Loading workspace…{:else}Workspace unavailable{/if}
    </span>
    {#if snapshot}
      <div class="layout-details">
        <span data-testid="grid-dimensions">{snapshot.grid.columns} columns × {snapshot.grid.rows} rows</span>
        <span data-testid="cell-size">{snapshot.grid.cell_px} px / cell</span>
      </div>
    {/if}
  </footer>
</div>
