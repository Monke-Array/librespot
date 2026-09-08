# Current objective

Keep the validated Mixer/non-Mixer playback mechanics unchanged while improving
offline transition quality. Blind pilot V1 reached decision B: learned behavior
is promising, but the deterministic transition vocabulary/rendering must
improve and pass another held-out blind pilot before runtime integration.

# Branch / commits

- Branch: `codex/m3a-live-auto-metadata`.
- Networking implementation: `cbc3d21` (`fix(connect): retire dealer on session
  replacement`), based on runtime-validated `948a5d8`.
- ML research remains isolated on S-01 branch `codex/pilot-v1`:
  implementation `227c7d0`; state/ratings template `2afa90d`.
- No ML code/model has entered librespot, spotifyd, or RPI-01 runtime.

# Architecture / invariants

- Mixer and ordinary-playlist routes remain distinct.
- Queue/context -> edge -> preload -> secondary decoder -> PCM readiness ->
  transition -> promotion -> queue ownership is runtime-validated.
- Deterministic fallback is mandatory. ML may only rank candidates that already
  satisfy deterministic legality, buffer, alignment, duration, and clipping
  constraints.
- S-01/S-02 are never runtime playback dependencies.
- Spotify-session replacement preserves recovery state and always closes the
  replaced dealer task.

# Blind pilot V1 / verified analysis

- Cohort: 48 legal, auditable MTG-Jamendo tracks; 24 track-disjoint pairs with
  frozen 14/5/5 train/validation/test splits (28/10/10 tracks).
- Each pair has 64 hard-valid candidates. The set contains 72 36-second stereo
  44.1 kHz PCM24 FLAC previews, shared non-boosting gain, and -1 dBFS ceiling.
- Linear selected the same candidate/audio as local Auto in all 24 pairs; MLP
  selected a different candidate/audio in all 24.
- Completed ratings passed the `transition-pilot-ratings/v2` validator with
  24/24 complete pairs, 72/72 samples, zero schema errors, correct order, and
  exact manifest/sample bindings before unblinding.
- Exact completed/frozen ratings SHA-256:
  `0e1c7f4bf27ec0241af115ca55bc16541a3c86b423c866baa61189b2a923688c`.
- Frozen read-only artifact:
  `C:\Users\janni\Desktop\spotify-transition-pilot-v1-20260907\analysis\frozen\ratings-completed.sha256-0e1c7f4bf27ec0241af115ca55bc16541a3c86b423c866baa61189b2a923688c.json`.
- Frozen source manifest SHA-256:
  `6370f69a00b10bf2359aedae42b5ed1ae319f239350e7d7ffe5209922b3909dd`;
  sanitized evaluator manifest:
  `489fb0022a3c6972fdc5018075e85e5374164e38b737e8e840b6bf293e09d2b0`;
  ratings template:
  `41d0f47ff50b4e0bf460cd5e785a3ad598e01e7c0a283452a96351591750eae6`.
- Condition mapping was decoded only after freezing, from the exact seeded
  generator procedure, then cross-checked because every mapped MLP sample is
  the unique audio hash. Full mapping is in the analysis report.

# Human-evaluation findings

- Identical Auto/linear controls occur in 24/24 pairs. Agreement was 19/24
  category, 16/24 smoothness, 14/24 intent, and only 6/24 exact rank/ranking
  group. Mean absolute differences were 0.375 smoothness, 0.667 intent, and
  0.833 rank; rank must be treated as imprecise.
- Control disagreement was not low-confidence-driven: any core disagreement
  occurred in 1/3 confidence-1–3 pairs versus 18/21 confidence-4–5 pairs.
- Validation MLP vs Auto: rank 1W/4L/0T, rating 1W/1L/3T,
  smoothness +0.60, intent +0.60. MLP vs linear: rank 3W/2L/0T,
  rating 2W/1L/2T, smoothness +0.60, intent +1.00. The contradictory ranks are
  confounded by 0/5 control-rank agreement in validation.
- Test MLP vs both baselines: rank 1W/4L/0T. Versus Auto it was -1.80
  smoothness/+0.40 intent; versus linear -1.40/0.00. Test preference improvement
  is not demonstrated, and the smoothness loss exceeds the control noise floor.
- On the three confidence-4–5 test pairs, MLP ranked last 3/3 against both
  baselines and had no rating or score wins. The two confidence-3 test pairs
  split one preference win/loss while showing +1.5 intent and -2.0 smoothness.
