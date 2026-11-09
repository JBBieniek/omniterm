use crate::snapshot::{LayoutSnapshotV1, WireU64};
use omniterm_core::{Block, BlockId, GridRect, GridSpec, LayoutError, Workspace};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::collections::BTreeMap;
use tauri::ipc::InvokeBody;

const MAX_PAYLOAD: usize = 64 * 1024;
const MAX_TITLE: usize = 256;
const MAX_BLOCKS: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceHost {
    workspace: Workspace,
    revision: u64,
    next_id: Option<u64>,
    session_bindings: BTreeMap<BlockId, u64>,
    text_documents: BTreeMap<BlockId, crate::open_call::TextDocument>,
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
            session_bindings: BTreeMap::new(),
            text_documents: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct CommandError {
    code: &'static str,
    pub(crate) message: &'static str,
}

impl CommandError {
    pub(crate) fn new(code: &'static str, message: &'static str) -> Self {
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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PlaceRequest {
    api_version: u32,
    expected_revision: WireU64,
    block_id: WireU64,
    rect: PlaceRect,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PlaceRect {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

pub(crate) fn decode<T: DeserializeOwned>(body: &InvokeBody) -> Result<T, CommandError> {
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
            .with_sessions(&self.session_bindings)
            .with_text(&self.text_documents)
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
        self.session_bindings
            .remove(&BlockId(u64::from(request.block_id)));
        self.text_documents
            .remove(&BlockId(u64::from(request.block_id)));
        self.revision = revision;
        Ok(None)
    }

    pub fn place(&mut self, body: &InvokeBody) -> MutationResult {
        let result = self.place_validated(body);
        self.response(result)
    }

    fn place_validated(&mut self, body: &InvokeBody) -> Result<Option<WireU64>, CommandError> {
        let request: PlaceRequest = decode(body)?;
        let revision = self.validate(request.api_version, request.expected_revision)?;
        let rect = GridRect::new(
            request.rect.x,
            request.rect.y,
            request.rect.width,
            request.rect.height,
        )
        .map_err(|_| CommandError::new("invalid_input", "Use at least 1 × 1 cell."))?;
        let id = BlockId(u64::from(request.block_id));
        if self.session_bindings.contains_key(&id) && (rect.width() < 4 || rect.height() < 3) {
            return Err(CommandError::new(
                "invalid_input",
                "Terminal blocks need at least 4 × 3 cells.",
            ));
        }
        let original = self
            .workspace
            .get(id)
            .ok_or_else(|| layout_error(LayoutError::MissingBlock))?
            .rect();
        self.workspace.place(id, rect).map_err(layout_error)?;
        if rect != original {
            self.revision = revision;
        }
        Ok(None)
    }

    pub(crate) fn check_content_room(&self, revision: WireU64) -> Result<(), CommandError> {
        self.validate(1, revision)?;
        if self.next_id.is_none() || self.workspace.blocks().count() >= MAX_BLOCKS {
            return Err(CommandError::new(
                "limit_exceeded",
                "The placement limit has been reached.",
            ));
        }
        self.workspace.first_fit(12, 8).ok_or_else(|| {
            CommandError::new(
                "no_room",
                "No room for a 12 × 8 block; remove a placement first.",
            )
        })?;
        Ok(())
    }
    pub(crate) fn attach_terminal(
        &mut self,
        session: u64,
        expected: WireU64,
    ) -> Result<WireU64, CommandError> {
        let revision = self.validate(1, expected)?;
        if let Some((id, _)) = self
            .session_bindings
            .iter()
            .find(|(_, value)| **value == session)
        {
            return Ok(id.0.into());
        }
        self.check_content_room(expected)?;
        let id = self.next_id.unwrap();
        let rect = self.workspace.first_fit(12, 8).unwrap();
        self.workspace
            .add(Block::new(
                BlockId(id),
                rect,
                format!("Terminal #{session}"),
            ))
            .map_err(layout_error)?;
        self.session_bindings.insert(BlockId(id), session);
        self.next_id = id.checked_add(1);
        self.revision = revision;
        Ok(id.into())
    }

    pub(crate) fn add_text(
        &mut self,
        document: crate::open_call::TextDocument,
    ) -> Result<WireU64, CommandError> {
        self.check_content_room(self.revision.into())?;
        if self.text_documents.len() >= 8
            || document.bytes > crate::open_call::MAX_TEXT
            || document.text.len() > crate::open_call::MAX_TEXT
        {
            return Err(CommandError::new(
                "limit_exceeded",
                "Text storage limit reached; remove a text placement first.",
            ));
        }
        let revision = self.validate(1, self.revision.into())?;
        let id = self.next_id.unwrap();
        let rect = self.workspace.first_fit(12, 8).unwrap();
        let name = std::path::Path::new(&document.path)
            .file_name()
            .and_then(|p| p.to_str())
            .unwrap_or("Text");
        let title = name
            .chars()
            .scan(0, |bytes, c| {
                *bytes += c.len_utf8();
                (*bytes <= MAX_TITLE).then_some(c)
            })
            .collect::<String>();
        self.workspace
            .add(Block::new(BlockId(id), rect, title))
            .map_err(layout_error)?;
        self.text_documents.insert(BlockId(id), document);
        self.next_id = id.checked_add(1);
        self.revision = revision;
        Ok(id.into())
    }
    pub(crate) fn text_block(
        &self,
        body: &InvokeBody,
    ) -> Result<crate::open_call::TextDocument, CommandError> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct TextRequest {
            api_version: u32,
            block_id: WireU64,
        }
        let request: TextRequest = decode(body)?;
        if request.api_version != 1 {
            return Err(CommandError::new(
                "unsupported_version",
                "Unsupported text API version.",
            ));
        }
        self.text_documents
            .get(&BlockId(request.block_id.into()))
            .cloned()
            .ok_or_else(|| CommandError::new("not_found", "That text placement no longer exists."))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_storage_is_atomic_bounded_inert_and_removed_with_placement() {
        let mut host = WorkspaceHost::default();
        let document = crate::open_call::TextDocument {
            source_session_id: 7.into(),
            path: "sub/<img src=x>.txt".into(),
            bytes: 5,
            text: "hello".into(),
        };
        let id = host.add_text(document.clone()).unwrap();
        let body = InvokeBody::Raw(
            format!(r#"{{"api_version":1,"block_id":"{}"}}"#, u64::from(id)).into_bytes(),
        );
        assert_eq!(host.text_block(&body).unwrap(), document);
        let value = serde_json::to_value(host.snapshot()).unwrap();
        assert_eq!(value["blocks"][2]["text"]["source_session_id"], "7");
        assert!(value["blocks"][2]["text"].get("text").is_none());
        for _ in 1..8 {
            host.add_text(document.clone()).unwrap();
        }
        let before = host.clone();
        assert!(host.add_text(document).is_err());
        assert_eq!(host, before);
        let id = u64::from(id).to_string();
        let revision = host.revision;
        host.remove(&remove(revision, &id));
        assert!(host.text_block(&body).is_err());
        assert_eq!(host.text_documents.len(), 7);
    }
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

    fn place(revision: u64, id: &str, rect: Value) -> InvokeBody {
        raw(
            json!({"api_version":1,"expected_revision":revision.to_string(),"block_id":id,"rect":rect}),
        )
    }

    fn rejected_place(host: &mut WorkspaceHost, body: InvokeBody, code: &str) {
        let before = host.clone();
        let response = value(host.place(&body));
        assert_eq!(response["status"], "error");
        assert_eq!(response["error"]["code"], code);
        assert_eq!(
            response["snapshot"],
            serde_json::to_value(before.snapshot()).unwrap()
        );
        assert_eq!(
            *host, before,
            "rejection preserves every placement, revision and next ID"
        );
    }

    #[test]
    fn placement_commits_once_without_changing_identity_neighbors_or_next_id() {
        let mut host = WorkspaceHost::default();
        let seed = serde_json::to_value(host.snapshot()).unwrap();
        let target = json!({"x":1,"y":9,"width":14,"height":9});
        let result = value(host.place(&place(0, "1", target.clone())));
        assert_eq!(result["status"], "ok");
        assert_eq!(result["snapshot"]["revision"], "1");
        assert_eq!(result["snapshot"]["blocks"][0]["rect"], target);
        assert_eq!(
            result["snapshot"]["blocks"][0]["title"],
            seed["blocks"][0]["title"]
        );
        assert_eq!(result["snapshot"]["blocks"][1], seed["blocks"][1]);
        assert_eq!(host.next_id, Some(3));
        assert_eq!(
            value(host.place(&place(1, "1", target)))["snapshot"]["revision"],
            "1",
            "no-op preserves revision"
        );
        let minimum = json!({"x":47,"y":31,"width":1,"height":1});
        assert_eq!(value(host.place(&place(1, "1", minimum)))["status"], "ok");
    }

    #[test]
    fn placement_rejects_collision_bounds_missing_stale_and_exhaustion_atomically() {
        let mut host = WorkspaceHost::default();
        rejected_place(
            &mut host,
            place(0, "1", json!({"x":1,"y":0,"width":12,"height":8})),
            "collision",
        );
        rejected_place(
            &mut host,
            place(0, "1", json!({"x":48,"y":0,"width":1,"height":1})),
            "out_of_bounds",
        );
        rejected_place(
            &mut host,
            place(
                0,
                "1",
                json!({"x":u32::MAX,"y":0,"width":u32::MAX,"height":1}),
            ),
            "out_of_bounds",
        );
        rejected_place(
            &mut host,
            place(0, "1", json!({"x":0,"y":0,"width":0,"height":1})),
            "invalid_input",
        );
        rejected_place(
            &mut host,
            place(0, "99", json!({"x":0,"y":9,"width":1,"height":1})),
            "not_found",
        );
        assert_eq!(
            value(host.place(&place(0, "1", json!({"x":0,"y":8,"width":12,"height":8}))))["status"],
            "ok"
        );
        rejected_place(
            &mut host,
            place(0, "1", json!({"x":0,"y":10,"width":12,"height":8})),
            "stale_revision",
        );
        host.revision = u64::MAX;
        rejected_place(
            &mut host,
            place(u64::MAX, "1", json!({"x":0,"y":10,"width":12,"height":8})),
            "limit_exceeded",
        );
    }

    #[test]
    fn placement_decoding_preserves_trust_boundary() {
        let mut host = WorkspaceHost::default();
        let valid = json!({"x":0,"y":9,"width":12,"height":8});
        for rect in [
            json!({"x":-1,"y":0,"width":1,"height":1}),
            json!({"x":0.5,"y":0,"width":1,"height":1}),
            json!({"x":0,"y":0,"width":1,"height":1,"extra":1}),
            json!({"x":0,"y":0,"width":4294967296u64,"height":1}),
            json!({}),
        ] {
            rejected_place(&mut host, place(0, "1", rect), "invalid_input");
        }
        for field in ["block_id", "expected_revision"] {
            for invalid in [json!(1), json!("01"), json!("18446744073709551616")] {
                let mut request =
                    json!({"api_version":1,"expected_revision":"0","block_id":"1","rect":valid});
                request[field] = invalid;
                rejected_place(&mut host, raw(request), "invalid_input");
            }
        }
        rejected_place(
            &mut host,
            raw(json!({"api_version":2,"expected_revision":"0","block_id":"1","rect":valid})),
            "unsupported_version",
        );
        rejected_place(
            &mut host,
            raw(
                json!({"api_version":1,"expected_revision":"0","block_id":"1","rect":valid,"extra":1}),
            ),
            "invalid_input",
        );
        rejected_place(&mut host,InvokeBody::Raw(br#"{"api_version":1,"expected_revision":"0","block_id":"1","rect":{"x":0,"x":1,"y":9,"width":1,"height":1}}"#.to_vec()),"invalid_input");
        rejected_place(&mut host, InvokeBody::Json(json!({})), "invalid_input");
        rejected_place(&mut host, InvokeBody::Raw(vec![255]), "invalid_input");
        rejected_place(
            &mut host,
            InvokeBody::Raw(vec![b' '; MAX_PAYLOAD + 1]),
            "limit_exceeded",
        );
    }

    #[test]
    fn concurrent_placements_with_one_revision_commit_only_once() {
        use std::sync::{Arc, Mutex};
        let host = Arc::new(Mutex::new(WorkspaceHost::default()));
        let handles: Vec<_> = (8..10)
            .map(|y| {
                let host = Arc::clone(&host);
                std::thread::spawn(move || {
                    value(host.lock().unwrap().place(&place(
                        0,
                        "1",
                        json!({"x":0,"y":y,"width":12,"height":8}),
                    )))
                })
            })
            .collect();
        let results: Vec<_> = handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect();
        assert_eq!(results.iter().filter(|r| r["status"] == "ok").count(), 1);
        assert_eq!(
            results
                .iter()
                .filter(|r| r["error"]["code"] == "stale_revision")
                .count(),
            1
        );
        assert_eq!(host.lock().unwrap().revision, 1);
    }

    #[test]
    fn terminal_binding_hide_reopen_and_minimum_are_placement_only() {
        let mut host = WorkspaceHost::default();
        let first = host.attach_terminal(42, 0.into()).unwrap();
        let snapshot = serde_json::to_value(host.snapshot()).unwrap();
        assert_eq!(snapshot["blocks"][2]["session_id"], "42");
        assert_eq!(host.attach_terminal(42, 1.into()).unwrap(), first);
        assert_eq!(host.revision, 1);
        let id = u64::from(first).to_string();
        rejected_place(
            &mut host,
            place(1, &id, json!({"x":20,"y":0,"width":3,"height":3})),
            "invalid_input",
        );
        assert_eq!(value(host.remove(&remove(1, &id)))["status"], "ok");
        assert!(host.session_bindings.is_empty());
        let second = host.attach_terminal(42, 2.into()).unwrap();
        assert_ne!(first, second);
        assert_eq!(
            serde_json::to_value(host.snapshot()).unwrap()["blocks"][2]["session_id"],
            "42"
        );
    }
}
