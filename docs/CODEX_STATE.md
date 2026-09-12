# Current objective

Make the existing live transition runtime correct and safe for an RPI-01 build.
This is bug-fix-only work: no new transition family, generator/ranking change,
blind-test UI, lossless work, or transition-quality tuning.

# Branch / baseline

- Branch: `codex/m3a-live-auto-metadata`.
- RPI-01 deployed baseline at session start:
  `af58ae155c33c299aa56b7844b4337eb2abe96e3`.
- The working tree was clean at that baseline before this task.
- RPI-01 was not edited or deployed during this task; U-01 remains source of
  truth.

# Architecture and invariants

- Mixer/local-Auto and non-Mixer normal-crossfade routes remain distinct.
- Incoming speed automation uses absolute source-track time. Transition overlap
  duration uses wall-clock time and must be converted before placing its unity
  endpoint.
- `SecondaryDecodeWorker` owns transition time stretching while preloaded.
- Completion, retained-source cancellation, or worker-backed promotion must
  retire time stretching synchronously and non-blockingly before ordinary
  playback owns the source.
- Promotion retains decoded raw PCM and the nominal source clock; it does not
  reload the promoted track or join the decoder thread.
- Source normalization is applied once before mixing. Master volume and dynamic
  limiting are applied once after mixing.

# Confirmed root causes and fixes

- P0 persistent DSP leak: local Auto emitted one non-unity incoming speed point.
  `SpeedAutomation::speed_at()` correctly holds the latest point indefinitely,
  so the promoted worker retained non-unity WSOLA for the rest of the track.
- Fix: local Auto now adds a 1.0 point at
  `start_b + source_duration_for_wall_time(start_b, overlap_wall_duration)`.
  For the captured oracle this is 8263.003226995468 ms, not
  `start_b + overlap_duration`.
- P0 ownership leak: the same worker/time-stretch processor was promoted or
  retained after transition cancellation without any retirement boundary.
- Fix: completion and every cancellation path that retains the preload call one
  centralized retirement path. It immediately removes WSOLA, preserves bounded
  decoded raw lookahead, hands off at the nominal automation clock, and chooses
  only a correlation-window-equivalent raw grain to avoid a large waveform
  step. No worker join, decoder reload, or blocking decode occurs.
- P1 session-replacement leak: a ready secondary worker, unlike a loading
  preload, survived replacement of its invalid owning Spotify session. It could
  retain transition DSP and continue decoding through obsolete session state.
- Fix: replacement now invalidates both loading and ready secondary preloads,
  advances their generation, drops any worker, and restarts the same track and
  transition preload on the replacement session.
- The leaked processor explains persistent speed/pitch and can alter apparent
  level beyond the transition. Independent tests found no persistent gain-curve
  or per-source-normalization leak.

# Loudness findings

- A promoted source and the same normally loaded source compute bit-identical
  Basic/Track normalization factors and identical normalized PCM.
- `TransitionEngine::complete()` and cancellation reset gain curves/spec/frame
  state; later single-source PCM is exact passthrough.
- Dynamic normalization/limiter state is intentionally global ordinary output
  DSP. A transition peak decays back to ordinary gain; it is bounded and is not
  transition gain automation.
- Master volume is not applied twice: source normalization precedes mixing and
  global volume/limiting follows it.

# Regression coverage

- Local-Auto speed has a mathematically correct unity endpoint in source time.
- Bounded speed changes duration only in its region and preserves post-region
  pitch.
- Promotion immediately removes time stretching, retains raw PCM, and keeps
  unity source-position increments.
- Raw handoff before first emission is lossless; active handoff is bounded and
  avoids a large waveform discontinuity.
- Cancellation before promotion, active manual-next promotion, current EOF,
  seek/reload, session replacement, and dropped-secondary cancellation do not
  retain speed DSP.
- Promoted and normally loaded normalization/gain behavior match.
- Completed transition gain curves cannot affect later PCM.
- Normal crossfade/no-speed behavior continues to use the existing path.

# Related audit

- Existing deterministic tests cover transition state progression and illegal
  operations, exact frame completion, mid-packet starts, unaligned PCM,
  underrun, current/secondary EOF and decoder errors, worker backpressure and
  cancellation, stale generations, promotion without double load, seek/load/
  stop, pause/resume, recovery, session replacement, and normal crossfade.
- Connect tests cover stale local-Auto results, context/edge identity, official
  transition precedence, repeated preload, canonical/playable identities,
  malformed metadata, unsupported DSP fail-closed behavior, and queue terminal
  event de-duplication.
- No additional provable deployment bug beyond the three fixes above was found
  in these audited boundaries.

# Verification

- Pre-change baseline: playback 83/83 passed; connect 113 unit + 5 oracle + 1
  doctest passed.
- Current playback suite: 93/93 unit tests and doctests passed.
- Current connect suite: 113 unit + 5 oracle + 1 doctest passed.
- `cargo check --workspace` passed.
- Playback/connect all-target Clippy passed with `-D warnings` after explicitly
  allowing six verified pre-existing lints (`int-plus-one`,
  `too-many-arguments`, `large-enum-variant`, `if-same-then-else`,
  `excessive-precision`, and `type-complexity`).
- Fresh final `cargo fmt --all -- --check`, playback, and connect suite runs
  passed immediately before the deployable commit gate.

# Unresolved issues

- No confirmed transition-runtime correctness issue from this session remains.
- Live audible confirmation on RPI-01 is still a post-build deployment gate,
  not a source-code blocker.
- The RPI-01 spotifyd build needs at least 2 GB swap available; preserve that
  requirement when executing the next action.

# NEXT ACTION

Ensure RPI-01 has at least 2 GB swap, build spotifyd with all eight librespot
crates pinned to the final commit, deploy that exact binary, and run one logged
live transition repro.
