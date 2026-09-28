# Current objective

Validate the immediate automix-preview admission reply on RPI-01. It must stop
Spotify's three-second dealer websocket closure, connection prompt, duplicate
signal, and frozen Preview UI without changing token ownership, restoration,
ordinary playback, or live transitions. Effect-family support is separate and
must not begin until this transport fix is runtime-validated.

# Branch and revisions

- Branch: `codex/m3a-live-auto-metadata`.
- Preview regression fixes: `abce5d5`, `bc2d61e`, `ae7a854`.
- Admission-reply implementation: `a8b8110` (acknowledge playable Preview after
  admission and player command enqueue instead of terminal playback).
- Approved preview spec:
  `docs/superpowers/specs/2026-09-21-spotify-automix-preview.md`.
- Regression plan:
  `docs/superpowers/plans/2026-09-26-automix-preview-runtime-regression.md`.

# Architecture and invariants

- SPIRC owns preview admission/authority; the player accepts only the complete
  `PreviewToken` through loaders, PCM, rendering, cancellation, and restoration.
- A playable dealer signal is acknowledged success after validation, admission,
  `SetPreviewAuthority`, and `StartPreview` enqueue. Success means accepted, not
  terminal playback success.
- Terminal Preview events remain internal and never send a second dealer reply.
- Malformed, unsupported, stale, and non-renderable requests still fail before
  player ownership and leave ordinary playback untouched.
- A normal command invalidates active preview ownership immediately and never
  waits for decoder teardown, rendering, restoration, or dealer work.
- Preview never advances the ordinary queue or emits ordinary promotion,
  `TrackChanged`, or `EndOfTrack`; stale terminal work is observational only.

# Confirmed root causes and findings

- Candidate `5891156` put `SetPreviewAuthority` on every ordinary command even
  with no active Preview. `abce5d5` removes that hot-path churn.
- Dealer responder tasks previously survived websocket transport loss.
  `bc2d61e` ends them when their transport closes.
- Duplicate pending Preview signals could grow responder waiters without bound.
  `ae7a854` bounded them; immediate admission replies now keep the active count
  at zero during normal operation.
- The original no-audio attempt used preset 1 with volume style 6 and EQ style
  4 (`Center/Centre bass swap`). Recipe resolution rejected unsupported EQ
  before allocating a token or touching the player.
- With EQ and filter set to `None`, preset 1 style `6/0/0/0/0/0` completed
  audibly and restored normal playback three times. Therefore the playback
  architecture works; unsupported physical EQ caused that no-audio case.
- Every successful Preview retained its dealer reply for the roughly twelve
  second audio lifetime. Spotify closed the websocket about three seconds after
  each signal, retried identical signals, froze the editor progress display,
  and prompted the user to reconnect to the speaker. This proves the terminal
  reply contract is incompatible with the live client.
- Rapid play/pause can grey the Spotify app button while lock-screen controls
  still work. The same symptom reproduced on exact pre-preview `94b4a69`, so it
  is not attributed to Preview.

# Runtime evidence

- RPI-01 currently runs committed candidate `aca9a5f`; deployed binary SHA-256
  `ed258012ffb58c2213d406db7ee3bec137d94dec3cf21082773e39f0872c5049`.
- Preserved fixed binary: `/usr/local/bin/spotifyd.candidate-aca9a5f-ed2580`.
- Preserved baseline: `/usr/local/bin/spotifyd.rollback-94b4a69-pre-5891156`,
  SHA-256 `b148f34d2cd0e84d33625c1ef5ba0096f9428420318c4ea33427b690e4f460b5`.
- Fixed candidate ordinary commands: 25 matched commands, p50 0.504 ms,
  p95 2.205 ms, max 23.875 ms, with no Preview player commands or disconnect.
- It also survived roughly 44 hours with one dealer websocket reset but no SPIRC
  session end or unexpected shutdown.
- Successful Preview samples started/completed at 16:59:42/16:59:54,
  17:00:03/17:00:16, and 17:00:19/17:00:31 on 2026-09-28. Their dealer
  websockets closed at 16:59:45, 17:00:06, and 17:00:22 respectively.

# Local verification

- Admission-reply regression test was observed RED against the terminal-reply
  implementation, then GREEN after the minimal change.
- `cargo test -p librespot-connect`: 168 unit, 5 oracle, 1 doctest passed.
- The remaining full local gate and RPI build/deployment are not yet run for the
  uncommitted admission-reply candidate.

# Effect inventory and unresolved work

- User supplied 8 Volume, 9 EQ, and 11 Filter editor labels; preserved in
  `docs/SPOTIFY_STYLE_CONTRACT.md`.
- Volume-only Smooth crossfade is audible. Exact EQ/filter normalized-control to
  physical DSP mappings remain unresolved; labels are not sufficient to claim
  support or silently approximate them.
- Runtime validation must determine whether immediate success makes the Preview
  bar animate and removes the connection prompt. Do not claim either from unit
  tests.

NEXT ACTION: finish the local gate, commit the admission-reply candidate, build
it on RPI-01 without polling, deploy the exact binary, then ask for one Preview
press and correlate UI behavior with dealer and lifecycle logs.
