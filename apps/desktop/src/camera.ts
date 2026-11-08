// Same logical-pixel transform as omniterm_core::grid::Camera:
// screen = (world - origin) * zoom/100; world = screen / (zoom/100) + origin.
// Subtract the viewport's client bounds before using pointer coordinates; DPI
// belongs to rasterization, never to cell geometry or this transform.
export const ZOOM_LEVELS = [25, 33, 50, 67, 100, 125, 150, 175, 200] as const;
export type Zoom = typeof ZOOM_LEVELS[number];
export type Point = { x: number; y: number };
export type Camera = { origin: Point; zoom: Zoom };

function checked(point: Point): Point {
  if (!Number.isFinite(point.x) || !Number.isFinite(point.y)) throw new RangeError('Invalid camera coordinate');
  return point;
}

export function viewportPoint(client: Point, bounds: { left: number; top: number }): Point {
  return checked({x: client.x - bounds.left, y: client.y - bounds.top});
}

export function worldToScreen(camera: Camera, world: Point): Point {
  const factor = camera.zoom / 100;
  return checked({x: (world.x - camera.origin.x) * factor, y: (world.y - camera.origin.y) * factor});
}

export function screenToWorld(camera: Camera, screen: Point): Point {
  const factor = camera.zoom / 100;
  return checked({x: screen.x / factor + camera.origin.x, y: screen.y / factor + camera.origin.y});
}

export function zoomAt(camera: Camera, zoom: Zoom, anchor: Point): Camera {
  const world = screenToWorld(camera, anchor);
  return {origin: checked({x: world.x - anchor.x / (zoom / 100), y: world.y - anchor.y / (zoom / 100)}), zoom};
}

// Positive delta moves the camera right/down, matching Rust's pan_screen.
export function panScreen(camera: Camera, dx: number, dy: number): Camera {
  return {origin: checked({x: camera.origin.x + dx / (camera.zoom / 100), y: camera.origin.y + dy / (camera.zoom / 100)}), zoom: camera.zoom};
}

export function stepZoom(zoom: Zoom, direction: -1 | 1): Zoom {
  return ZOOM_LEVELS[Math.max(0, Math.min(ZOOM_LEVELS.length - 1, ZOOM_LEVELS.indexOf(zoom) + direction))];
}

type ShortcutEvent = Pick<KeyboardEvent, 'key' | 'code' | 'ctrlKey' | 'altKey' | 'metaKey' | 'isComposing' | 'keyCode'>;
export function zoomShortcut(event: ShortcutEvent, blocked: boolean): -1 | 0 | 1 | null {
  if (blocked || event.isComposing || event.keyCode === 229 || !event.ctrlKey || event.altKey || event.metaKey) return null;
  if (event.key === '+' || event.key === '=' || event.code === 'Equal' || event.code === 'NumpadAdd') return 1;
  if (event.key === '-' || event.code === 'Minus' || event.code === 'NumpadSubtract') return -1;
  if (event.key === '0' || event.code === 'Digit0' || event.code === 'Numpad0') return 0;
  return null;
}
