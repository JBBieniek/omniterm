use omniterm_core::grid::{Camera, Point, Zoom};
use serde_json::Value;

fn point(value: &Value) -> Point {
    Point::new(value[0].as_f64().unwrap(), value[1].as_f64().unwrap())
}

fn zoom(value: &Value) -> Zoom {
    Zoom::from_percent(value.as_u64().unwrap() as u16).unwrap()
}

fn near(actual: Point, expected: Point) {
    assert!((actual.x - expected.x).abs() < 1e-9);
    assert!((actual.y - expected.y).abs() < 1e-9);
}

#[test]
fn frontend_and_rust_share_transform_and_anchor_fixtures() {
    let cases: Value = serde_json::from_str(include_str!("../../tests/camera-cases.json")).unwrap();
    for case in cases["world_to_screen"].as_array().unwrap() {
        let camera = Camera::new(point(&case["origin"]), zoom(&case["zoom"])).unwrap();
        near(
            camera.world_to_screen(point(&case["world"])).unwrap(),
            point(&case["screen"]),
        );
        near(
            camera.screen_to_world(point(&case["screen"])).unwrap(),
            point(&case["world"]),
        );
    }
    for case in cases["zoom_at"].as_array().unwrap() {
        let mut camera = Camera::new(point(&case["origin"]), zoom(&case["old_zoom"])).unwrap();
        camera
            .zoom_at(zoom(&case["new_zoom"]), point(&case["anchor"]))
            .unwrap();
        near(camera.origin(), point(&case["new_origin"]));
    }
}
