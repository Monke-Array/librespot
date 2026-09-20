# Current objective

Spotify Mixer source resolution, recipe hydration, deterministic style lookup,
and `TransitionPlan` adaptation are implemented locally. The exact committed
candidate still needs deployment and one bounded live validation on RPI-01.

# Branch and commits

- Branch: `codex/m3a-live-auto-metadata`.
- Protocol baseline: `a104334`.
- Typed source/edge resolver: `a40b588`.
- Extension-244 envelope hardening: `fd24cf4`.
- Deterministic preset/style resolver: `00739b4`.
- Live SPIRC integration: `01cf0ad`.
- Edge ownership and fallback hardening: `3f6985b`.
- The state-document commit after these entries is part of the exact candidate.

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
- Extension 244 requires one TRANSITION_DATA array/entity, zero provider/entity
  status, exact entity URI, exact Any type URL, and exact playlist/row/A/B data.
  `latest_transition_uri` is informational and cannot replace the requested
  revision.

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

# RPI-01 baseline and next action

- Last verified deployed candidate remains `b63949c10895735f0acd78252aef7d406e14dbf4`.
- Snapshot: `/home/amogus/.cache/spotifyd-runtime-build-b63949c`.
- Known-good rollback binary:
  `/usr/local/bin/spotifyd.rollback-90e1628-pre-b63949c`.
- Deployed known-good SHA256:
  `32e6d39c07aeb2a55bfa5fb247f99e5d155a95aeb7e5143541248bc8d00d4af3`.
- Rollback SHA256:
  `54a8c36d487c5cbc39239fac1dd9f19fa9a50a5fc82695259b2e315d3edd2e9c`.

NEXT ACTION: push the clean final commit, deploy that exact revision to RPI-01
without replacing the rollback binary, start bounded diagnostics, then request
one user playback reproduction and correlate saved/Blended rejection/fallback,
local Auto selection, deterministic fallback, queue/promotion ownership, and
ALSA/XRUN evidence.
