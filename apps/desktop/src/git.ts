import { invoke } from '@tauri-apps/api/core';
export type GitSummary = {task_id: string; cwd: string; state: string};
export type GitTask = GitSummary & {revision: number; source_session_id: string; program: string | null;
  argv: string[]; exit_code: number | null; stdout_eof: boolean; stderr_eof: boolean; elapsed_ms: number;
  counts: {staged: number; unstaged: number; untracked: number} | null; interpretation: string;
  error: string | null; dropped_bytes: number;
  frames: Array<{sequence: number; elapsed_ms: number; stream: string; bytes: number[]}>};
export function gitCommand<T>(command: object): Promise<T> {
  return invoke<T>('git_command', new TextEncoder().encode(JSON.stringify({api_version: 1, command})));
}
// Raw bytes stay in Rust; this is inert display decoding, never terminal control parsing.
export function rawDisplay(task: GitTask): string {
  return ['stdout', 'stderr'].map(stream => {
    const frames = task.frames.filter(frame => frame.stream === stream);
    const bytes = new Uint8Array(frames.reduce((n, frame) => n + frame.bytes.length, 0));
    let offset = 0;
    for (const frame of frames) { bytes.set(frame.bytes, offset); offset += frame.bytes.length; }
    const text = new TextDecoder().decode(bytes).replace(/[\x00-\x08\x0b\x0c\x0e-\x1f\x7f]/g,
      c => `\\x${c.charCodeAt(0).toString(16).padStart(2, '0')}`);
    return `${stream} (${bytes.length} retained bytes)\n${text}`;
  }).join('\n');
}
