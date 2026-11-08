use omniterm_core::{Block, BlockId, GridRect, GridSpec, Workspace};

#[test]
fn first_fit_is_row_major_and_fills_a_removed_gap() {
    let mut workspace = Workspace::new(GridSpec::new(4, 3, 64).unwrap());
    let first = GridRect::new(0, 0, 2, 2).unwrap();
    workspace
        .add(Block::new(BlockId(1), first, "First"))
        .unwrap();
    let second = workspace.first_fit(2, 2).unwrap();
    assert_eq!(second, GridRect::new(2, 0, 2, 2).unwrap());
    workspace
        .add(Block::new(BlockId(2), second, "Second"))
        .unwrap();
    assert_eq!(
        workspace.first_fit(2, 1),
        Some(GridRect::new(0, 2, 2, 1).unwrap())
    );
    workspace.remove(BlockId(1));
    assert_eq!(workspace.first_fit(2, 2), Some(first));
}

#[test]
fn no_room_or_invalid_size_does_not_change_the_workspace() {
    let mut workspace = Workspace::new(GridSpec::new(2, 2, 64).unwrap());
    workspace
        .add(Block::new(
            BlockId(1),
            GridRect::new(0, 0, 2, 2).unwrap(),
            "Full",
        ))
        .unwrap();
    let before = workspace.clone();
    for (width, height) in [(1, 1), (0, 1), (1, 0), (3, 1), (1, 3), (u32::MAX, u32::MAX)] {
        assert_eq!(workspace.first_fit(width, height), None);
        assert_eq!(workspace, before);
    }
}
