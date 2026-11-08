import type { Point } from './camera';

export type Rect = { x: number; y: number; width: number; height: number };
type Grid = { columns: number; rows: number };
type PlacedBlock = { id: string; rect: Rect; session_id?: string };

export function snapDelta(cells: number): number {
  if (!Number.isFinite(cells)) throw new RangeError('Invalid placement delta');
  const snapped = Math.floor(Math.abs(cells) + .5);
  return cells < 0 && snapped !== 0 ? -snapped : snapped;
}

// Keep the original rectangle and world grab point for the entire gesture.
// Snap the delta once, rather than rounding successive pointer movements.
export function placementCandidate(original: Rect, grab: Point, current: Point, cellPx: number, kind: 'move' | 'resize'): Rect {
  // Camera round trips can put a mathematical half-cell just below the tie.
  // Normalize that transform noise to 1e-9 cells before the single snap.
  const dx = snapDelta(Number(((current.x - grab.x) / cellPx).toFixed(9)));
  const dy = snapDelta(Number(((current.y - grab.y) / cellPx).toFixed(9)));
  return kind === 'move'
    ? {...original, x: original.x + dx, y: original.y + dy}
    : {...original, width: original.width + dx, height: original.height + dy};
}

// Preview hints only; Rust validates every submitted placement independently.
export function placementError(rect: Rect, grid: Grid, blocks: PlacedBlock[], id: string): string | null {
  if (!Object.values(rect).every(Number.isSafeInteger) || rect.width < 1 || rect.height < 1) return 'Use at least 1 × 1 cell.';
  if (blocks.some(block => block.id === id && block.session_id) && (rect.width < 4 || rect.height < 3)) return 'Terminal blocks need at least 4 × 3 cells.';
  if (rect.x < 0 || rect.y < 0 || rect.x + rect.width > grid.columns || rect.y + rect.height > grid.rows) return 'The placement is outside the board.';
  if (blocks.some(block => block.id !== id && rect.x < block.rect.x + block.rect.width && block.rect.x < rect.x + rect.width
    && rect.y < block.rect.y + block.rect.height && block.rect.y < rect.y + rect.height)) return 'The placement overlaps another block.';
  return null;
}

export function sameRect(a: Rect, b: Rect): boolean {
  return a.x === b.x && a.y === b.y && a.width === b.width && a.height === b.height;
}

export function keyboardPlacement(rect: Rect, key: string, resize: boolean): Rect | null {
  const dx = key === 'ArrowLeft' ? -1 : key === 'ArrowRight' ? 1 : 0;
  const dy = key === 'ArrowUp' ? -1 : key === 'ArrowDown' ? 1 : 0;
  if (!dx && !dy) return null;
  return resize ? {...rect, width: rect.width + dx, height: rect.height + dy} : {...rect, x: rect.x + dx, y: rect.y + dy};
}
