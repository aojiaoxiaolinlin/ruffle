//! Standalone offline fixture: no game, network, or third-party dependencies.
use std::{
    backtrace::Backtrace,
    fs,
    hint::black_box,
    path::Path,
    thread,
    time::{Duration, Instant},
};

const BLOCK: usize = 256 * 1024;

#[inline(never)]
fn retained_allocation_site() -> Vec<Vec<u8>> {
    (0..8).map(|_| vec![0x5a; BLOCK]).collect()
}

#[inline(never)]
fn transient_allocation_site() -> Vec<Vec<u8>> {
    (0..64).map(|_| vec![0xa5; BLOCK]).collect()
}

#[inline(never)]
fn symbol_probe_site() -> String {
    Backtrace::force_capture().to_string()
}

fn wait(directory: &Path, command: &str, automatic: bool) {
    if automatic {
        return;
    }
    let deadline = Instant::now() + Duration::from_secs(90);
    while !directory.join(command).exists() {
        assert!(
            Instant::now() < deadline,
            "fixture command timed out: {command}"
        );
        thread::sleep(Duration::from_millis(50));
    }
}

fn mark(directory: &Path, name: &str, bytes: usize) {
    fs::write(
        directory.join(name),
        format!("owned_payload_bytes={bytes}\n"),
    )
    .unwrap();
    println!("{name}: owned_payload_bytes={bytes}");
}

fn main() {
    let args: Vec<_> = std::env::args().collect();
    let directory = Path::new(args.get(1).expect("output directory required"));
    let automatic = args.get(2).is_some_and(|arg| arg == "--self-test");
    fs::create_dir_all(directory).unwrap();
    fs::write(directory.join("symbol-probe.txt"), symbol_probe_site()).unwrap();
    mark(directory, "baseline.ready", 0);
    wait(directory, "allocate.command", automatic);
    let retained = retained_allocation_site();
    let transient = transient_allocation_site();
    black_box((&retained, &transient));
    let retained_bytes = retained.iter().map(Vec::len).sum::<usize>();
    let transient_bytes = transient.iter().map(Vec::len).sum::<usize>();
    assert_eq!(retained_bytes, 2 * 1024 * 1024);
    assert_eq!(transient_bytes, 16 * 1024 * 1024);
    mark(
        directory,
        "allocated.ready",
        retained_bytes + transient_bytes,
    );
    wait(directory, "release.command", automatic);
    drop(transient);
    black_box(&retained);
    mark(directory, "released.ready", retained_bytes);
    wait(directory, "exit.command", automatic);
    drop(retained);
    mark(directory, "finished.ready", 0);
}
