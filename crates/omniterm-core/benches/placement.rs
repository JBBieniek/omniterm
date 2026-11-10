use omniterm_core::{Block, BlockId, GridRect, GridSpec, LayoutError, Workspace};
use std::hint::black_box;
use std::time::{Duration, Instant};

fn rect(x: u32, y: u32, w: u32, h: u32) -> GridRect {
    GridRect::new(x, y, w, h).unwrap()
}

// Setup is outside timing. Values are per-batch means, not individual-call tails.
fn measure(id: &str, blocks: usize, mut operation: impl FnMut()) {
    operation();
    let mut iterations = 1_u32;
    loop {
        let began = Instant::now();
        for _ in 0..iterations {
            operation();
        }
        if began.elapsed() >= Duration::from_millis(10) || iterations >= 16_777_216 {
            break;
        }
        iterations *= 2;
    }
    let samples: Vec<_> = (0..12)
        .map(|_| {
            let began = Instant::now();
            for _ in 0..iterations {
                operation();
            }
            began.elapsed().as_secs_f64() * 1e9 / f64::from(iterations)
        })
        .collect();
    println!("OMNITERM_BENCH {{\"id\":\"{id}\",\"revision\":1,\"blocks\":{blocks},\"iterations_per_batch\":{iterations},\"batch_mean_ns\":{samples:?}}}");
}

fn first_fit(id: &str, workspace: Workspace, width: u32, height: u32, expected: Option<GridRect>) {
    assert_eq!(workspace.first_fit(width, height), expected);
    measure(id, workspace.blocks().count(), || {
        black_box(black_box(&workspace).first_fit(black_box(width), black_box(height)));
    });
    assert_eq!(workspace.first_fit(width, height), expected);
}

fn main() {
    // The normal debug test run checks fixtures once, without timing debug code.
    let fixtures_only = cfg!(debug_assertions) || std::env::args().any(|arg| arg == "--test");
    let mut sparse = Workspace::new(GridSpec::default());
    for i in 0..3 {
        sparse
            .add(Block::new(
                BlockId(i + 1),
                rect(i as u32 * 12, 0, 12, 8),
                "",
            ))
            .unwrap();
    }
    let mut packed = Workspace::new(GridSpec::default());
    for i in 0..16 {
        packed
            .add(Block::new(
                BlockId(i + 1),
                rect(i as u32 % 4 * 12, i as u32 / 4 * 8, 12, 8),
                "",
            ))
            .unwrap();
    }
    let mut fragmented = Workspace::new(GridSpec::new(32, 32, 64).unwrap());
    let mut maximum = Workspace::new(GridSpec::new(512, 512, 64).unwrap());
    for i in 0..256 {
        fragmented
            .add(Block::new(
                BlockId(i + 1),
                rect(i as u32 % 16 * 2, i as u32 / 16 * 2, 1, 1),
                "",
            ))
            .unwrap();
        maximum
            .add(Block::new(
                BlockId(i + 1),
                rect(i as u32 * 2, 0, 2, 512),
                "",
            ))
            .unwrap();
    }
    assert_eq!(sparse.first_fit(12, 8), Some(rect(36, 0, 12, 8)));
    assert_eq!(packed.first_fit(12, 8), None);
    assert_eq!(fragmented.first_fit(2, 2), None);
    assert_eq!(maximum.first_fit(12, 8), None);
    assert_eq!(
        maximum.place(BlockId(1), rect(510, 0, 2, 512)),
        Err(LayoutError::Collision(BlockId(256)))
    );
    maximum.place(BlockId(1), rect(0, 0, 2, 512)).unwrap();
    if fixtures_only {
        return;
    }
    first_fit(
        "B-PLACE-01",
        Workspace::new(GridSpec::default()),
        12,
        8,
        Some(rect(0, 0, 12, 8)),
    );
    first_fit("B-PLACE-02", sparse, 12, 8, Some(rect(36, 0, 12, 8)));
    first_fit("B-PLACE-03", packed, 12, 8, None);
    first_fit("B-PLACE-04", fragmented, 2, 2, None);
    first_fit("B-PLACE-05", maximum.clone(), 12, 8, None);
    measure("B-PLACE-06", 256, || {
        black_box(maximum.place(black_box(BlockId(1)), black_box(rect(0, 0, 2, 512)))).unwrap();
    });
    measure("B-PLACE-07", 256, || {
        black_box(maximum.place(black_box(BlockId(1)), black_box(rect(510, 0, 2, 512))))
            .unwrap_err();
    });
}
