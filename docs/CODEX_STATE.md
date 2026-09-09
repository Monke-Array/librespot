# Current objective

Execute the approved offline transition-operator implementation through M3.
M1-M3 are complete and committed. Live playback remains unchanged. M4 may
resume only with local work that does not depend on the missing S-01 Pilot V1
feature-schema audit.

# Branch / normative baselines

- Branch: `codex/m3a-live-auto-metadata`.
- Formal specification: `25da884a0c2e648d9cf86fc938d3efade827f389`.
- Approved implementation plan: `01818f0145da91ca00b96e76c61001d3e195c262`.
- M1 commits: `9cd5e81`, `73f1f6f`, `65f1e55`, `9cad5ce`, `2aeba66`,
  and golden/fixture gate `6a78080`.
- M2 commits: `71b9bcd`, `8f35b2c`, `1b2ff8f`, `d8416f7`, and state/gate
  record `c6066ae`.
- M3 commits: `09193f5` (render boundary), `f4b8332` (gain/dynamics/tails),
  `5c5dc1f` (filters/crossovers), `addb582` (time stretch), `82c3848`
  (output safety), and `1d0e02b` (assembled renderer/QC).
- Earlier runtime/network fix remains separate at `cbc3d21`; it was not changed
  by the offline operator work.

# Architecture and invariants

- `tools/transition-operator` is a standalone Rust workspace with no root
  workspace, librespot playback, or connect dependency.
- The data path is feature snapshot -> cue/geometry proposals -> deterministic
  named templates -> validated `OperatorPlan/v1` candidates -> hard validation
  -> offline renderer/QC -> future critic. The critic has no DSP-graph or
  playback-state authority.
- `safe_crossfade/v1` is an independent guaranteed fallback and experimental
  control, not the target musical behavior.
- Runtime Mixer/Auto and ordinary-playlist crossfade routes remain distinct.
- S-01/S-02 are never runtime dependencies. No RPI renderer or runtime lowering
  was introduced.

# M1 canonical foundation

- Closed `OperatorPlan/v1` models exactly seven approved operation kinds using
  signed 44.1 kHz frames and fixed integer units.
- Canonical JSON fails closed on floats/exponents, null, duplicate/unknown
  members, unsafe integers, noncanonical bytes, and non-ASCII plan strings.
- Operation order is validated, never silently reordered. Structural and
  contextual validation are separate and validated tokens have private fields.
- Plan/audio-semantics/candidate hashes are domain-separated and avoid
  self-reference. Paths, timestamps, and list positions do not enter semantic
  identity. Capabilities are derived from plan contents.
- Golden safe plan hash:
  `73a89d840145c1404737b89c932b1c90e1f351cad6815a57f7f63fb775dc381f`.
  Golden audio-semantics hash:
  `f95ec69c0b5944a45964fabe29dcc3d766e8c29ba25224f35f9269bff11f8594`.
  Golden candidate ID:
  `cand1-1d40381d56d6a176560ebb6780221b30a4c760945dd52cfb1d18438de54a41f3`.

# M2 deterministic candidate generation

- Semantic cue/window/geometry IDs, canonical input sorting, sealed predicates,
  fixed need/tie ordering, named recipes, one-geometry-per-recipe binding,
  deterministic quotas, semantic deduplication, diagnostics, and caps are
  implemented.
- All nine approved families are represented. At most six rich families are
  retained; the mathematical six-family maximum is 43 candidates including
  fallback. Soft cap is 48 and hard cap is 64 with nonrecursive fallback-only
  behavior.
- One analytic, nonboosting pair gain is injected byte-identically into every
  survivor before hashing. Render results never trigger candidate-local repair.
- The representative all-feature fixture yields 37 candidates across six rich
  families plus fallback. Frozen candidate-set hash:
  `afd1ea6b570cf67c4021496a942361e381231be445c908cfccc8573c127f8ee6`.

# M3 offline renderer and DSP

- Canonical s16le stereo 44.1 kHz source ingestion checks complete PCM hashes
  and frame counts through a private locator. Absolute paths are excluded from
  render and backend-program identities.
- The renderer processes one continuous window beginning exactly 4,096 frames
  before requested output. Its fixed order is time map, spectral stages,
  dynamics, tail capture, primary gains, sum, shared pair gain, limiter,
  measurement, PCM24 quantization, FLAC encode, and decode/hash verification.
