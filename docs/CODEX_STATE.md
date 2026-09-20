# Current objective

Spotify Mixer source resolution, recipe hydration, deterministic style lookup,
and `TransitionPlan` adaptation are implemented, locally verified, and deployed
on RPI-01. Live validation covers saved transition hydration, capability-gated
fallback to local Auto, scheduled rendering, promotion, and queue ownership.

# Branch and commits

- Branch: `codex/m3a-live-auto-metadata`.
- Protocol baseline: `a104334`.
- Typed source/edge resolver: `a40b588`.
- Extension-244 envelope hardening: `fd24cf4`.
- Deterministic preset/style resolver: `00739b4`.
- Live SPIRC integration: `01cf0ad`.
- Edge ownership and fallback hardening: `3f6985b`.
- Initial implementation state: `aad0456`.
- Live hydration status correction: `94b4a69`.

# Production architecture

Connect remains authoritative for what plays. Mixer transition data controls
only how the already-authoritative outgoing track becomes the already-
authoritative incoming track. Recipes never choose or advance the queue.

The Mixer source order is now:

1. valid inline saved recipe;
2. saved transition-URI hydration when available;
3. present backend Auto metadata (presence is the current guarded enablement
   signal; no backend fetch or entitlement inference was invented);
4. locally calculated Auto;
5. existing deterministic Mixer safety fallback.

Known preset `NONE` (ID 0) is terminal for the edge and suppresses backend and
local Auto. Missing, malformed, mismatched, unknown, or unrenderable recipes
continue to the next source. Preview provenance cannot authorize live playback.
Non-Mixer contexts retain the independent configurable normal-crossfade route.

The live path consumes the semantic `Transition` recipe. Downstream `audio.*`
materialization remains diagnostic/legacy test coverage and is no longer a
primary live protocol source in SPIRC.

# Ownership and ingress validation

- Every candidate is bound to context URI, authentic outgoing/incoming rows,
  canonical A/B, known playable A/B, item-speed bits, session ID, and edge
  generation.
- Canonical, playable, row, item-speed, session, generation, and exact active
  edge mismatches reject without queue mutation.
- Local Auto jobs and prepared results carry the same ownership. Newly resolved
  playable identities are explicitly bound into that owned edge before work
  starts. Session replacement adopts retained deterministic work into a new
  generation; cancelled/stale async results cannot regain ownership.
- Extension 244 requires one TRANSITION_DATA array/entity, present provider and
  entity headers with HTTP-style status 200, exact entity URI, exact Any type
  URL, and exact playlist/row/A/B data. `latest_transition_uri` is informational
  and cannot replace the requested revision. Missing proto3 headers/default-zero
  fields are not accepted as success.

# Style and plan resolution

- Preset IDs 0..22 map to the pinned six style families from the sanitized
  numeric evidence. Explicit style-ID overrides, including zero, take priority.
- Effective bars clamp to 2..32. Positive BPM is retained; otherwise BPM derives
  from overlap duration/bars when possible, then falls back to 120. Per-side
  item speed is applied once.
- Captured volume families are generated deterministically, including sampled
  bar-dependent Cut/edge geometry. Volume style 8 remains explicitly
  unsupported because its complete algorithm is not proven.
- Official saved/backend plans require every requested family to be renderable.
  Unknown EQ/filter/FX physical mappings, jogwheel, looping, unsupported volume,
  and custom curve overrides while the effective native override gate is unknown
  reject the candidate and continue source fallback.
- Local Auto applies the selected preset's proven volume curve and bounded
  incoming source-clock speed automation. Unsupported optional EQ/filter/FX
  families are retained in the resolved style and logged as omitted; no physical
  DSP mapping was invented.
- Non-unity outgoing overlap speed remains unsupported. Non-unity incoming speed
  is represented by `SpeedAutomation` ending at the exact overlap source-time
  boundary. Existing player generation, PCM readiness, rendering, promotion,
  and queue ownership remain unchanged.

# Observability

