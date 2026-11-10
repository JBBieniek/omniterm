fn main() {
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(
        tauri_build::AppManifest::new().commands(&[
            "get_layout_snapshot",
            "add_placeholder",
            "remove_placement",
            "place_block",
            "terminal_command",
            "get_text_block",
            "git_command",
        ]),
    ))
    .expect("failed to build the desktop host");
}
