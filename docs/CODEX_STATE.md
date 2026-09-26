# Current objective

Stabilize the `automix-preview` implementation without removing or extending it.
Connect stability and ordinary playback/control correctness take precedence over
audible Preview. Local fixes are complete; fixed-candidate RPI-01 validation is
still required.

# Branch and commits

- Branch: `codex/m3a-live-auto-metadata`.
- Preview candidate investigated: `5891156`.
- Avoid idle player authority churn: `abce5d5`.
- Release dealer responders on transport loss: `bc2d61e`.
- Bound duplicate preview reply waiters: `ae7a854`.
- Approved preview spec:
  `docs/superpowers/specs/2026-09-21-spotify-automix-preview.md`.
- Regression plan:
  `docs/superpowers/plans/2026-09-26-automix-preview-runtime-regression.md`.

# Architecture and invariants

- SPIRC owns preview admission/authority; the player accepts only exact
  `PreviewToken` ownership through loaders, PCM, rendering, and restoration.
- A normal command always advances cheap SPIRC normal ownership. With no active
  preview it sends no player preview command. With an active preview it drains
  waiters and publishes one player-side invalidation before the normal action;
  it never waits for teardown, rendering, restoration, or dealer reply work.
- Malformed, unsupported, and non-renderable requests fail before player
  ownership and leave ordinary playback untouched.
- Preview never emits ordinary queue advancement, promotion, `TrackChanged`, or
  `EndOfTrack`; stale terminal work is observational only.
- Dealer requests run in independent tasks. A pending Preview reply does not
  serialize later dealer commands, and its task now ends when its websocket
  transport closes.
- One identical active Preview retains at most eight reply waiters. Overflow
  fails only the new waiter without replacing the token or audio. Cancellation,
  session change, and shutdown drain admitted waiters.

# Confirmed findings and root causes

- Ordinary-control regression: candidate `5891156` advanced preview authority
  and enqueued `SetPreviewAuthority` for every normal command even when no
  Preview existed. This unnecessary cross-thread work was added on the normal
  hot path. `abce5d5` removes the player publication in the no-preview case
  while retaining immediate ownership invalidation.
- Dealer lifecycle defect: request handling was already concurrent, so one
  pending Preview responder did not block subsequent requests. However, a
  responder task could retain its reply receiver forever after websocket loss.
  `bc2d61e` terminates that task on transport closure. This was a real leak but
  is not proven to have caused the observed disconnects.
- Preview reply growth: identical retries could append unbounded waiters.
  `ae7a854` adds a fixed bound without cancelling/restarting the active Preview.
- No-audio Preview: retained live traces reached signal decode, then failed in
  `resolve_automix_preview` with `UnsupportedRenderer` for presets 1 and 17.
  No token was allocated and no `StartPreview` reached the player. Supporting
  those presets is new feature work and is intentionally out of scope.
- Disconnects were recorded during long candidate intervals containing zero
  Preview signals, including idle periods. Therefore the available evidence
  does not support one shared Preview-request cause for lag, disconnects, and
  no audio. Fixed-candidate runtime observation must determine whether session
  stability improved or whether a separate transport issue remains.
- Established `slow_operation=player_command` records around 620--640 ms also
  occur on the pre-preview baseline and are not evidence for this regression.

# Runtime A/B evidence (2026-09-26)

- Candidate A: exact `5891156` binary SHA-256
  `dfe890c8d8441288b24d91884415144a3486e2b9d946a72e3ebf528462d073d4`.
  Same bounded control sequence produced 46 matched dealer-to-SPIRC commands:
  p50 0.456 ms, p95 463.946 ms, max 954.047 ms. Four `skip_next`
  requests measured 954.05, 762.51, 644.92, and 463.95 ms.
- Baseline B: exact validated `94b4a69` binary SHA-256
  `b148f34d2cd0e84d33625c1ef5ba0096f9428420318c4ea33427b690e4f460b5`.
  The same sequence produced 28 matched commands: p50 0.405 ms, p95 13.402
  ms, max 79.969 ms.
- Neither bounded A nor B window contained a dealer disconnect. Retained
  candidate history contained 18 receive-task drops over three days, including
  no-Preview intervals; causation remains unresolved.
- RPI-01 currently runs the known-good `94b4a69` rollback. Candidate `5891156`
  is preserved at `/usr/local/bin/spotifyd.candidate-5891156-dfe890`.
- Preserve rollback `/usr/local/bin/spotifyd.rollback-94b4a69-pre-5891156` and
  older rollback `/usr/local/bin/spotifyd.rollback-90e1628-pre-b63949c`.

# Local verification (2026-09-26)

- Red/green tests proved all three fixes: no-preview authority outcome,
  responder termination on transport close, and bounded duplicate admission.
- `cargo fmt --all -- --check`: passed.
- `cargo check --workspace`: passed.
- `cargo test -p librespot-playback`: 119 passed.
- `cargo test -p librespot-connect`: 168 unit, 5 oracle, 1 doctest passed.
- `cargo test -p librespot-core dealer::manager::tests`: 2 passed.
- `git diff --check`: passed.
- Existing warnings remain for two unread internal `SourceOwner` fields and a
  future-incompatibility notice in `num-bigint-dig`; no gate failed.

# Unresolved runtime claims

- Fixed-candidate normal command latency, rapid pause/play and seek behavior,
  bounded idle Connect stability, and ordinary live-transition behavior are not
  yet RPI-01 validated.
- Preview is expected to fail cleanly at recipe resolution when Spotify sends
  presets 1 or 17; no audible result should be claimed unless a supported
  recipe is actually observed end-to-end.

NEXT ACTION: build and deploy the exact fixed commit on RPI-01, then validate
ordinary controls, idle Connect stability, and live transitions before pressing
Preview once.
