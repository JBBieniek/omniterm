use crate::snapshot::{LayoutSnapshotV1, WireU64};
use omniterm_core::{Block, BlockId, GridRect, GridSpec, LayoutError, Workspace};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use tauri::ipc::InvokeBody;

const MAX_PAYLOAD: usize = 64 * 1024;
const MAX_TITLE: usize = 256;
const MAX_BLOCKS: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceHost {
    workspace: Workspace,
    revision: u64,
    next_id: Option<u64>,
}

impl Default for WorkspaceHost {
    fn default() -> Self {
        let mut workspace = Workspace::new(GridSpec::default());
        for (id, x, width, title) in [(1, 0, 12, "Placeholder 1"), (2, 12, 8, "Placeholder 2")] {
            workspace
                .add(Block::new(
                    BlockId(id),
                    GridRect::new(x, 0, width, 8).expect("valid initial rectangle"),
                    title,
                ))
                .expect("valid initial placement");
        }
        Self {
            workspace,
            revision: 0,
            next_id: Some(3),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct CommandError {
    code: &'static str,
    message: &'static str,
}

impl CommandError {
    fn new(code: &'static str, message: &'static str) -> Self {
        Self { code, message }
    }
}

#[derive(Debug, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum MutationResult {
    Ok {
        snapshot: LayoutSnapshotV1,
        #[serde(skip_serializing_if = "Option::is_none")]
        block_id: Option<WireU64>,
    },
    Error {
        error: CommandError,
        snapshot: LayoutSnapshotV1,
    },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AddRequest {
    api_version: u32,
    expected_revision: WireU64,
    title: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RemoveRequest {
    api_version: u32,
    expected_revision: WireU64,
    block_id: WireU64,
}

fn decode<T: DeserializeOwned>(body: &InvokeBody) -> Result<T, CommandError> {
    let InvokeBody::Raw(bytes) = body else {
        return Err(CommandError::new(
            "invalid_input",
            "Send UTF-8 JSON as a raw command body.",
        ));
    };
    if bytes.len() > MAX_PAYLOAD {
        return Err(CommandError::new(
            "limit_exceeded",
            "Command payload exceeds 64 KiB.",
        ));
    }
    serde_json::from_slice(bytes).map_err(|_| {
        CommandError::new(
            "invalid_input",
            "The command has invalid or missing fields.",
        )
    })
}

fn layout_error(error: LayoutError) -> CommandError {
    match error {
        LayoutError::DuplicateId => {
            CommandError::new("internal_error", "Could not allocate a unique block ID.")
        }
        LayoutError::MissingBlock => CommandError::new("not_found", "That block no longer exists."),
        LayoutError::OutOfBounds => {
            CommandError::new("out_of_bounds", "The placement is outside the board.")
        }
        LayoutError::Collision(_) => {
            CommandError::new("collision", "The placement overlaps another block.")
        }
    }
}

impl WorkspaceHost {
    pub fn snapshot(&self) -> LayoutSnapshotV1 {
        LayoutSnapshotV1::from_workspace(&self.workspace, self.revision)
    }

    fn validate(&self, version: u32, revision: WireU64) -> Result<u64, CommandError> {
        if version != 1 {
            return Err(CommandError::new(
                "unsupported_version",
                "This command API version is not supported.",
            ));
        }
        if u64::from(revision) != self.revision {
            return Err(CommandError::new(
                "stale_revision",
                "The workspace changed; review its current layout and try again.",
            ));
        }
        self.revision.checked_add(1).ok_or_else(|| {
            CommandError::new(
                "limit_exceeded",
                "The layout revision limit has been reached.",
            )
        })
    }

    fn response(&self, result: Result<Option<WireU64>, CommandError>) -> MutationResult {
        match result {
            Ok(block_id) => MutationResult::Ok {
                snapshot: self.snapshot(),
                block_id,
            },
            Err(error) => MutationResult::Error {
                error,
                snapshot: self.snapshot(),
            },
        }
    }

    pub fn add(&mut self, body: &InvokeBody) -> MutationResult {
        let result = self.add_validated(body);
        self.response(result)
    }

    fn add_validated(&mut self, body: &InvokeBody) -> Result<Option<WireU64>, CommandError> {
        let request: AddRequest = decode(body)?;
        let revision = self.validate(request.api_version, request.expected_revision)?;
        if request.title.len() > MAX_TITLE || self.workspace.blocks().count() >= MAX_BLOCKS {
            return Err(CommandError::new(
                "limit_exceeded",
                "Titles may use at most 256 UTF-8 bytes, and the workspace at most 256 blocks.",
            ));
        }
        if request.title.trim().is_empty() || request.title.chars().any(char::is_control) {
            return Err(CommandError::new(
                "invalid_input",
                "Enter a nonempty title without control characters.",
            ));
        }
        let id = self.next_id.ok_or_else(|| {
            CommandError::new("limit_exceeded", "The block ID limit has been reached.")
        })?;
        let rect = self.workspace.first_fit(12, 8).ok_or_else(|| {
            CommandError::new(
                "no_room",
                "No room for another 12 × 8 placeholder; remove a placement first.",
            )
        })?;
        self.workspace
            .add(Block::new(BlockId(id), rect, request.title))
            .map_err(layout_error)?;
        self.revision = revision;
        self.next_id = id.checked_add(1);
        Ok(Some(id.into()))
    }

    pub fn remove(&mut self, body: &InvokeBody) -> MutationResult {
        let result = self.remove_validated(body);
        self.response(result)
    }

    fn remove_validated(&mut self, body: &InvokeBody) -> Result<Option<WireU64>, CommandError> {
        let request: RemoveRequest = decode(body)?;
        let revision = self.validate(request.api_version, request.expected_revision)?;
        self.workspace
            .remove(BlockId(u64::from(request.block_id)))
            .ok_or_else(|| layout_error(LayoutError::MissingBlock))?;
        self.revision = revision;
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};
    use tauri::ipc::InvokeBody;

    fn raw(value: Value) -> InvokeBody {
        InvokeBody::Raw(serde_json::to_vec(&value).unwrap())
    }

    fn add(revision: u64, title: &str) -> InvokeBody {
        raw(json!({"api_version":1,"expected_revision":revision.to_string(),"title":title}))
    }

    fn remove(revision: u64, id: &str) -> InvokeBody {
        raw(json!({"api_version":1,"expected_revision":revision.to_string(),"block_id":id}))
    }

    fn value(result: MutationResult) -> Value {
        serde_json::to_value(result).unwrap()
    }

    fn rejected(host: &mut WorkspaceHost, body: InvokeBody, code: &str, is_add: bool) {
        let before = host.clone();
        let response = value(if is_add {
            host.add(&body)
        } else {
            host.remove(&body)
        });
        assert_eq!(response["status"], "error");
        assert_eq!(response["error"]["code"], code);
        assert_eq!(
            response["snapshot"],
            serde_json::to_value(before.snapshot()).unwrap()
        );
        assert_eq!(
            *host, before,
            "failed commands preserve layout, revision and next ID"
        );
    }

    #[test]
    fn seed_and_mutations_keep_exact_ids_and_revisions() {
        let mut host = WorkspaceHost::default();
        let initial = serde_json::to_value(host.snapshot()).unwrap();
        assert_eq!(initial["revision"], "0");
        assert_eq!(initial["blocks"].as_array().unwrap().len(), 2);
        assert_eq!(
            initial["blocks"][0]["rect"],
            json!({"x":0,"y":0,"width":12,"height":8})
        );
        assert_eq!(
            initial["blocks"][1]["rect"],
            json!({"x":12,"y":0,"width":8,"height":8})
        );
        let result = value(host.add(&add(0, "<img src=x onerror=alert(1)>")));
        assert_eq!(result["status"], "ok");
        assert_eq!(result["block_id"], "3");
        assert_eq!(result["snapshot"]["revision"], "1");
        assert_eq!(result["snapshot"]["blocks"][0], initial["blocks"][0]);
        assert_eq!(result["snapshot"]["blocks"][1], initial["blocks"][1]);
        assert_eq!(
            result["snapshot"]["blocks"][2]["rect"],
            json!({"x":20,"y":0,"width":12,"height":8})
        );
        assert_eq!(
            result["snapshot"]["blocks"][2]["title"],
            "<img src=x onerror=alert(1)>"
        );
        let removed = value(host.remove(&remove(1, "3")));
        assert_eq!(removed["snapshot"]["revision"], "2");
        assert!(removed.get("block_id").is_none());
        let next = value(host.add(&add(2, "Next")));
        assert_eq!(next["block_id"], "4");
        assert_eq!(
            next["snapshot"]["blocks"][2]["rect"],
            result["snapshot"]["blocks"][2]["rect"]
        );
    }

    #[test]
    fn malformed_versions_revisions_and_ids_are_atomic() {
        let mut host = WorkspaceHost::default();
        rejected(
            &mut host,
            raw(json!({"api_version":2,"expected_revision":"0","title":"x"})),
            "unsupported_version",
            true,
        );
        for revision in [
            json!(0),
            json!("01"),
            json!("+1"),
            json!("18446744073709551616"),
        ] {
            rejected(
                &mut host,
                raw(json!({"api_version":1,"expected_revision":revision,"title":"x"})),
                "invalid_input",
                true,
            );
        }
        for body in [
            raw(json!({})),
            raw(json!({"api_version":1,"expected_revision":"0","title":"x","width":1})),
            InvokeBody::Raw(b"not json".to_vec()),
            InvokeBody::Raw(vec![255]),
            InvokeBody::Json(json!({})),
        ] {
            rejected(&mut host, body, "invalid_input", true);
        }
        rejected(&mut host, remove(0, "999"), "not_found", false);
        rejected(&mut host, remove(0, "01"), "invalid_input", false);
        value(host.add(&add(0, "Accepted")));
        rejected(&mut host, add(0, "Stale"), "stale_revision", true);
        rejected(&mut host, remove(0, "1"), "stale_revision", false);
    }

    #[test]
    fn concurrent_requests_with_one_revision_commit_only_once() {
        use std::sync::{Arc, Mutex};
        let host = Arc::new(Mutex::new(WorkspaceHost::default()));
        let handles: Vec<_> = (0..2)
            .map(|_| {
                let host = Arc::clone(&host);
                std::thread::spawn(move || value(host.lock().unwrap().add(&add(0, "Concurrent"))))
            })
            .collect();
        let results: Vec<_> = handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect();
        assert_eq!(
            results
                .iter()
                .filter(|result| result["status"] == "ok")
                .count(),
            1
        );
        assert_eq!(
            results
                .iter()
                .filter(|result| result["error"]["code"] == "stale_revision")
                .count(),
            1
        );
        let host = host.lock().unwrap();
        assert_eq!(host.revision, 1);
        assert_eq!(host.workspace.blocks().count(), 3);
        assert_eq!(host.next_id, Some(4));
    }

    #[test]
    fn utf8_title_and_raw_payload_limits_are_enforced() {
        let mut host = WorkspaceHost::default();
        for title in ["".to_owned(), " \t".to_owned(), "bad\0title".to_owned()] {
            rejected(&mut host, add(0, &title), "invalid_input", true);
        }
        rejected(&mut host, add(0, &"é".repeat(129)), "limit_exceeded", true);
        rejected(
            &mut host,
            InvokeBody::Raw(vec![b' '; MAX_PAYLOAD + 1]),
            "limit_exceeded",
            true,
        );
        let InvokeBody::Raw(mut exact) = add(0, &"é".repeat(128)) else {
            unreachable!()
        };
        exact.resize(MAX_PAYLOAD, b' ');
        assert_eq!(value(host.add(&InvokeBody::Raw(exact)))["status"], "ok");
    }

    #[test]
    fn full_board_and_block_cap_preserve_state() {
        let mut host = WorkspaceHost::default();
        while value(host.add(&add(host.revision, "Fill")))["status"] == "ok" {}
        let revision = host.revision;
        rejected(&mut host, add(revision, "No room"), "no_room", true);
        let mut workspace = Workspace::new(GridSpec::new(32, 32, 64).unwrap());
        for id in 1..=256 {
            workspace
                .add(Block::new(
                    BlockId(id),
                    GridRect::new(((id - 1) % 32) as u32, ((id - 1) / 32) as u32, 1, 1).unwrap(),
                    "x",
                ))
                .unwrap();
        }
        host.workspace = workspace;
        host.next_id = Some(257);
        rejected(&mut host, add(revision, "Cap"), "limit_exceeded", true);
    }

    #[test]
    fn id_and_revision_exhaustion_never_wrap_or_partially_commit() {
        let mut host = WorkspaceHost {
            next_id: Some(u64::MAX),
            ..WorkspaceHost::default()
        };
        assert_eq!(
            value(host.add(&add(0, "Last ID")))["block_id"],
            u64::MAX.to_string()
        );
        rejected(&mut host, add(1, "Exhausted"), "limit_exceeded", true);
        host.revision = u64::MAX;
        rejected(&mut host, remove(u64::MAX, "1"), "limit_exceeded", false);
        host.next_id = Some(100);
        rejected(
            &mut host,
            add(u64::MAX, "Revision exhausted"),
            "limit_exceeded",
            true,
        );
    }
}
