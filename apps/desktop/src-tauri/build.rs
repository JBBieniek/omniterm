fn main() {
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(
        tauri_build::AppManifest::new().commands(&[
            "get_layout_snapshot",
            "add_placeholder",
            "remove_placement",
        ]),
    ))
    .expect("failed to build the desktop host");
}
