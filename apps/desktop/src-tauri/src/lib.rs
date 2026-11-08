use omniterm_core::{GridSpec, Workspace};
use tauri::State;

pub mod snapshot;

#[tauri::command]
fn get_layout_snapshot(workspace: State<'_, Workspace>) -> snapshot::LayoutSnapshotV1 {
    // The read-only host cannot mutate layout; revision starts at zero.
    snapshot::LayoutSnapshotV1::from_workspace(&workspace, 0)
}

pub fn run() {
    tauri::Builder::default()
        .manage(Workspace::new(GridSpec::default()))
        .invoke_handler(tauri::generate_handler![get_layout_snapshot])
        .run(tauri::generate_context!())
        .expect("failed to run Omniterm");
}
