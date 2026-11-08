use omniterm_core::{Block, BlockId, GridRect, GridSpec, Workspace};
use omniterm_desktop::snapshot::{LayoutSnapshotV1, WireU64};
use serde_json::json;

#[test]
fn empty_snapshot_projects_the_core_defaults_and_version() {
    let workspace = Workspace::new(GridSpec::default());
    let before = workspace.clone();
    let snapshot = LayoutSnapshotV1::from_workspace(&workspace, 0);
    assert_eq!(
        serde_json::to_value(snapshot).unwrap(),
        json!({
            "api_version": 1,
            "revision": "0",
            "grid": {"columns": 48, "rows": 32, "cell_px": 64},
            "blocks": []
        })
    );
    assert_eq!(workspace, before);
}

#[test]
fn snapshot_preserves_exact_ids_titles_geometry_and_revision() {
    let mut workspace = Workspace::new(GridSpec::new(64, 40, 80).unwrap());
    workspace
        .add(Block::new(
            BlockId(u64::MAX),
            GridRect::new(3, 4, 12, 8).unwrap(),
            "<script>inert title</script>",
        ))
        .unwrap();
    let before = workspace.clone();
    let value =
        serde_json::to_value(LayoutSnapshotV1::from_workspace(&workspace, u64::MAX)).unwrap();
    assert_eq!(value["revision"], "18446744073709551615");
    assert_eq!(value["grid"], json!({"columns":64,"rows":40,"cell_px":80}));
    assert_eq!(value["blocks"][0]["id"], "18446744073709551615");
    assert_eq!(value["blocks"][0]["title"], "<script>inert title</script>");
    assert_eq!(
        value["blocks"][0]["rect"],
        json!({"x":3,"y":4,"width":12,"height":8})
    );
    assert_eq!(workspace, before);
}

#[test]
fn wire_integers_round_trip_without_javascript_precision_loss() {
    for value in [0, 1, 9_007_199_254_740_993, u64::MAX] {
        let wire = WireU64::from(value);
        let json = serde_json::to_string(&wire).unwrap();
        assert_eq!(json, format!("\"{value}\""));
        assert_eq!(serde_json::from_str::<WireU64>(&json).unwrap(), wire);
    }
    for invalid in [
        "\"\"",
        "\"00\"",
        "\"01\"",
        "\"+1\"",
        "\"-1\"",
        "\" 1\"",
        "\"1 \"",
        "\"1.0\"",
        "\"1e2\"",
        "\"18446744073709551616\"",
        "1",
        "null",
    ] {
        assert!(
            serde_json::from_str::<WireU64>(invalid).is_err(),
            "{invalid}"
        );
    }
}
