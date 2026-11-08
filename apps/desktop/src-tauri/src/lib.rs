use omniterm_core::{GridSpec, Workspace};
use tauri::State;

pub mod snapshot;

#[tauri::command]
fn get_layout_snapshot(workspace: State<'_, Workspace>) -> snapshot::LayoutSnapshotV1 {
    // The read-only host cannot mutate layout; revision starts at zero.
    snapshot::LayoutSnapshotV1::from_workspace(&workspace, 0)
}

/// Construct the real app; the separate test executable is a second consumer.
pub fn application() -> tauri::Result<tauri::App<tauri::Wry>> {
    tauri::Builder::default()
        .manage(Workspace::new(GridSpec::default()))
        .invoke_handler(tauri::generate_handler![get_layout_snapshot])
        .build(tauri::generate_context!())
}

pub fn run() {
    application()
        .expect("failed to build Omniterm")
        .run(|_, _| {});
}
