use omniterm_core::BlockId;
use omniterm_core::Workspace;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

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

impl From<WireU64> for u64 {
    fn from(value: WireU64) -> Self {
        value.0
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
    #[serde(skip_serializing_if = "Option::is_none")]
    session_id: Option<WireU64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<TextMetadata>,
}

#[derive(Debug, Serialize)]
struct TextMetadata {
    source_session_id: WireU64,
    path: String,
    bytes: usize,
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
                        session_id: None,
                        text: None,
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

    pub(crate) fn with_sessions(mut self, bindings: &BTreeMap<BlockId, u64>) -> Self {
        for block in &mut self.blocks {
            block.session_id = bindings
                .get(&BlockId(u64::from(block.id)))
                .copied()
                .map(Into::into);
        }
        self
    }
    pub(crate) fn with_text(
        mut self,
        documents: &BTreeMap<BlockId, crate::open_call::TextDocument>,
    ) -> Self {
        for block in &mut self.blocks {
            block.text = documents
                .get(&BlockId(block.id.into()))
                .map(|doc| TextMetadata {
                    source_session_id: doc.source_session_id,
                    path: doc.path.clone(),
                    bytes: doc.bytes,
                });
        }
        self
    }
}
