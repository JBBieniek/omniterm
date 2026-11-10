<script lang="ts">
  import { onMount } from 'svelte';
  import { gitCommand, rawDisplay } from './git';
  import type { GitTask } from './git';
  let { taskId, onrefresh }: {taskId: string; onrefresh: (id: string) => void} = $props();
  let task = $state<GitTask | null>(null);
  let error = $state('');
  let pending = $state(false);
  let raw = $state(false);
  const live = $derived(!task || task.state === 'running' || task.state === 'draining');
  onMount(() => {
    let disposed = false;
    let timer: ReturnType<typeof setTimeout>;
    async function read() {
      try {
        const result = await gitCommand<{task: GitTask | null}>({op: 'read', task_id: taskId, after: task?.revision});
        if (disposed) return;
        if (result.task) task = result.task;
        if (live) timer = setTimeout(read, 200);
      } catch { if (!disposed) error = 'Could not inspect this run; show it again to retry the read.'; }
    }
    void read();
    return () => { disposed = true; clearTimeout(timer); };
  });
  async function cancel() {
    pending = true;
    try { await gitCommand({op: 'cancel', task_id: taskId}); }
    catch { error = 'Could not confirm cancellation; inspect the run before another action.'; }
    finally { pending = false; }
  }
</script>

<section class="git-view" aria-label="Git status run" data-git-task-id={taskId} data-git-state={task?.state ?? 'loading'}>
  {#if task}
    <div class="git-source">Task #{taskId} · from Session #{task.source_session_id} · {task.cwd}</div>
    <div class="git-outcome" data-testid="git-outcome" role="status">{task.state} · exit {task.exit_code ?? 'unavailable'} · {task.elapsed_ms} ms
      · stdout EOF {task.stdout_eof ? 'yes' : 'no'} · stderr EOF {task.stderr_eof ? 'yes' : 'no'}</div>
    {#if task.counts}
      <div class="git-counts" data-testid="git-counts"><span>Staged <strong>{task.counts.staged}</strong></span>
        <span>Unstaged <strong>{task.counts.unstaged}</strong></span><span>Untracked <strong>{task.counts.untracked}</strong></span></div>
    {:else}<p data-testid="git-unavailable">Counts unavailable</p>{/if}
    <p class="git-interpretation">{task.interpretation}</p>
    {#if task.error}<p role="alert">{task.error}</p>{/if}
    {#if task.dropped_bytes}<p role="alert" data-testid="git-gap">Raw gap: {task.dropped_bytes} bytes omitted; counts unavailable.</p>{/if}
    <div class="git-actions">
      <button type="button" data-testid="refresh-git" disabled={live} onclick={() => onrefresh(taskId)}>New status run</button>
      <button type="button" data-testid="cancel-git" disabled={!live || pending} onclick={cancel}>Cancel run</button>
      <button type="button" data-testid="raw-git" aria-expanded={raw} onclick={() => raw = !raw}>{raw ? 'Hide' : 'Show'} raw output</button>
    </div>
    {#if raw}
      <details><summary>Source and byte provenance</summary><p>{task.program ?? 'Git missing'} {task.argv.join(' ')}</p>
        <p>UTF-8 display; invalid bytes use replacement characters; controls are escaped; exact retained bytes stay in the runtime.</p>
        <pre>{task.frames.map(f => `#${f.sequence} +${f.elapsed_ms}ms ${f.stream} ${f.bytes.length} bytes`).join('\n')}</pre></details>
      <div class="git-raw" data-testid="git-raw" role="textbox" aria-readonly="true" aria-multiline="true" tabindex="0" aria-label="Raw Git stdout and stderr">{rawDisplay(task)}</div>
    {/if}
  {:else}<p role="status">Loading Git run…</p>{/if}
  {#if error}<p role="alert">{error}</p>{/if}
</section>
