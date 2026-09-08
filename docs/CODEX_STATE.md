# Current objective

Keep the validated Mixer/non-Mixer playback mechanics unchanged while improving
offline transition quality. Blind pilot V1 reached decision B: learned behavior
is promising, but the deterministic transition vocabulary/rendering must
improve and pass another held-out blind pilot before runtime integration.

# Branch / commits

- Branch: `codex/m3a-live-auto-metadata`.
- Blind Pilot V1 analysis/state commit: `98dcd31` (`docs: record blind
  transition pilot findings`).
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
- Condition mapping was decoded only after freezing, from the exact seeded
  generator procedure, then cross-checked because every mapped MLP sample is
  the unique audio hash. Full mapping is in the analysis report.

# Human-evaluation findings

- Identical Auto/linear controls had only 6/24 exact rank/ranking-group
  agreement; rank is noisy and must be interpreted against repeat controls.
- Held-out test MLP preference was 1W/4L against both baselines; smoothness lost
  while intent showed isolated gains. Runtime integration is not justified.
- Across all pairs, MLP more often improved intent than smoothness, but it also
  produced more flags than either baseline.
- Comments repeatedly describe baselines as safe/boring crossfades, while MLP
  exposes interesting cues but frequent beat mismatch, simultaneous overlap,
  and old material dragging too far into the new track. Some MLP choices are
  plainly poor; other ideas appear limited by the primitive operator vocabulary.
- Gate decision: **B**. Improve deterministic operators/candidate vocabulary
  and run another blind pilot before any runtime integration.
- Full analysis:
  `docs/MTG_JAMENDO_PILOT_V1_BLIND_ANALYSIS.md`.

# Approved direction / design-preparation result

- Architecture A is approved in principle: semantic musical templates compile
  into a flat, explicit, versioned `OperatorPlan`; Phase 1 is offline-only and a
  possible RPI renderer is a separate later lowering target.
- Proposed initial templates are safe crossfade, shaped handoff, beat cut, bass
  handoff, spectral handoff, resolved ducked overlap, feed-forward echo tail,
  energy ramp, and short rhythmic handoff. Reverb, noise/riser, stems, and
  arbitrary hybrids are deferred pending evidence.
- Proposed generation retains fallback plus at most six optional families,
  normally at most 48 candidates and never more than 64. Templates emit zipped
  named recipes rather than Cartesian parameter grids.
- The canonical plan uses fixed processing stages, typed bounded operations,
  integer units, strict validation/canonicalization, and no backend strings or
  arbitrary graph. The critic ranks only already-valid candidate IDs.
- Detailed approval draft:
  `docs/TRANSITION_OPERATOR_DESIGN_PREPARATION.md`.

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

- 2026-09-08 private inventory: 66/66 MP3s probed and decoded successfully;
  66 unique canonical decoded-PCM hashes; no duplicates or short/unsuitable
  files. All are stereo 44.1 kHz. One bitrate warning; 57/66 measure at or above
  0 dBTP, motivating mandatory shared headroom and true-peak QC.
- 2026-09-08 disposable DSP spike covered 15 programs and all requested
  operator classes. Three representative combinations were each rendered three
  times with matching container and decoded-PCM hashes. Median throughput was
  about 1,140x real time; the worst stretch/filter/limiter program was about 60x
  real time; maximum resident set was about 38.1 MiB.
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
- Private inventory (outside Git), SHA-256
  `30f8d5bef4bee0e2e53018b59e367a2a0295caeaf9ba41cafdad56442a38706a`:
  `C:\Users\janni\Desktop\spotify-transition-private-v2\private-music-inventory-v1.json`.
- Private feasibility record (outside Git), SHA-256
  `fd2c789edabba8ba7e850170921a8bcb30f1b604111f4c09dee4c641c6ef2317`:
  `C:\Users\janni\Desktop\spotify-transition-private-v2\offline-dsp-feasibility-v1.json`.
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
- The exact S-01 Pilot V1 offline feature schema is not in the local Git object
  store. Audit it when S-01 is next available; it was not woken for design work.
- The private MP3s have no embedded identity tags; private artist/album/style
  annotation is required before Pilot V2 pairing and leakage controls are frozen.
- Genuine Spotify Lossless still needs a supported resolver/manifest path that
  this client does not currently implement or receive.

# NEXT ACTION

Human-review `docs/TRANSITION_OPERATOR_DESIGN_PREPARATION.md` and approve or
revise the detailed IR, nine-template vocabulary, safety/candidate budgets, and
Pilot V2 protocol before the formal design specification or implementation plan
is written.
