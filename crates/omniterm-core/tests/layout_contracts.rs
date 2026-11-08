use omniterm_core::{
    Block, BlockId, Camera, GridError, GridRect, GridSpec, LayoutError, Point, Workspace, Zoom,
    ZOOM_LEVELS,
};

fn rect(x: u32, y: u32, width: u32, height: u32) -> GridRect {
    GridRect::new(x, y, width, height).unwrap()
}

fn near(a: Point, b: Point) {
    assert!((a.x - b.x).abs() < 1e-9, "{a:?} != {b:?}");
    assert!((a.y - b.y).abs() < 1e-9, "{a:?} != {b:?}");
}

fn populated() -> Workspace {
    let mut workspace = Workspace::new(GridSpec::default());
    workspace
        .add(Block::new(BlockId(1), rect(0, 0, 12, 8), "Terminal"))
        .unwrap();
    workspace
        .add(Block::new(BlockId(2), rect(12, 0, 8, 8), "Inspector"))
        .unwrap();
    workspace
}

#[test]
fn invalid_grid_dimensions_are_rejected() {
    for (columns, rows, cell) in [(0, 1, 64), (1, 0, 64), (513, 1, 64), (1, 1, 15)] {
        assert_eq!(
            GridSpec::new(columns, rows, cell),
            Err(GridError::InvalidGrid)
        );
    }
    assert!(GridSpec::new(512, 512, 256).is_ok());
}

#[test]
fn zero_size_rectangles_are_rejected() {
    assert_eq!(GridRect::new(0, 0, 0, 1), Err(GridError::InvalidRect));
    assert_eq!(GridRect::new(0, 0, 1, 0), Err(GridError::InvalidRect));
}

#[test]
fn exact_grid_boundary_is_allowed_but_overflow_is_not() {
    let grid = GridSpec::default();
    assert!(grid.contains(rect(47, 31, 1, 1)));
    assert!(!grid.contains(rect(48, 31, 1, 1)));
    assert!(!grid.contains(rect(u32::MAX, 0, u32::MAX, 1)));
}

#[test]
fn adjacent_rectangles_do_not_overlap() {
    assert!(!rect(0, 0, 8, 8).overlaps(rect(8, 0, 8, 8)));
    assert!(rect(0, 0, 8, 8).overlaps(rect(7, 0, 8, 8)));
}

#[test]
fn collisions_reject_add_without_mutation() {
    let mut workspace = populated();
    let before = workspace.clone();
    assert_eq!(
        workspace.add(Block::new(BlockId(3), rect(1, 1, 2, 2), "Overlap")),
        Err(LayoutError::Collision(BlockId(1)))
    );
    assert_eq!(workspace, before);
}

#[test]
fn duplicate_ids_are_rejected() {
    let mut workspace = populated();
    assert_eq!(
        workspace.add(Block::new(BlockId(1), rect(30, 20, 2, 2), "Duplicate")),
        Err(LayoutError::DuplicateId)
    );
}

#[test]
fn failed_move_is_atomic() {
    let mut workspace = populated();
    let before = workspace.clone();
    assert_eq!(
        workspace.place(BlockId(1), rect(12, 0, 12, 8)),
        Err(LayoutError::Collision(BlockId(2)))
    );
    assert_eq!(workspace, before);
}

#[test]
fn a_block_can_remain_at_its_own_position() {
    let mut workspace = populated();
    assert!(workspace.place(BlockId(1), rect(0, 0, 12, 8)).is_ok());
}

#[test]
fn missing_block_move_is_rejected() {
    assert_eq!(
        populated().place(BlockId(99), rect(0, 10, 2, 2)),
        Err(LayoutError::MissingBlock)
    );
}

#[test]
fn invalid_shrink_is_atomic_and_expand_preserves_rectangles() {
    let mut workspace = populated();
    let before = workspace.clone();
    assert_eq!(
        workspace.reconfigure(GridSpec::new(10, 10, 64).unwrap()),
        Err(LayoutError::OutOfBounds)
    );
    assert_eq!(workspace, before);
    workspace
        .reconfigure(GridSpec::new(96, 64, 64).unwrap())
        .unwrap();
    assert_eq!(workspace.get(BlockId(1)), before.get(BlockId(1)));
}