- Across all 24, MLP intent outcomes were 13W/6L/5T vs Auto and 14W/6L/4T vs
  linear, while smoothness was 5W/12L/7T and 6W/12L/6T. MLP had 14 flagged
  samples/24 flag instances; each baseline had 9/12.
- Comments repeatedly describe baselines as safe/boring crossfades, while MLP
  exposes interesting cues but frequent beat mismatch, simultaneous overlap,
  and old material dragging too far into the new track. Some MLP choices are
  plainly poor; other ideas appear limited by the primitive operator vocabulary.
- Gate decision: **B**. Improve deterministic operators/candidate vocabulary
  and run another blind pilot before any runtime integration.
- Full analysis:
  `docs/MTG_JAMENDO_PILOT_V1_BLIND_ANALYSIS.md`.

# Proposed offline follow-up

- Add deterministic EQ/spectral and bass handoffs, filter sweeps, beat/bar
  envelopes, rhythmic cuts, reverb/echo tails, ducking, energy ramps,
  appropriate noise/riser masking, feasible stem-aware handoff, and constrained
  combinations; do not implement these inside playback ownership logic.
- A later private U-01 pilot should use 30 pairs/60 of the 66 owned MP3s with an
  18/6/6 track-disjoint split, 15 compatible and 15 awkward pairs, three core
  conditions, and 12 seeded exact repeats (102 samples). Audio remains local,
  private, hash-addressed, gitignored, and fully QC-probed; tie UX must use
  explicit ordered groups.

# Audio/runtime state

- spotifyd requests Ogg/Vorbis 320; this librespot path does not expose Spotify
  Lossless despite protocol FLAC enums.
- Librespot mixes stereo 44.1 kHz float and dithers to S16. RPI-01 uses direct
  `hw:0,0`, avoiding the former 48 kHz ALSA resample. Config rollback:
  `/home/amogus/.config/spotifyd/spotifyd.conf.before-direct-44100-20260907`.
- `cbc3d21` closed the leaked replaced-session dealer in a controlled staged
  test: recovery remained about two seconds and ping traffic returned from four
  to two pings/minute. The staged candidate was not installed.
- Installed RPI binary SHA-256 remains
  `b064c33ee2c8eaef0971c12a57d88e7eecef153dfe6e0259345e3044d2430679`;
  rollback remains `21e24592bf67d81f92ae79f9f6130c7e1398071b10f9a38cf491e82e56d7f3bd`.

# Latest verification

- 2026-09-08 U-01 evaluator: 37/37 Node tests passed with global Web Crypto;
  all 72 FLAC hashes and frozen manifest/template hashes matched.
- 2026-09-08 direct ratings validation: zero errors, 24 complete pairs, 72
  complete samples; frozen-copy size/hash and read-only attribute verified.
- Analysis metrics were recomputed from the hash-checked frozen artifact and
  cross-checked against the report before the disposable local script was
  removed.
- Previous U-01 Rust gate: `cargo fmt --check`, `cargo check --workspace`, and
  `git diff --check` passed; connect passed 113 unit, 5 integration, 1 doctest;
  playback passed 83 tests.
- S-01 worker `job-20260907T100800-321c5317`: 85 ML tests passed. Full pilot
  worker `job-20260907T094622-f847c929` completed with zero warnings.

# Artifacts / external state

- Self-contained public pilot:
  `C:\Users\janni\Desktop\spotify-transition-pilot-v1-20260907`.
- S-01 public manifest:
  `/home/profdrhuso/Projects/spotify-transition-ml-pilot-v1/evaluations/blind/mtg-jamendo-pilot-v1/manifest.json`.
- S-01 private report:
  `/home/profdrhuso/Projects/spotify-transition-ml-pilot-v1/datasets/cache/mtg-jamendo-pilot-v1/private-pilot-report.json`.
- S-01 was offline during analysis and was not woken; no worker job was started.

# Known unresolved issues

- Pilot V1 has one rater, small held-out splits, and a large repeat-derived rank
  noise floor; another controlled pilot is required.
- Candidate intent and renderer/operator quality changed together, so the pilot
  cannot fully attribute poor output to selection versus realization.
- Genuine Spotify Lossless still needs a supported resolver/manifest path that
  this client does not currently implement or receive.

# NEXT ACTION

Design and implement the richer deterministic offline transition-operator
vocabulary, then freeze a new track-disjoint blind pilot before reconsidering
runtime integration.
