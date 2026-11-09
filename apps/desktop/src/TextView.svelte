<script lang="ts">
  import { onMount } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  let { blockId }: {blockId: string} = $props();
  let document = $state<{text: string; path: string; bytes: number; source_session_id: string} | null>(null);
  let error = $state('');
  onMount(() => {
    let disposed = false;
    void invoke<typeof document>('get_text_block', new TextEncoder().encode(JSON.stringify({api_version: 1, block_id: blockId})))
      .then(value => { if (!disposed) document = value; })
      .catch(() => { if (!disposed) error = 'Could not load this text snapshot; remove and open it again.'; });
    return () => { disposed = true; };
  });
</script>

<section class="text-view" aria-label="Opened text" data-testid="text-view" data-text-block-id={blockId}>
  {#if document}
    <div class="text-source">{document.path} · {document.bytes} bytes · from Session #{document.source_session_id} · snapshot</div>
    <div role="textbox" aria-readonly="true" aria-multiline="true" tabindex="0"
      aria-label={`Text from ${document.path}`} data-testid="text-content">{document.text}</div>
  {:else if error}<p role="alert">{error}</p>
  {:else}<p role="status">Loading text…</p>{/if}
</section>
