Regression for synchronous reentry into `Loader.unload()` from its own
`unload` or content `removedFromStage` listener. Real game scene disposal
recursed until the Android native thread stack overflowed.

The callback recursion is capped at three calls so the unfixed engine
produces a trace mismatch rather than crashing the test process. Each phase
reloads the same loader, testing that the reentry guard clears afterward;
the third phase performs an ordinary unload. Repeated empty unloads must
remain harmless. Expected output describes these invariants; it has not
been recorded from a reference Flash Player.

`child.swf` is a generated empty AS3 movie (SWF 11, one frame, 24 fps).
