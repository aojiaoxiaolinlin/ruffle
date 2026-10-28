# GC pacing model

Pinned to gc-arena 0.7.0, the registry version in the player's Cargo.lock.

```powershell
cargo run --release --locked --offline --manifest-path desktop/tools/memory-session/gc-pacing-probe/Cargo.toml --target-dir target/gc-pacing-probe-build
```

The model retains a traced chain of 600,000 GC nodes and one current temporary
node. It allocates 54 temporary nodes per logical update, for 18,000 updates at
a simulated 60 updates/sec (300 logical seconds, 3240 allocations/sec). There
are no sleeps, game assets, network requests, rendering, or actual game frames.
Every policy verifies that all 600,000 retained nodes remain accessible.

Three policies run with the same reachable graph:

- Default gc-arena pacing, collection driven only by allocation debt.
- The same sleep pacing, with a minimum debt of 2000 work units once per 50
  logical milliseconds while a collection is active.
- The same work floor plus sleep_factor=0.2, a separate exploratory model.

Only the second policy is exposed by the current diagnostic player experiment.
Work units are object-count pacing operations, not bytes or elapsed time. A
quota does not place a hard wall-time bound on an individual trace/drop call.
Timing reports measure native CPU wall time on this synthetic graph. They are
not measured game memory savings or guarantees about game pauses.
