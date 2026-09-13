# Current objective

Deploy and validate the resolved-context edge ownership fix after the live
Hypa Hypa -> Y.K.P -> Nash Gimn stale-promotion incident. Keep the bounded
recorder passive and classify any new XRUN independently.

# Branch and deployment

- Branch: `codex/m3a-live-auto-metadata`; latest implementation commit
  `b78e849929a3e57778db46996a34d6abbd32bd53`.
- Playback candidate source commit: `09369c860eb8890603b2a50dcbafc6acd6673c96`.
- The ARM spotifyd snapshot pins all eight librespot dependencies and lockfile
  sources exactly to that commit. Build unit `spotifyd-arm-build-09369c86`
  finished successfully with `--release --locked -j 2`.
- Deployed `/usr/local/bin/spotifyd` SHA256:
  `c2f208c471ced9e2675f5f48fe32b06c0d6d41c0b42cd718b49025ca92edac5d`.
- Rollback `/usr/local/bin/spotifyd.rollback-5448a347-pre-09369c86` SHA256:
  `ee0f561df0bef9bbda122695a73acb5803a95fde52a3c0686e4cd5e5d92ce252`.
- Diagnostic recorder commits `eab442a` and `477c2c9` are separately deployed.
  Installed recorder SHA256:
  `667d8d3df981550397d0120f1b58ed5710a30b5419cb6ab735f8742554eb2715`.
- Spotifyd and `spotifyd-diagnostics.service` are active with zero restarts
  since intentional deployment restarts.

# Architecture and invariants

- Mixer/local-Auto and non-Mixer normal-crossfade routes remain distinct.
- A playing-source seek closes a Running sink before decoder seek or blocking
  read-ahead. It cancels secondary ownership and resets transition state.
- Seek completion leaves the sink temporarily closed. The normal playback poll
  reopens it only when the current source is again available to produce PCM.
- Ready/loading secondary state belongs to its session and generation. Stale
  generations cannot deliver PCM or promote; cancellation retires transition
  DSP and runnable worker state.
- After an asynchronously resolved context is applied, SPIRC compares the
  active `(current, next)` edge with the pre-resolution edge. A changed edge
  cancels old hydration/local-Auto ownership and schedules the authoritative
  next preload; an unchanged edge does not churn the existing preload.
- Promotion occurs once, advances queue ownership through the old request's
  terminal event, preserves the incoming source clock, and retires transition
  DSP before ordinary playback owns the source.
- Mixer/local-Auto queue ownership remains deterministic; networking and ML do
  not control sink, decoder, queue, or transition lifetime.

# Confirmed fixes

- `f51a26b`: playing seeks call `ensure_sink_stopped(true)` before synchronous
  decoder seek/read-ahead. Existing explicit-load promotion behavior is intact.
- `09369c8`: `slow_operation=sink_write` threshold is 250 ms. ALSA periods can
  normally block for about 125 ms, so the old 100 ms threshold logged ordinary
  writes; 250 ms retains detection of stalls lasting at least two periods.
- `eab442a`: a timed-out `vcgencmd` telemetry sample is recorded and skipped
  instead of terminating the recorder.
- `477c2c9`: the recorder persists its journal cursor every 30 seconds,
  immediately on XRUN, and on shutdown. It resumes with `--after-cursor`, so a
  restart cannot replay a recent XRUN as a new incident.
- `b78e849`: resolved context replacement now invalidates stale transition
  ownership and preloads the new authoritative edge.

# Regression and verification evidence

- The seek lifecycle regression failed before the fix because the sink stop
  count was zero inside decoder seek; it passes after the fix.
- It verifies sink stop ordering, secondary/transition cancellation, source
  position, no premature restart, and normal-poll restart with PCM available.
- Existing explicit-load regression remains green.
- Fresh gate for the playback candidate passed:
  `cargo fmt --all -- --check`, `cargo check --workspace --locked`,
  `cargo test -p librespot-playback -p librespot-connect --locked`, targeted
  all-target Clippy with `-D warnings`, and `git diff --check`.
- The resolved-context regression failed before `b78e849` because no replacement
  preload was scheduled. It now covers the Mixer A -> B to A -> C replacement,
  B ownership retirement, C preload scheduling, and unchanged-edge no-churn.
