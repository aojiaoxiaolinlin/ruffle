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

## Android validation and default enablement follow-up

Status: Android has not been tested. Keep `vtable_memory_experiment` disabled by
default until Android validation is completed and its results are recorded here.
The Windows results above do not establish Android compatibility or memory gains.

Before considering default enablement:

- Confirm the Android candidate actually enables the core feature; merely merging
  this commit or testing a default build does not exercise the compact layout.
- Compare baseline and compact builds on the same device, renderer, game assets,
  and comparable play sequence. Include Roco and Kabu, map changes, battles,
  repeated scene transitions, and a sustained session.
- Check touch input, UI interactions, animation, loading, and stability; compare
  memory and responsiveness after equivalent loading and warm-up. Cover both a
  representative lower-memory device and a normal device when available.
- Record device/Android version, renderer, commit IDs, build feature settings,
  test duration, and results. Investigate regressions before changing defaults.

If these checks show no compatibility, responsiveness, or memory regressions,
the compact layout can be proposed as the default in a separate change. A large
process-memory reduction is not required, but a single successful launch is not
enough evidence. Prefer retaining an opt-out during initial rollout; testing
completion alone must not silently change the default.
