# Compact AVM2 VTable trait storage

The opt-in `vtable_memory_experiment` feature stores one namespace/property pair
inline per local name in VTable trait maps, instead of two. Additional pairs
remain supported by SmallVec heap storage. Application-domain maps and other
PropertyMap users retain the original two-entry layout.

```powershell
cargo build --release -p ruffle_desktop --features vtable_memory_experiment
```

The feature is disabled by default and does not enable performance diagnostics.
It applies to AVM2 content without detecting game names or URLs. No XML query,
GC, frame scheduling, graphics quality, or renderer backend changes are involved.
Tables remain independently owned; namespace matching and property ordering use
the existing code. Multiple-namespace names can require additional allocations.

## Validation

Both layouts passed the same 121 selected AVM2 regressions covering namespaces,
inheritance, interfaces, slots, metadata, XML, and mouse behavior. Unit tests
check namespace ordering, replacement, mutation, removal, and independent clones
under both inline capacities.

A fixed offline Flash inheritance fixture loaded identical classes and traits.
Its estimated VTable trait payload decreased from 7.69 MiB to 4.67 MiB, including
spilled buffers. An alternating release-mode microbenchmark also showed smaller
payload without a lookup/copy/drop slowdown in that workload; this is not an
application-wide performance guarantee.

Windows desktop testing with Vulkan found no reported click or stutter issues
in Roco baseline/candidate runs and a Kabu candidate compatibility run. Roco's
trait payload lower bound was 180.7 MiB versus 112.0 MiB with nearly equal live
VTable counts. Different loaded assets and run durations prevent attributing the
entire process-memory difference to this change. The Kabu run had no matched
baseline and establishes compatibility, not a measured memory reduction.

These observations support a modest storage optimization, not a promise to
remove multiple GiB of process memory. Android was not exercised in these runs.
