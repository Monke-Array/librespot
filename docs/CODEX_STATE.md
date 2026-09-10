# Current objective

The requested second engineering listening set stopped after its exhaustive
family-applicability audit because four requested unseen families have zero
honest applicable pairs. No second-set pair selection or rendering occurred.
Wait for owner direction; do not start critic training, Pilot V2, M5/M6,
browser evaluation, or runtime/RPI integration. Live playback remains unchanged.

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

# First private listening set

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

# Second vocabulary applicability audit

- Private directory (outside Git):
  `C:\Users\janni\Desktop\spotify-transition-listening-demo-v2-20260910`.
- First-set qualitative observations are recorded privately at pair granularity
  only. They contain no candidate labels and are not approved training data.
- The audit used the approved FeatureSnapshot/v2 extractor and real M2 generator
  over all 4,290 directed distinct-track pairs in the authorized 66-track
  Downloads corpus. All 4,290 pairs produced a snapshot and candidate set.
- Applicable-pair counts are: safe crossfade 4,290; shaped handoff 600; beat cut
  0; bass handoff 51; spectral handoff 492; ducked overlap 0; echo-tail handoff
  0; energy ramp 18; rhythmic handoff 0.
- Beat cut, echo-tail handoff, and rhythmic handoff are unreachable with the
  current approved extractor because they require outgoing vocal evidence. The
  extractor intentionally omits vocal evidence and required missing values fail
  closed; absence may not be interpreted as vocal-free.
- Ducked overlap is also absent: vocal collision is unavailable and measured
  cue-relative transient IoU spans 16,934–269,544 ppm, below the localized
  collision band beginning at 300,000 ppm. Existing adversarial vectors show
  the IoU metric can reach the band, so this is a real-corpus calibration gap,
  not an implementation defect. No threshold changed.
- Energy ramp is honestly applicable to 18 pairs. Eleven emit a distinct M2
  candidate; in seven negative-delta cases second-based plans deduplicate with
  shaped-handoff audio semantics while bar/filter recipes lack compatible
  rhythm geometry.
- The mandated zero-family stop fired before selection/rendering. The v2
  directory contains audit evidence and zero audio samples; no unblinding or
  ratings template was created.

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
- The second audit independently recounts 4,290 pair rows with zero failures and
  matches every recorded family count. Private manifest artifact hashes match.
- Fresh 2026-09-10 checks pass: transition-operator formatting, all-target
  Clippy with `-D warnings`, and its complete suite (including 12 feature, 13
  generator, 14 template, and 16 renderer tests); root workspace formatting and
  checking; 83 playback tests; and 113 connect unit tests plus 5 oracle tests.
- The second audit retained at most two four-track decoded blocks. Measured peak
  working set was 1,929,699,328 bytes (1,840.30 MiB); full-corpus PCM was never
  retained.
- No runtime/playback source, private music, private source filename, private
  manifest, or rendered audio is tracked by the repository.

# NEXT ACTION

Owner reviews the zero-family audit and explicitly decides whether to authorize
a separately versioned vocal-evidence extractor and/or a localized-collision
calibration change before requesting another listening set.
