# Current objective

Spotify Mixer protocol handoff is checkpointed in
`docs/SPOTIFY_LIVE_RECIPE_PROTOCOL.md` and `docs/SPOTIFY_STYLE_CONTRACT.md`.
No production playback integration was changed. Full audible Spotify DSP parity
is not established; explicit release gates remain for physical EQ/filter/FX
mappings, some speed/override configuration, and outer crossfade policy.

# Branch and work

- Branch `codex/m3a-live-auto-metadata`; investigation baseline `e9585c9`.
- This documentation/evidence change records September15-19 captures and native
  code findings for official Windows1.3.0.277, updated September20.
- Native DLL SHA256:
  `65c131dc7f9dcce90eb3874033e6e22ef422a493f0877e3f031805198839b7c4`.
- Clean protocol/docs/evidence commit is intended for origin on this branch.
  Use Git for its final commit identity; do not infer runtime deployment from it.

# Architecture and invariants

- Connect owns what plays. Recipe/Auto/style data controls only how authoritative
  A becomes authoritative B. It cannot choose B or advance the queue.
- Validate context/rows/canonical and playable identity, revision, item speeds,
  session and edge generation. Stale async results cannot regain ownership.
- Mixer/Auto and normal configurable crossfade remain separate runtime routes.
- Current source, secondary PCM/decoder, rendering and promotion retain their
  existing generation/ownership rules; deterministic fallback remains available.

# Verified protocol findings

- Saved `automix.auto_transition_recipe` can contain Blended content despite its
  name. Extension244 hydration already works; tested preset2 failed specifically
  because preset/style rendering is unsupported, not because metadata was absent.
- Native candidate loops order saved recipe, enabled nonempty backend recipe,
  then empty-input local Auto. A local computed-preset marker keeps a result
  provisional while later candidates are tried; accepted stored content stops.
- Known preset0 NONE returns no Automix result and stops a nonempty candidate.
  Unknown/mismatched recipe falls through to local Auto; these are different.
- Backend enablement is conditional. No backend field was observed in the
  captured60 official row observations or236 RPI inventories. This is not proof
  about account entitlement or global availability. No backend fetch is invented.
- Saved/local results converge on native materializer1040aa4; serializers1049874
  and104a158 and node writers810f90/8110ec produce downstream `audio.*` metadata.
  XPUI queue output is not evidence that these attributes came from a backend.
- Complete current lookup wire schemas, preset0..22 mappings, style IDs and
 69 numeric diagnostic request/response fixtures are durable under
  `tools/runtime-diagnostics/evidence/2026-09-19-*.json`.
- FX request fields are IDs1/BPM2/bars3. Materializer clamps bars2..32. Curves
  can depend on bars; sampled FX wet/dry envelopes do not establish BPM-independent
  effect timing. Native curve conversion ignores envelope minimum/maximum.
- Curve-override gate is `core-automix/auto_transition_use_curve_overrides`;
  current accessor defaults false. Effective account override remains unknown.
- Standard preset mappings use no jogwheel/looping; explicit nonzero overrides
  should initially be rejected as unsupported, not silently ignored.

# Evidence limits and safe handoff

- Native source ordering is code-derived; no live backend collision experiment
  exists. The strongest live result is same-pair saved recipe ingress on RPI
  versus downstream materialization on official Windows.
- September15 snapshots overlap cumulatively. September19 user preview activity
  is a separate multi-pair session, not a controlled natural live transition.
- Desktop was restarted with local diagnostics September19; native log persisted.
  Large stage export hit ENOBUFS; no successful new full stage export is claimed.
  Debug port subsequently became unavailable; no further restart was performed.
- Safe Sol start: typed source/outcome/ownership contract, ingress hardening,
  deterministic fixture-backed style resolver, lossless render-plan representation.
- Physical EQ/filter/FX mappings remain unknown; do not claim preset2 audible
  compatibility or ignore its EQ. Partial overrides and speed ramp details need
  explicit capability checks. Outer fallback uncertainty is documented, not guessed.

# Retained runtime stabilization state (separate workstream)

- Last verified deployed playback candidate: `b63949c10895735f0acd78252aef7d406e14dbf4`.
- Snapshot `/home/amogus/.cache/spotifyd-runtime-build-b63949c`, snapshot-local
  `target/release/spotifyd`; deployed `/usr/local/bin/spotifyd` SHA256:
  `32e6d39c07aeb2a55bfa5fb247f99e5d155a95aeb7e5143541248bc8d00d4af3`.
- Rollback `/usr/local/bin/spotifyd.rollback-90e1628-pre-b63949c`, SHA256:
  `54a8c36d487c5cbc39239fac1dd9f19fa9a50a5fc82695259b2e315d3edd2e9c`.
- Changes retain sink-stop-before-seek, async changed-position Load/preload,
  edge-change invalidation and hard-link audio-cache publication with copy fallback.
- Storage incident `/var/lib/spotifyd-diagnostics/manual/1789384890613886092-xrun`:
  cached current decode stalled3.336s amid doubled preload writes. `40b713c`
  fixes cache write amplification; runtime writeback proof remains pending.
- Stale-edge skip `/var/lib/spotifyd-diagnostics/manual/1789308023769543909-skip`
  is fixed by `b78e849`; post-fix natural edge-replacement evidence still pending.
- M5 remains open. This protocol work did not deploy, rebuild, or validate audible
  runtime fixes. September19 SSH confirmed user spotifyd service active; a system
  service query reported inactive because the runtime is a user service.

# Verification

- Prior production gate at b63949c: fmt/workspace check passed, playback99/99,
  Connect116/116, oracle5/5, doctest1/1; documented-allowance Clippy passed.
- Current change is docs and sanitized data only; no Rust or tooling code changed.
- Numeric style export validated recursively as numeric/boolean/null objects,
  with no string values, credentials, account/device IDs or client source.
- Final checks:4 evidence JSON files valid;69 style calls exactly match sources;
  preset2/FX sample checks,10 doc links, credential scan and diff check passed.
- Existing oracle consistency script completed:9/9 beats hashes,107/107 presets,
 105/107 geometry,22/107 speed bits; max speed delta7.6294e-6 and score still
  `not-yet-exact`. This does not establish full Auto parity.

# NEXT ACTION

Use the exact Sol High sequence in `docs/SPOTIFY_LIVE_RECIPE_PROTOCOL.md`, starting
with the capability-gated semantic resolver; keep unresolved DSP compatibility
claims disabled until their documented validation gates pass.
