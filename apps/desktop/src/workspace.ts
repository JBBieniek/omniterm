import type { Rect } from './placement';
export type Layout = {api_version: 1; revision: string;
  grid: {columns: number; rows: number; cell_px: number};
  blocks: Array<{id: string; title: string; rect: Rect; session_id?: string; git_task_id?: string;
    text?: {source_session_id: string; path: string; bytes: number}}>};
