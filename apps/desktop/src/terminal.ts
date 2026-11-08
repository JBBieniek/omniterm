import { invoke } from '@tauri-apps/api/core';

export type CellRun = { column: number; width: number; text: string; fg: string; bg: string;
  bold: boolean; dim: boolean; italic: boolean; underline: boolean };
export type Screen = { session_id: string; revision: string; rows: number; cols: number;
  cursor_row: number; cursor_col: number; cursor_visible: boolean; lines: CellRun[][];
  state: string; exit_code: number | null; raw_bytes: number; evicted_bytes: number;
  output_eof: boolean; read_failed: boolean; reply_dropped: boolean };
export type Session = {session_id: string; state: string; exit_code: number | null};
export type TerminalReply = {status: 'ok'; screen?: Screen; sessions?: Session[];
  layout?: import('./workspace').Layout; block_id?: string; session_id?: string}
  | {status: 'error'; error: {code: string; message: string}};

// A failed transport is deliberately never retried: input/start may already have run.
export function terminalCommand(command: Record<string, unknown>): Promise<TerminalReply> {
  return invoke('terminal_command', new TextEncoder().encode(JSON.stringify({api_version: 1, command})));
}

export function fitTerminal(width: number, height: number, cellWidth: number) {
  return {cols: Math.max(10, Math.min(120, Math.floor(width / cellWidth))),
    rows: Math.max(2, Math.min(60, Math.floor(height / 20)))};
}