- The paired player regression starts with B ready and armed, replaces it with
  C, and verifies B's generation is retired, transition state is Idle, C owns
  the loader, and B cannot promote.
- Fresh `b78e849` results: playback 96/96; connect 116/116 unit, 5/5 oracle,
  1/1 doctest. Format, workspace check, targeted tests, all-target Clippy with
  the six documented pre-existing lint allowances, and diff check pass.
- Recorder tests are 4/4 and include forced telemetry timeout plus persistent
  XRUN cursor restart coverage; Python byte-compilation and diff checks pass.
- Independent review of `5448a347..09369c86` found no critical, important, or
  minor issue.

# Runtime evidence

- Pre-change active eight-second window: 67 sink-write events and 10,050 text
  bytes at 100 ms; none reached 250 ms. Earlier 15-minute evidence contained
  7,068 sink-write events, all below 150 ms.
- Audible "little stop" reported at 2026-09-13 14:06:08+02:00 is preserved as
  incident `1789301128310568290`. On the old binary, current decoder packet
  production stalled for 4.486 s and 1.447 s while the Pi ARM build ran; each
  stall produced one ALSA Broken-pipe/underrun pair.
- Incident telemetry showed about 1.0 GiB memory available and stable swap, but
  eight blocked tasks and 60-66% I/O wait. Classification: Pi build-induced
  storage I/O starvation, not memory exhaustion and not evidence about either
  deployed playback change. Do not build on RPI-01 while listening.
- Recorder restarts 7 and 8 came from uncaught `vcgencmd get_throttled`
  timeouts. Restart replay also created duplicate incident
  `1789301558842656438` with the same underlying journal cursor. Both recorder
  defects are now fixed and deployed.
- Post-deploy startup/idle windows through 15:32 CEST contain zero XRUN markers,
  zero spotifyd/recorder restarts, and zero sink-write traces. They do not prove
  active-playback rate because no sink-start or Playing event occurred.
- A user-reported skip at 16:00:23 CEST is preserved in
  `/var/lib/spotifyd-diagnostics/manual/1789308023769543909-skip`. Hypa Hypa
  prepared Y.K.P, two `update_context` commands changed Connect's authoritative
  next edge to Nash Gimn, but the already-ready Y.K.P transition still promoted.
  Connect then loaded Nash Gimn about 0.62 seconds later. There was no ALSA XRUN,
  decoder/load failure, or network failure in the incident window. Classification:
  stale transition ownership across asynchronous resolved-context replacement.

# Targeted audit findings

- Transition generation, cancellation, EOF/promotion exclusivity, session
  replacement, queue replacement, and DSP retirement guards are coherent with
  existing regressions. No additional demonstrated transition defect was found.
- Preload TransientService cancels secondary/transition state and preserves the
  Connect queue. Current-source transient failures latch same-track recovery;
  success resumes the URI/position, repeated failure stays latched, and invalid
  session waits for replacement. No concrete local recovery violation was found.
- Ordinary crossfade queue replacement is already fixed by `82d71a2` and covered
  by `queue_replacement_refreshes_preload_outside_mixer`; the previous state-file
  note calling it pending was stale.

# External artifacts

- ARM snapshot: `/home/amogus/.cache/spotifyd-runtime-build-09369c86`.
- ARM target: `/home/amogus/.cache/codex-spotifyd-target-5448a347`.
- Recorder state and incidents: `/var/lib/spotifyd-diagnostics`.
- Preserved context-edge incident:
  `/var/lib/spotifyd-diagnostics/manual/1789308023769543909-skip`.
- Post-deploy journal cursor at 15:22:13 CEST:
  `s=a7721078550c4aad9c2c6e841619427c;i=1d1a7005;b=c372bd5fe0114b7ebdbd81298aa3a7f4;m=13f2690e84;t=65b5d343c72b1;x=772ec61232f1652a`.

# Unresolved issues

- `b78e849` is locally verified but not yet built or deployed on RPI-01; the
  runtime still uses playback source `09369c860eb8890603b2a50dcbafc6acd6673c96`.
- Earlier context-update XRUNs remain unclassified. Neither the seek fix nor
  instrumentation threshold should be credited or blamed without new evidence.

# NEXT ACTION

Push `b78e849`, build one exact ARM candidate pinned to that source revision,
deploy it with rollback/hash verification, then resume passive monitoring.
