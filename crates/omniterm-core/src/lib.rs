//! Small, UI-independent grid model. Future features belong in later milestones.
#![forbid(unsafe_code)]

pub mod grid;
pub mod workspace;

pub use grid::{Camera, GridError, GridRect, GridSpec, Point, Zoom, ZOOM_LEVELS};
pub use workspace::{Block, BlockId, LayoutError, Workspace};
