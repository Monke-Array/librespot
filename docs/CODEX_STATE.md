# Current objective

Spotify `automix-preview` playback is implemented and locally verified on
`codex/m3a-live-auto-metadata`. The candidate is not yet RPI-01 validated.

# Candidate ancestry

- Approved specification: `a39d908`.
- Approved implementation plan: `fb450b5`.
- Typed dealer signal ingress: `24e8cd3`.
- Strict protobuf decode and validation: `18d955f`.
- Shared recipe materialization: `4d97204`.
- Playback preview ownership types: `e2f6c2f`.
- Preview generations and dealer waiters: `b8ac588`.
- Token-owned dual-source loading: `b1071cf`.
- Token-owned secondary PCM: `96b134c`.
- Transition rendering and exact post-roll: `3294cf3`.
- Authority-safe normal playback restoration: `91a5298`.
- SPIRC ingress, preemption, replies, and zero-queue ownership: `beaf559`.
- The diagnostics/state commit containing this file is based on `beaf559`.

# Verified architecture and invariants

- Dealer `Command::Signal(SignalCommand)` carries `automix-preview`; strict
  bounded, presence-aware protobuf decoding rejects malformed, incomplete,
  absolute-start, explicit-stop, non-3000-ms, mismatched-identity, and invalid
  timing/speed requests.
- The evidenced profile is exactly relative 3000 ms: outgoing loading begins at
  `start_a - 3000 ms`, the shared `TransitionPlan` renders the transition, then
  incoming audio renders for exactly 3000 ms of post-roll.
- Preview recipes use the existing Spotify recipe -> deterministic style
  materializer -> `TransitionPlan` -> renderer path. They never enter the live
  queue-edge candidate resolver and never substitute local Auto.
- Preset `NONE` is an intentional immediate no-audio success. Unknown presets,
  unsupported styles, custom overrides, and unsupported DSP reject explicitly.
- `PreviewToken` contains Connect session ID, preview generation, and normal
  ownership generation. The full token travels through source loads, secondary
  decoder ownership, PCM blocks, rendering, terminal events, and restoration.
- Identical requests under the same authority attach a dealer waiter to the
  active generation without restarting audio. Material differences fail old
  waiters and replace the generation. Every terminal path drains waiters once.
- `PlayerInternal` retains normal playback, cancels normal preload/transition
  state, loads preview A/B using canonical plus expected-playable identity,
  seeks both sources to their validated positions, and uses the existing sink.
- Preview B handoff and post-roll are preview-internal. Preview never emits an
  ordinary `TrackChanged`, promotion, or `EndOfTrack`, and SPIRC consumes every
  preview event before ordinary queue handling.
- Normal Connect commands advance normal authority and cancel preview before
  acting. Restore is allowed only while the exact retained authority remains;
  replacement/session/inactive/shutdown ownership discards stale retained state.
- Stale load, PCM, renderer, cancellation, and completion results cannot affect
  a newer preview, restore stale playback, promote a source, or advance the
  authoritative queue.

# Observability

- Structured `[spotify-preview]` records include bounded session/generation
  ownership, sanitized canonical/playable A/B, 12-hex semantic fingerprint,
  field presence, provenance, preset/style IDs, exact transition/load/post-roll
  timing, speeds, cancellation/failure reason, and restore result.
- Every preview terminal record explicitly says `queue_advanced=false` and
  `normal_promotion_emitted=false`; stale terminal events are labeled.
- Raw `preview_parameters` base64/protobuf tracing was removed. Diagnostics do
  not log credentials, account data, raw payloads, or audio.

# Local verification (2026-09-22)

- Focused ingress: 3 passed.
- Focused Connect preview decode/materialization/diagnostics: 18 passed.
- Focused SPIRC ownership/queue regressions: 27 passed.
- Focused playback preview types: 2 passed; player preview lifecycle: 7 passed.
- `cargo fmt --check`: passed.
- `cargo check --workspace`: passed.
- `cargo test -p librespot-playback`: 119 passed.
- `cargo test -p librespot-connect`: 163 unit, 5 integration, and 1 doctest
  passed.
- Both requested package clippy gates passed. Warnings remain for established
  test arithmetic/argument count, large enums (including typed preview command
  payloads), and currently unread internal source-owner fields; no lint failed.
- Runtime diagnostics: 4 passed.
- Spotify Automix oracle fixture validation: passed; 8 pairs, 9/9 hashes,
  107/107 presets, 105/107 exact geometry, and 22/107 exact speed bits. Its
  pre-existing score status remains `not-yet-exact`.
- Spotify Mixer harness: 28 passed.
- `git diff --check`: passed.

# Runtime status and rollback

- Preview playback is not yet RPI-01 validated; audibility, exact live seek
  behavior, restoration, replacement, queue non-advancement, and sink health
  remain runtime claims to verify.
- Previously validated live-transition deployment was
  `94b4a69b435a0e413f93d7a8f993cfdfc87a3db2`, binary SHA256
  `b148f34d2cd0e84d33625c1ef5ba0096f9428420318c4ea33427b690e4f460b5`.
- Preserve known-good rollback
  `/usr/local/bin/spotifyd.rollback-90e1628-pre-b63949c` with SHA256
  `54a8c36d487c5cbc39239fac1dd9f19fa9a50a5fc82695259b2e315d3edd2e9c`.

NEXT ACTION: deploy the exact pushed preview candidate to RPI-01 and perform one
bounded Spotify Mix Preview reproduction with typed lifecycle and sink diagnostics.
