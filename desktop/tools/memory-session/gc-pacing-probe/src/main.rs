use gc_arena::{
    Arena, Collect, Gc, Rootable, arena::CollectionPhase, lock::RefLock, metrics::Pacing,
};
use std::time::Instant;
#[derive(Collect)]
#[collect(no_drop)]
struct Node<'gc> {
    next: Option<Gc<'gc, Node<'gc>>>,
    payload: [u8; 128],
}
#[derive(Collect)]
#[collect(no_drop)]
struct State<'gc> {
    retained: Option<Gc<'gc, Node<'gc>>>,
    current: Option<Gc<'gc, Node<'gc>>>,
}

fn run(name: &str, floor: bool, sleep: f64) {
    let mut arena = Arena::<Rootable![Gc<'_, RefLock<State<'_>>>]>::new(|mc| {
        let mut retained = None;
        for _ in 0..600_000 {
            retained = Some(Gc::new(
                mc,
                Node {
                    next: retained,
                    payload: [0; 128],
                },
            ));
        }
        Gc::new(
            mc,
            RefLock::new(State {
                retained,
                current: None,
            }),
        )
    });
    arena.metrics().set_pacing(Pacing {
        sleep_factor: sleep,
        ..Pacing::DEFAULT
    });
    arena.finish_cycle();
    let mut peak = arena.metrics().total_gc_count();
    let mut phase = arena.collection_phase();
    let mut phase_start = 0;
    let mut transitions = Vec::new();
    let mut samples = Vec::new();
    let started = Instant::now();
    for tick in 0..18_000 {
        arena.mutate(|mc, root| {
            let mut state = root.borrow_mut(mc);
            for _ in 0..54 {
                state.current = Some(Gc::new(
                    mc,
                    Node {
                        next: None,
                        payload: [0; 128],
                    },
                ));
            }
        });
        peak = peak.max(arena.metrics().total_gc_count());
        // 60 logical updates/sec; grant once per 50ms. Units are NOT bytes.
        if floor && tick % 3 == 0 && arena.collection_phase() != CollectionPhase::Sleeping {
            let debt = arena.metrics().allocation_debt();
            if debt < 2000.0 {
                arena.metrics().adjust_debt(2000.0 - debt);
            }
        }
        let before = Instant::now();
        arena.collect_debt();
        samples.push(before.elapsed().as_nanos());
        let next = arena.collection_phase();
        if next != phase {
            if transitions.len() < 10 {
                transitions.push(format!(
                    "{phase:?}:{:.2}s->{next:?}",
                    (tick - phase_start) as f64 / 60.0
                ));
            }
            phase = next;
            phase_start = tick;
        }
    }
    let total_ms = samples.iter().sum::<u128>() as f64 / 1e6;
    // All policies must preserve the same reachable graph; lower counts must
    // come from garbage reclamation, never losing the retained objects.
    arena.mutate(|_, root| {
        let state = root.borrow();
        let mut node = state.retained;
        let mut count = 0;
        while let Some(value) = node {
            assert_eq!(value.payload, [0; 128]);
            count += 1;
            node = value.next;
        }
        assert_eq!(count, 600_000);
        assert!(state.current.is_some());
    });
    samples.sort_unstable();
    println!(
        "policy={name} peak={} final={} gc_total_ms={total_ms:.3} p95_ms={:.6} max_ms={:.3} wall_ms={:.1}",
        peak,
        arena.metrics().total_gc_count(),
        samples[samples.len() * 95 / 100] as f64 / 1e6,
        *samples.last().unwrap() as f64 / 1e6,
        started.elapsed().as_secs_f64() * 1000.0
    );
    println!("transitions={}", transitions.join(", "));
}
fn main() {
    run("default", false, 0.5);
    run("active-floor-2000-per-50ms", true, 0.5);
    run("short-sleep-and-active-floor", true, 0.2);
}
