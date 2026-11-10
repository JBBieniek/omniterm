use commands::{MutationResult, WorkspaceHost};
use std::sync::Mutex;
use tauri::ipc::Request;
use tauri::State;
use tauri::{Emitter, Manager};

pub mod commands;
pub mod git_task;
pub mod open_call;
pub mod snapshot;
pub mod terminal;
mod terminal_commands;

#[tauri::command]
async fn git_command(
    app: tauri::AppHandle,
    request: Request<'_>,
) -> Result<serde_json::Value, String> {
    let body = request.body().clone();
    tauri::async_runtime::spawn_blocking(move || git_task::dispatch(&app, &body))
        .await
        .map_err(|_| "Git command worker failed.")?
}

#[tauri::command]
fn get_text_block(
    request: Request<'_>,
    workspace: State<'_, Mutex<WorkspaceHost>>,
) -> Result<open_call::TextDocument, commands::CommandError> {
    workspace
        .lock()
        .map_err(|_| commands::CommandError::new("internal_error", "Workspace unavailable."))?
        .text_block(request.body())
}

#[tauri::command]
async fn terminal_command(
    app: tauri::AppHandle,
    request: Request<'_>,
) -> Result<terminal_commands::Reply, &'static str> {
    let body = request.body().clone();
    tauri::async_runtime::spawn_blocking(move || terminal_commands::dispatch(&app, &body))
        .await
        .map_err(|_| "Terminal command worker failed.")
}

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

#[tauri::command]
fn place_block(
    request: Request<'_>,
    workspace: State<'_, Mutex<WorkspaceHost>>,
) -> Result<MutationResult, &'static str> {
    Ok(workspace
        .lock()
        .map_err(|_| "Workspace unavailable.")?
        .place(request.body()))
}

/// Construct the real app; the separate test executable is a second consumer.
pub fn application() -> tauri::Result<tauri::App<tauri::Wry>> {
    tauri::Builder::default()
        .manage(Mutex::new(WorkspaceHost::default()))
        .manage(git_task::GitHost::default())
        .manage(terminal::TerminalHost::new(
            std::env::current_dir().unwrap_or_default(),
        ))
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if window.state::<terminal::TerminalHost>().has_live()
                    || window.state::<git_task::GitHost>().has_live()
                {
                    api.prevent_close();
                    let _ = window.emit("request-quit", ());
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_layout_snapshot,
            add_placeholder,
            remove_placement,
            place_block,
            terminal_command,
            get_text_block,
            git_command
        ])
        .build(tauri::generate_context!())
}

pub fn run() {
    run_app(application().expect("failed to build Omniterm"));
}

/// Common runtime shutdown applies to production and the separate native host.
pub fn run_app(app: tauri::App<tauri::Wry>) {
    app.run(|app, event| {
        if let tauri::RunEvent::ExitRequested { api, .. } = event {
            if let Err(message) = app.state::<git_task::GitHost>().shutdown() {
                api.prevent_exit();
                let _ = app.emit("quit-failed", message);
            }
            if let Err(message) = app.state::<terminal::TerminalHost>().shutdown() {
                api.prevent_exit();
                let _ = app.emit("quit-failed", message);
            }
        }
    });
}
