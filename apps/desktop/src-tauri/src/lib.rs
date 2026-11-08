use commands::{MutationResult, WorkspaceHost};
use std::sync::Mutex;
use tauri::ipc::Request;
use tauri::State;

pub mod commands;
pub mod snapshot;

#[tauri::command]
fn get_layout_snapshot(
    workspace: State<'_, Mutex<WorkspaceHost>>,
) -> Result<snapshot::LayoutSnapshotV1, &'static str> {
    Ok(workspace
        .lock()
        .map_err(|_| "Workspace unavailable.")?
        .snapshot())
}

#[tauri::command]
fn add_placeholder(
    request: Request<'_>,
    workspace: State<'_, Mutex<WorkspaceHost>>,
) -> Result<MutationResult, &'static str> {
    Ok(workspace
        .lock()
        .map_err(|_| "Workspace unavailable.")?
        .add(request.body()))
}

#[tauri::command]
fn remove_placement(
    request: Request<'_>,
    workspace: State<'_, Mutex<WorkspaceHost>>,
) -> Result<MutationResult, &'static str> {
    Ok(workspace
        .lock()
        .map_err(|_| "Workspace unavailable.")?
        .remove(request.body()))
}

/// Construct the real app; the separate test executable is a second consumer.
pub fn application() -> tauri::Result<tauri::App<tauri::Wry>> {
    tauri::Builder::default()
        .manage(Mutex::new(WorkspaceHost::default()))
        .invoke_handler(tauri::generate_handler![
            get_layout_snapshot,
            add_placeholder,
            remove_placement
        ])
        .build(tauri::generate_context!())
}

pub fn run() {
    application()
        .expect("failed to build Omniterm")
        .run(|_, _| {});
}
