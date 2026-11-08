//! Placement-only ownership. No process handles, Task state, or runtime dependencies.

use crate::grid::{GridRect, GridSpec};
use std::collections::BTreeMap;
use std::fmt;

/// Workspace-local seed ID. Persistence/global IDs are introduced in M02/M03.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct BlockId(pub u64);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    id: BlockId,
    rect: GridRect,
    title: String,
}

impl Block {
    pub fn new(id: BlockId, rect: GridRect, title: impl Into<String>) -> Self {
        Self {
            id,
            rect,
            title: title.into(),
        }
    }

    pub fn id(&self) -> BlockId {
        self.id
    }

    pub fn rect(&self) -> GridRect {
        self.rect
    }

    pub fn title(&self) -> &str {
        &self.title
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutError {
    DuplicateId,
    MissingBlock,
    OutOfBounds,
    Collision(BlockId),
}

impl fmt::Display for LayoutError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for LayoutError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Workspace {
    grid: GridSpec,
    blocks: BTreeMap<BlockId, Block>,
}

impl Workspace {
    pub fn new(grid: GridSpec) -> Self {
        Self {
            grid,
            blocks: BTreeMap::new(),
        }
    }

    pub fn grid(&self) -> GridSpec {
        self.grid
    }

    pub fn blocks(&self) -> impl Iterator<Item = &Block> {
        self.blocks.values()
    }

    pub fn get(&self, id: BlockId) -> Option<&Block> {
        self.blocks.get(&id)
    }

    /// First available rectangle, scanning rows before columns; no reflow.
    pub fn first_fit(&self, width: u32, height: u32) -> Option<GridRect> {
        if width == 0 || height == 0 {
            return None;
        }
        let last_x = self.grid.columns().checked_sub(width)?;
        let last_y = self.grid.rows().checked_sub(height)?;
        for y in 0..=last_y {
            for x in 0..=last_x {
                let rect = GridRect::new(x, y, width, height).ok()?;
                if self.validate_rect(rect, None).is_ok() {
                    return Some(rect);
                }
            }
        }
        None
    }

    pub fn add(&mut self, block: Block) -> Result<(), LayoutError> {
        if self.blocks.contains_key(&block.id) {
            return Err(LayoutError::DuplicateId);
        }
        self.validate_rect(block.rect, None)?;
        self.blocks.insert(block.id, block);
        Ok(())
    }

    /// Atomic placement commit: failed validation leaves existing layout untouched.
    pub fn place(&mut self, id: BlockId, rect: GridRect) -> Result<(), LayoutError> {
        if !self.blocks.contains_key(&id) {
            return Err(LayoutError::MissingBlock);
        }
        self.validate_rect(rect, Some(id))?;
        if let Some(block) = self.blocks.get_mut(&id) {
            block.rect = rect;
        }
        Ok(())
    }

    /// Remove a placement only. Resource termination must be an explicit later API.
    pub fn remove(&mut self, id: BlockId) -> Option<Block> {
        self.blocks.remove(&id)
    }

    /// Shrinking refuses to discard or silently move existing blocks.
    pub fn reconfigure(&mut self, grid: GridSpec) -> Result<(), LayoutError> {
        if self.blocks.values().any(|block| !grid.contains(block.rect)) {
            return Err(LayoutError::OutOfBounds);
        }
        self.grid = grid;
        Ok(())
    }

    fn validate_rect(&self, rect: GridRect, exclude: Option<BlockId>) -> Result<(), LayoutError> {
        if !self.grid.contains(rect) {
            return Err(LayoutError::OutOfBounds);
        }
        for block in self.blocks.values() {
            if Some(block.id) != exclude && block.rect.overlaps(rect) {
                return Err(LayoutError::Collision(block.id));
            }
        }
        Ok(())
    }
}
