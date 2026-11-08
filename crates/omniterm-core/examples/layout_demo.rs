use omniterm_core::{Block, BlockId, Camera, GridRect, GridSpec, Point, Workspace};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut workspace = Workspace::new(GridSpec::default());
    workspace.add(Block::new(
        BlockId(1),
        GridRect::new(0, 0, 12, 8)?,
        "Future terminal",
    ))?;
    workspace.add(Block::new(
        BlockId(2),
        GridRect::new(12, 0, 8, 8)?,
        "Future task summary",
    ))?;
    let mut camera = Camera::default();
    camera.zoom_at(camera.zoom().step_out(), Point::new(640.0, 360.0))?;
    for block in workspace.blocks() {
        println!("{}: {:?}", block.title(), block.rect());
    }
    println!(
        "Zoom {}%, camera {:?}; this is a model demo, not a desktop UI.",
        camera.zoom().percent(),
        camera.origin()
    );
    Ok(())
}