#[test]
fn removal_only_changes_placements() {
    let mut workspace = populated();
    assert!(workspace.remove(BlockId(1)).is_some());
    assert!(workspace.remove(BlockId(1)).is_none());
    assert_eq!(workspace.blocks().count(), 1);
}

#[test]
fn world_hit_testing_uses_half_open_cells() {
    let grid = GridSpec::default();
    assert_eq!(grid.world_cell(Point::new(63.9, 64.0)), Ok((0, 1)));
    assert!(grid.world_cell(Point::new(3072.0, 0.0)).is_err());
    assert!(grid.world_cell(Point::new(-0.1, 0.0)).is_err());
    assert!(grid.world_cell(Point::new(f64::NAN, 0.0)).is_err());
}

#[test]
fn zoom_levels_are_discrete_and_clamped() {
    assert!(Zoom::from_percent(99).is_err());
    assert_eq!(Zoom::default().step_in().percent(), 125);
    assert_eq!(Zoom::default().step_out().percent(), 67);
    assert_eq!(Zoom::from_percent(25).unwrap().step_out().percent(), 25);
    assert_eq!(Zoom::from_percent(200).unwrap().step_in().percent(), 200);
}

#[test]
fn all_zoom_levels_round_trip_coordinates() {
    for percent in ZOOM_LEVELS {
        let camera = Camera::new(
            Point::new(-100.0, 75.0),
            Zoom::from_percent(percent).unwrap(),
        )
        .unwrap();
        for world in [Point::new(0.0, 0.0), Point::new(1000.0, 250.0)] {
            near(
                camera
                    .screen_to_world(camera.world_to_screen(world).unwrap())
                    .unwrap(),
                world,
            );
        }
    }
}

#[test]
fn zoom_preserves_anchor_and_does_not_mutate_workspace() {
    let workspace = populated();
    let before = workspace.clone();
    let mut camera = Camera::default();
    let anchor = Point::new(640.0, 360.0);
    let world = camera.screen_to_world(anchor).unwrap();
    for percent in ZOOM_LEVELS {
        camera
            .zoom_at(Zoom::from_percent(percent).unwrap(), anchor)
            .unwrap();
        near(camera.world_to_screen(world).unwrap(), anchor);
        assert_eq!(workspace, before);
    }
}

#[test]
fn panning_uses_zoom_scaled_distances() {
    let mut camera = Camera::new(Point::new(0.0, 0.0), Zoom::from_percent(50).unwrap()).unwrap();
    camera.pan_screen(100.0, 50.0).unwrap();
    near(camera.origin(), Point::new(200.0, 100.0));
}

#[test]
fn invalid_camera_changes_are_atomic() {
    assert!(Camera::new(Point::new(f64::INFINITY, 0.0), Zoom::default()).is_err());
    let mut camera = Camera::default();
    let before = camera;
    assert!(camera
        .zoom_at(Zoom::default(), Point::new(f64::NAN, 0.0))
        .is_err());
    assert_eq!(camera, before);
    assert!(camera.pan_screen(f64::INFINITY, 0.0).is_err());
    assert_eq!(camera, before);
}

#[test]
fn nonfinite_transform_inputs_and_overflow_are_rejected() {
    let camera = Camera::default();
    assert!(camera.world_to_screen(Point::new(f64::NAN, 0.0)).is_err());
    assert!(camera
        .screen_to_world(Point::new(f64::INFINITY, 0.0))
        .is_err());
    let large = Camera::new(Point::new(-f64::MAX, 0.0), Zoom::default()).unwrap();
    assert!(large.world_to_screen(Point::new(f64::MAX, 0.0)).is_err());
}

#[test]
fn overlap_is_symmetric_for_a_small_exhaustive_grid() {
    for ax in 0..4 {
        for ay in 0..4 {
            for bx in 0..4 {
                for by in 0..4 {
                    let a = rect(ax, ay, 2, 2);
                    let b = rect(bx, by, 2, 2);
                    assert_eq!(a.overlaps(b), b.overlaps(a));
                }
            }
        }
    }
}
