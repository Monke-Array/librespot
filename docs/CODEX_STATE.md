# Current objective

The private real-music engineering listening set is complete. Stop before any
further M4 work until the owner has listened and gives a new direction. M1-M3
remain complete; Task 20, Task 21, and the minimum Task-22 extractor bridge are
implemented. Live playback remains unchanged.

# Branch / normative baselines

- Branch: `codex/m3a-live-auto-metadata`.
- Formal specification: `25da884a0c2e648d9cf86fc938d3efade827f389`.
- Approved implementation plan: `01818f0145da91ca00b96e76c61001d3e195c262`.
- M3 gate/state: `449dc7f`.
- Task 20 Pilot V1 audit: `e399875`.
- Task 21 feature snapshot schema: `aeb76da`.
- Initial Task-22 extractor: `18e2d35`.
- Task-22 rhythm/collision follow-up: `f246f20`.
- Large FFmpeg pipe deadlock fix: `7b9f115`.

# Architecture and invariants

- `tools/transition-operator` remains a standalone offline crate with no root
  workspace, librespot playback, or connect dependency.
- Data flow remains audio-derived feature snapshot -> deterministic M2
  candidates -> validated OperatorPlan -> certified M3 render/QC.
- `safe_crossfade/v1` is the fallback/control, not the rich musical target.
- The extractor does not fabricate missing values. Vocal activity/collision is
  absent because Task 20 found no reusable frozen Pilot V1 semantic source.
- No critic, Pilot V2 freeze/evaluator, M5/M6, runtime integration, RPI work,
  new transition family, or arbitrary DSP graph was started.

# Task 20 / Task 21

- The S-01 evidence audit found Pilot V1 BPM/downbeat/cue-confidence and peak
  fields unsuitable for direct reuse: peak was mono sample peak, cue confidence
  was a fixed constant, and downbeats included a synthetic fallback. These are
  recomputed in v2; vocal remains absent.
- `transition-feature-snapshot/2` is strict, hashed, fixed-unit, absence-based,
  and exposes only the approved M2 TemplateInputs.
- No contradiction was found between the frozen evidence and the approved M4
  plan/specification.

# Minimum Task-22 extractor

- Canonical input is complete s16le stereo 44.1 kHz PCM with exact PCM identity.
- Whole-source true peak uses the M3 BS.1770 4x implementation.
- Rhythm uses 10 ms positive RMS-dB flux and a 40-240 BPM autocorrelation search.
  Confidence is based on selected-peak prominence over the lag-distribution p90,
  with explicit monotone anchors. Four-beat phase confidence is independent and
  has no synthetic fallback.
- Synthetic strong periodic/accented, uniform-meter-ambiguous, silence,
  monotonic calibration, repeatability, and invalid-input cases are covered.
- Cue-relative transient collision is temporal onset intersection-over-union,
  not the geometric mean of aggregate transient activity. Aligned, partial, and
  displaced onset vectors have adversarial coverage.
- Windows measure transient, bass, three-band occupancy/stability, energy mean
  and variability. Geometry proposal remains deterministic and bounds-safe.
- Frozen extractor algorithm hash is recorded in the synthetic fixture and in
  every private feature snapshot.

# Private listening set

- Private directory (outside Git):
  `C:\Users\janni\Desktop\spotify-transition-listening-demo-20260910`.
- Four pairs were selected deterministically before transition rendering from a
  20-track order-statistic pool under the authorized Downloads root.
- Complete M2 candidate counts are 16, 25, 9, and 9.
- Rendered family sets are respectively:
  - pair 01: safe crossfade, shaped handoff, bass handoff;
  - pair 02: safe crossfade, shaped handoff, bass handoff, spectral handoff;
  - pair 03: safe crossfade, shaped handoff, spectral handoff;
  - pair 04: safe crossfade, shaped handoff, spectral handoff.
- One lowest-candidate-ID representative per applicable family was selected,
  then presentation order was deterministically hash-shuffled.
- Thirteen blind 36-second FLACs passed M3 validation, rendering, encode/decode
  PCM identity, and QC. There were zero render/QC failures.
- A safe control and one rich sample were rendered twice with identical FLAC
  bytes and decoded PCM hashes.
- Private snapshots, full candidate sets, the private manifest, source names,
  and unblinding file remain only in the private directory outside Git.

# Defects discovered and fixed

- Prior ~3.7 GB usage came from retaining 20 complete decoded `f64` stereo PCM
  buffers inside `SignalFeatures`: the actual pool represented 3,252,838,400
  bytes (3.03 GiB) before allocator/renderer overhead. The private harness now
  retains only small immutable rhythm/identity summaries, releases each pool
  PCM immediately, and re-decodes only one selected pair at a time. Observed
  rendering working set was about 415 MiB.
- Real non-unity time stretching exposed a bidirectional pipe deadlock: the M3
  backend wrote multi-megabyte stdin before draining FFmpeg stdout. The backend
  now feeds stdin on a scoped writer while stdout/stderr are collected. A 4 MiB
  regression test hung before the fix and completes after it. DSP semantics and
  render identities were not changed.

# Verification

- Focused Task-22: 12/12 feature tests and 13/13 generator tests pass.
- Complete standalone transition-operator suite passes, including 5/5 time
  stretch tests and 16/16 renderer tests; one performance characterization is
  intentionally ignored.
- Standalone formatting and all-target Clippy with `-D warnings` pass.
- All 13 private FLACs are non-empty, match recorded container hashes, decode
  successfully in pinned FFmpeg, and report the exact 1,587,600-frame window.
- All listening samples map uniquely to candidates in the preserved full M2
  candidate sets. M3 recorded decoded PCM hashes and QC measurements.
- No runtime/playback source, private music, private source filename, private
  manifest, or rendered audio is tracked by the repository.

# NEXT ACTION

Owner listens to the blind FLACs and records qualitative engineering feedback.
Do not resume Task 23/24 or broader M4 work without a new explicit request.
