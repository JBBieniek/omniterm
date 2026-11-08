use omniterm_core::Workspace;
use serde::{Deserialize, Serialize};

/// Exact opaque integers at the JavaScript boundary (FOUNDATION-01).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct WireU64(u64);

impl From<u64> for WireU64 {
    fn from(value: u64) -> Self {
        Self(value)
    }
}

impl From<WireU64> for String {
    fn from(value: WireU64) -> Self {
        value.0.to_string()
    }
}

impl TryFrom<String> for WireU64 {
    type Error = &'static str;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        let parsed = value.parse::<u64>().map_err(|_| "invalid u64 string")?;
        if parsed.to_string() != value {
            return Err("noncanonical u64 string");
        }
        Ok(Self(parsed))
    }
}

#[derive(Debug, Serialize)]
pub struct LayoutSnapshotV1 {
    api_version: u32,
    revision: WireU64,
    grid: GridSnapshot,
    blocks: Vec<BlockSnapshot>,
}

#[derive(Debug, Serialize)]
struct GridSnapshot {
    columns: u32,
    rows: u32,
    cell_px: u32,
}

#[derive(Debug, Serialize)]
struct BlockSnapshot {
    id: WireU64,
    title: String,
    rect: RectSnapshot,
}

#[derive(Debug, Serialize)]
struct RectSnapshot {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

impl LayoutSnapshotV1 {
    pub fn from_workspace(workspace: &Workspace, revision: u64) -> Self {
        let grid = workspace.grid();
        Self {
            api_version: 1,
            revision: revision.into(),
            grid: GridSnapshot {
                columns: grid.columns(),
                rows: grid.rows(),
                cell_px: grid.cell_px(),
            },
            blocks: workspace
                .blocks()
                .map(|block| {
                    let rect = block.rect();
                    BlockSnapshot {
                        id: block.id().0.into(),
                        title: block.title().to_owned(),
                        rect: RectSnapshot {
                            x: rect.x(),
                            y: rect.y(),
                            width: rect.width(),
                            height: rect.height(),
                        },
                    }
                })
                .collect(),
        }
    }
}