Structured debug records include authoritative A/B, source, provenance, preset,
resolved style IDs, bars/BPM, timing/speed, rejection/fallback reasons, omitted
DSP capabilities, session, and transition edge generation. Existing player
runtime traces retain secondary generation and promotion results. Logs contain
no recipe payloads, credentials, account data, or raw audio.

# Verified local gate (2026-09-20)

- `cargo fmt --check`: passed.
- `cargo check --workspace`: passed.
- `cargo test -p librespot-playback`: 99 passed.
- `cargo test -p librespot-connect`: 138 unit + 5 integration + 1 doctest passed.
- `cargo clippy -p librespot-playback --all-targets`: passed with the established
  `int_plus_one`, large-enum, and argument-count warnings.
- `cargo clippy -p librespot-connect --all-targets`: passed with established
  Auto `if_same_then_else`/test float warnings plus inherited playback warnings;
  no new resolver/style/hydration warning remains.
- `node tools/spotify-automix-oracle/validate_input_fixtures.js`: 9/9 beat hashes,
  107/107 presets, 105/107 geometry, 22/107 exact speed bits; known score status
  remains `not-yet-exact`.
- Runtime diagnostics unit tests: 4 passed.
- Spotify Mixer harness tests: 28 passed.
- `git diff --check`: passed.

# Explicit compatibility limit

Full saved Blended preset-2 DSP is not claimed: preset 2 requires the still-
unknown EQ physical mapping. It is decoded, validated, style-resolved, reported
as unsupported, and safely falls through. This is the maximum evidence-backed
production behavior until a physical mapping and its renderer validation gate
exist. The same rule applies to unresolved filter/FX, custom curves, blocks, and
outgoing speed semantics.

# RPI-01 live evidence

- Deployed implementation commit:
  `94b4a69b435a0e413f93d7a8f993cfdfc87a3db2`.
- Deployed binary SHA256:
  `b148f34d2cd0e84d33625c1ef5ba0096f9428420318c4ea33427b690e4f460b5`.
- Exact build snapshot: `/home/amogus/.cache/spotifyd-runtime-build-94b4a69`.
- Known-good rollback binary:
  `/usr/local/bin/spotifyd.rollback-90e1628-pre-b63949c`.
- Rollback SHA256:
  `54a8c36d487c5cbc39239fac1dd9f19fa9a50a5fc82695259b2e315d3edd2e9c`.
- Playback on 2026-09-20 completed three scheduled local-Auto transitions and
  matching promotions (player generations 4, 7, and 8). Each promotion advanced
  the authoritative queue exactly once; one edge retained distinct canonical
  and relinked playable identities. No XRUN, underrun, EPIPE, panic, or stale
  promotion was logged.
- The corrected build hydrated and validated extension 244 for the exact saved
  Blended edge `3eekarcy7kvN4yt5ZFzltW` -> `7ycWLEP1GsNjVvcjawXz3z`
  under session `c6e64d6b7aae493dbb6d8466245290b0`, edge generation 7.
- Saved transition URI
  `spotify:transition:77XHoqQ5xJMsKf5HHXeY7a:1789483453786` decoded as preset
  2. The unsupported EQ physical mapping rejected only that source; local Auto
  then selected preset 1 (`startA=173210`, `startB=9589`, `duration=5000`) and
  logged EQ style 4 as omitted rather than inventing DSP.
- The scheduled plan started and completed, player generation 11 promoted the
  exact incoming playable track at 12122 ms, and SPIRC advanced the authoritative
  queue once from A to B. The subsequent current/next edge was B to the original
  following context row. There were zero XRUN, underrun, EPIPE, panic, audio/sink
  error, or stale-promotion matches in the bounded run.
- Separate live mismatched-edge probes decoded saved data but rejected it because
  returned B did not match the authoritative incoming row, then selected local
  Auto without changing queue ownership.
- RPI-01 remains active with 2.0 GiB swap (1.7 GiB free after the build). Root
  filesystem had 531 MiB free after deployment; build snapshots were retained.

NEXT ACTION: implement an evidence-backed physical EQ mapping and renderer
validation before enabling saved preset-2 DSP; until then retain the verified
capability-gated local-Auto fallback.