- Working operators: linear/smoothstep/equal-power/asymmetric gains; canonical
  hard/soft cuts; resolved ducking; rhythmic gate/handoff; feed-forward delay
  tails; energy-ramp plans; RBJ LP/HP automation on the signed 64-frame grid;
  LR4 two/three-band crossover and spectral handoff; constant incoming
  pitch-preserving time stretch; shared pre-gain; and linked lookahead limiting.
- The limiter uses inclusive 221-frame lookahead, immediate attack, fixed
  4,410-frame release, whole-render maximum reduction, and transition-window
  activity. True peak uses explicit BS.1770-4 Annex 2 four-phase/12-tap math;
  backend meters do not define IR semantics.
- Canonical PCM24 uses ties-away quantization and rejects any required clamp.
  Metadata-stripped FLAC is decoded again and must reproduce the exact PCM hash.
- QC rejects nonfinite/clipping/true-peak/limiter/length/source/backend/hash
  failures without normalization or rerender. Rich failures are isolated;
  fallback failure is terminal and never synthesizes silence or a hard cut.
- Artifact, measurement, render, QC, and private backend-provenance records are
  hash-linked. Backend command text remains renderer-private.
- Descriptive loudness is deterministic ungated full-buffer stereo RMS because
  the formal spec names a loudness field without defining a loudness profile;
  it is not used for QC, normalization, generation, or selection.

# TDD and verification evidence

- Tasks 14-19 followed the approved RED -> GREEN sequence: focused tests first
  exposed missing APIs/unsupported renderer stages, then passed after the
  minimum implementation; each task was committed at its coherent boundary.
- 2026-09-09 standalone full gate: 109 Rust tests passed (including 16 renderer,
  8 safety, 6 envelope/tail, 6 filter/crossover, 4 time-stretch, 13 generator,
  14 template, 26 validation); one performance characterization is ignored by
  default. Rust doc tests passed.
- Independent Node canonical/hash golden: 1/1 passed.
- Standalone `cargo fmt --check` and all-target Clippy with `-D warnings` passed.
  Root `cargo fmt --check`, `cargo check --workspace`, and `git diff --check`
  passed.
- Root regressions: `librespot-playback` 83/83; `librespot-connect` 113/113
  unit, 5/5 integration, and 1/1 doctest.
- Installed-FFmpeg tests decode synthetic WAV, round-trip exact PCM24 through
  FLAC, and render the same request three times with identical container,
  decoded-PCM, and measurement results.
- Ignored 64-candidate/36-second performance characterization passed:
  39.598 s total, 0.01719 render/audio ratio, 258,019,328-byte peak RSS,
  9,525,600-byte maximum artifact, and one live candidate artifact. Bounds are
  300 s, 0.131, 512 MiB, 512 MiB, and one respectively. Report is disposable
  under `tools/transition-operator/target/performance/` and is not tracked.

# Adversarial review / boundary audit

- Valid approved operation subsets, rather than an unapproved arbitrary hybrid,
  exercise every v1 DSP stage three times.
- Malformed or permuted plans fail before locator/backend calls. Negative
  timeline/filter-grid behavior uses signed arithmetic and mathematical modulo.
- DSP/filter/delay/limiter state is per render; no state crosses requests.
- Source windows, frame-zero cuts, tail half-open bounds, exact output length,
  cue alignment, encode/decode identity, and path-independent render identity
  have direct tests.
- No backend auto-normalization, candidate-specific loudness repair, feedback
  delay, detector ducking, reverb, synthetic noise/riser, stems, arbitrary
  graphs, or new template was added.
- No runtime/playback files, audio, private filenames/manifests, credentials, or
  temporary DSP artifacts are tracked by M1-M3.

# External state / unresolved issues

- Pilot V1 remains a small single-rater result with a large repeat-control noise
  floor. Its gate decision remains B: improve operators and run another blind
  pilot before runtime integration.
- The exact S-01 Pilot V1 feature schema is absent from the local Git object
  store. S-01 was not woken. M4 must stop before correctness depends on that
  audit.
- Private Pilot V2 pairing/corpus freezing, final evaluator datasets, critic,
  and runtime integration remain explicitly out of scope.

# NEXT ACTION

Begin M4 Task 20 only when the S-01 Pilot V1 feature-schema audit is authorized
and available. Until then, the safe local resume point is to inspect Task 21 for
work provably independent of that schema; do not infer or freeze feature fields.
