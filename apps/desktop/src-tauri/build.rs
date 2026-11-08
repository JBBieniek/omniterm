fn main() {
    tauri_build::try_build(
        tauri_build::Attributes::new()
            .app_manifest(tauri_build::AppManifest::new().commands(&["get_layout_snapshot"])),
    )
    .expect("failed to build the desktop host");
}
