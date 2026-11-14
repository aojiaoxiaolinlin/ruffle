# Initial frame scripts and Loader completion

Nested frame processing can reach exitFrame while a newly loaded movie still has its initial frame script queued. Publishing Loader INIT/COMPLETE at that point lets consumers read uninitialized data and unload the movie before initialization.

MovieClip now retains the load until a pending initial script has run. Movies without pending scripts and loads that have already published INIT keep their existing behavior. This applies to the shared core, without game-specific playback workarounds.

The original Roco report reproduced missing map and combat BGM. The captured failure had `new_32_fla::MainTimeline` at frame 1 with `last_script_frame=None` and `pending=true`, followed by a caught #1009 accessing `soundConfig.sounds`. A candidate desktop build with the guard completed 37 map loads and 13 combat starts with no audio configuration exceptions; the user reported normal audio. Loader regressions: 52 passed, 2 ignored. Goto regressions: 71 passed. Android runtime validation remains separate.
