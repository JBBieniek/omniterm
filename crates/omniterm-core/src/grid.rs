//! Integer world placement and a separate camera in logical (not device) pixels.

use std::fmt;

pub const ZOOM_LEVELS: [u16; 9] = [25, 33, 50, 67, 100, 125, 150, 175, 200];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GridError {
    InvalidGrid,
    InvalidRect,
    InvalidZoom,
    InvalidCoordinate,
}

impl fmt::Display for GridError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for GridError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GridSpec {
    columns: u32,
    rows: u32,
    cell_px: u32,
}

impl Default for GridSpec {
    fn default() -> Self {
        Self {
            columns: 48,
            rows: 32,
            cell_px: 64,
        }
    }
}

impl GridSpec {
    pub fn new(columns: u32, rows: u32, cell_px: u32) -> Result<Self, GridError> {
        if !(1..=512).contains(&columns)
            || !(1..=512).contains(&rows)
            || !(16..=256).contains(&cell_px)
        {
            return Err(GridError::InvalidGrid);
        }
        Ok(Self {
            columns,
            rows,
            cell_px,
        })
    }

    pub fn columns(self) -> u32 {
        self.columns
    }

    pub fn rows(self) -> u32 {
        self.rows
    }

    pub fn cell_px(self) -> u32 {
        self.cell_px
    }

    pub fn contains(self, rect: GridRect) -> bool {
        u64::from(rect.x) + u64::from(rect.width) <= u64::from(self.columns)
            && u64::from(rect.y) + u64::from(rect.height) <= u64::from(self.rows)
    }

    /// Convert world pixels to a cell, rejecting positions outside the finite grid.
    /// This is hit-testing (floor), not drag-delta snapping (nearest).
    pub fn world_cell(self, world: Point) -> Result<(u32, u32), GridError> {
        if !world.is_finite() || world.x < 0.0 || world.y < 0.0 {
            return Err(GridError::InvalidCoordinate);
        }
        let column = (world.x / f64::from(self.cell_px)).floor();
        let row = (world.y / f64::from(self.cell_px)).floor();
        if column >= f64::from(self.columns) || row >= f64::from(self.rows) {
            return Err(GridError::InvalidCoordinate);
        }
        Ok((column as u32, row as u32))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GridRect {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

impl GridRect {
    pub fn new(x: u32, y: u32, width: u32, height: u32) -> Result<Self, GridError> {
        if width == 0 || height == 0 {
            return Err(GridError::InvalidRect);
        }
        Ok(Self {
            x,
            y,
            width,
            height,
        })
    }

    pub fn x(self) -> u32 {
        self.x
    }

    pub fn y(self) -> u32 {
        self.y
    }

    pub fn width(self) -> u32 {
        self.width
    }

    pub fn height(self) -> u32 {
        self.height
    }

    /// Half-open rectangles: adjacent edges do not collide. Widen before addition.
    pub fn overlaps(self, other: Self) -> bool {
        u64::from(self.x) < u64::from(other.x) + u64::from(other.width)
            && u64::from(other.x) < u64::from(self.x) + u64::from(self.width)
            && u64::from(self.y) < u64::from(other.y) + u64::from(other.height)
            && u64::from(other.y) < u64::from(self.y) + u64::from(self.height)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

impl Point {
    pub fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }

    pub fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Zoom {
    index: usize,
}

impl Default for Zoom {
    fn default() -> Self {
        Self { index: 4 }
    }
}

impl Zoom {
    pub fn from_percent(percent: u16) -> Result<Self, GridError> {
        ZOOM_LEVELS
            .iter()
            .position(|value| *value == percent)
            .map(|index| Self { index })
            .ok_or(GridError::InvalidZoom)
    }

    pub fn percent(self) -> u16 {
        ZOOM_LEVELS[self.index]
    }

    pub fn factor(self) -> f64 {
        f64::from(self.percent()) / 100.0
    }

    pub fn step_in(self) -> Self {
        Self {
            index: (self.index + 1).min(ZOOM_LEVELS.len() - 1),
        }
    }

    pub fn step_out(self) -> Self {
        Self {
            index: self.index.saturating_sub(1),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Camera {
    origin: Point,
    zoom: Zoom,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            origin: Point::new(0.0, 0.0),
            zoom: Zoom::default(),
        }
    }
}

impl Camera {
    pub fn new(origin: Point, zoom: Zoom) -> Result<Self, GridError> {
        if !origin.is_finite() {
            return Err(GridError::InvalidCoordinate);
        }
        Ok(Self { origin, zoom })
    }

    pub fn origin(self) -> Point {
        self.origin
    }

    pub fn zoom(self) -> Zoom {
        self.zoom
    }

    pub fn world_to_screen(self, world: Point) -> Result<Point, GridError> {
        checked(Point::new(
            (world.x - self.origin.x) * self.zoom.factor(),
            (world.y - self.origin.y) * self.zoom.factor(),
        ))
    }

    pub fn screen_to_world(self, screen: Point) -> Result<Point, GridError> {
        checked(Point::new(
            screen.x / self.zoom.factor() + self.origin.x,
            screen.y / self.zoom.factor() + self.origin.y,
        ))
    }

    /// Keep the world point under a viewport-local screen anchor fixed.
    pub fn zoom_at(&mut self, zoom: Zoom, anchor: Point) -> Result<(), GridError> {
        let world = self.screen_to_world(anchor)?;
        let origin = checked(Point::new(
            world.x - anchor.x / zoom.factor(),
            world.y - anchor.y / zoom.factor(),
        ))?;
        self.origin = origin;
        self.zoom = zoom;
        Ok(())
    }

    /// Camera movement in screen pixels. Positive dx moves the viewport right.
    pub fn pan_screen(&mut self, dx: f64, dy: f64) -> Result<(), GridError> {
        let origin = checked(Point::new(
            self.origin.x + dx / self.zoom.factor(),
            self.origin.y + dy / self.zoom.factor(),
        ))?;
        self.origin = origin;
        Ok(())
    }
}

fn checked(point: Point) -> Result<Point, GridError> {
    if point.is_finite() {
        Ok(point)
    } else {
        Err(GridError::InvalidCoordinate)
    }
}
