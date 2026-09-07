# Current objective

Keep the validated Mixer/non-Mixer playback mechanics unchanged while advancing
offline transition quality. The immediate gate is human blind preference on the
new real-audio pilot.

# Branch / commits

- Branch: `codex/m3a-live-auto-metadata`.
- Networking implementation: `cbc3d21` (`fix(connect): retire dealer on session
  replacement`), based on validated runtime commit `948a5d8`.
- ML work is isolated on S-01 branch `codex/pilot-v1`: implementation
  `227c7d0`; state/ratings template `2afa90d`.
- No ML code or model has entered librespot, spotifyd, or RPI-01 runtime.

# Architecture / invariants

- Mixer and ordinary-playlist routes remain distinct.
- The queue/context -> edge -> preload -> secondary decoder -> PCM readiness ->
  transition -> promotion -> queue ownership path is already runtime-validated.
- Deterministic fallback always remains available. ML may rank only candidates
  that have independently passed hard playback constraints.
- S-01/S-02 are never runtime playback dependencies.
- Replacing a Spotify session preserves SPIRC/player recovery state, retires the
  old AP only when valid, and now always closes the old dealer task.

# ML transition-quality findings

- S-01 has a legal, auditable 48-track MTG-Jamendo cohort with unique artists,
  albums, and tracks; derivative-compatible CC licenses, source revision,
  attribution, acquisition date, and hashes are retained privately.
- Tempo-nearest pairing produced 24 track-disjoint pairs with frozen 14/5/5
  pair splits (28/10/10 tracks), preventing track leakage.
- Each pair has 64 hard-valid candidates. The blind set has 72 stereo 44.1 kHz
  PCM24 FLAC previews, 36 seconds each, with shared per-pair non-boosting gain
  and a -1 dBFS ceiling.
- Linear selected the same candidate as local Auto for all 24 pairs; the small
  MLP selected a different candidate for all 24. There are 48 unique hashes;
  duplicate hidden conditions remain as a consistency check.
- Rankers were trained only on synthetic fixtures. Perfect synthetic accuracy
  proves plumbing, not musical quality. No human preference is recorded yet.
- Public manifest SHA-256:
  `6370f69a00b10bf2359aedae42b5ed1ae319f239350e7d7ffe5209922b3909dd`.

# Audio-quality findings / implementation

- The live spotifyd config already requests bitrate 320. Classic metadata
  probes for five current tracks exposed AAC 24 plus Ogg/Vorbis 96/160/320 and
  no FLAC; current selection therefore delivers Ogg/Vorbis 320.
- Protocol enums mention FLAC/24-bit FLAC, but this client does not compile/use
  the newer audio-file extension, does not include FLAC in selection order, and
  advertises `VERY_HIGH` without `supports_hifi`. Merely setting the capability
  would not make the stream lossless.
- Spotify officially offers Premium Lossless on supported clients/devices, but
  the active librespot metadata route does not expose it. Genuine lossless is
  therefore unavailable on this client path today.
- Librespot decodes/mixes at stereo 44.1 kHz floating point, then applies
  triangular dither to the S16 output. The Pi analog device supports only U8 or
  S16, though it natively supports 44.1 kHz.
- The old ALSA `default` path negotiated S16_LE/48 kHz and resampled the 44.1 kHz
  engine output. RPI config now uses direct `device = "hw:0,0"`, preserving the
  native 44.1 kHz route; bitrate remains 320 and format remains S16.
- Config rollback:
  `/home/amogus/.config/spotifyd/spotifyd.conf.before-direct-44100-20260907`.

# Networking findings / runtime measurement

- The validated old binary recovered from an AP disconnect in about two
  seconds with `NRestarts=0`, but two offset dealer ping loops remained after
  replacement: four pings/minute instead of two. The old dealer task leaked.
- `cbc3d21` always closes the replaced session's dealer, including when the AP
  session is already invalid. A unit policy test covers both valid/invalid AP
  retirement cases.
- Exact staged ARM candidate SHA-256:
  `cb59d7621a4f37327eeebafc395af5b56c67a7ccad2c7f5c36d4666ad966e2ea`.
- Controlled staged test: invalidation at 12:12:54, authentication at 12:12:56,
  replacement at 12:12:57. The old dealer logged closure/drop, and exactly one
  later 30-second ping/pong cycle appeared: two pings/minute, down from four.
- Candidate test used a temporary exact-AP nft rule. Its scheduled cleanup
  succeeded and no Codex nft tables remain.
- The staged candidate was not installed. Normal `spotifyd.service` is active,
  PID 33695, `NRestarts=0`.
- Installed binary remains
  `b064c33ee2c8eaef0971c12a57d88e7eecef153dfe6e0259345e3044d2430679`;
  rollback remains `21e24592bf67d81f92ae79f9f6130c7e1398071b10f9a38cf491e82e56d7f3bd`.

# Latest verification

- U-01: `cargo fmt --check`, `cargo check --workspace`, and
  `git diff --check` passed.
- U-01: `cargo test -p librespot-connect` passed 113 unit, 5 integration, and 1
  doctest; `cargo test -p librespot-playback` passed 83 tests.
- RPI-01 native release build of staged `cbc3d21` completed in 18m31s with one
  job and release LTO disabled; installed/rollback binaries were untouched.
- S-01 worker `job-20260907T100800-321c5317`: all 85 ML tests passed in 1.052s.
- S-01 full pilot `job-20260907T094622-f847c929`: completed in 1221.92s, peak
  RAM 1400.602 MB, peak CPU 128.764%, GPU unavailable, zero warnings.

# Artifacts / jobs

- User-facing blind set:
  `C:\Users\janni\Desktop\spotify-transition-pilot-v1-20260907` (72 FLACs,
  manifests, protocol, ratings template; no answer keys), 453,106,649 audio
  bytes. Transfer archive SHA-256 was
  `38e8280a174521ba7296b2a4e6b42ce58ada29cf00d208120b3b061f905da49d`.
- S-01 public manifest:
  `/home/profdrhuso/Projects/spotify-transition-ml-pilot-v1/evaluations/blind/mtg-jamendo-pilot-v1/manifest.json`.
- S-01 private report:
  `/home/profdrhuso/Projects/spotify-transition-ml-pilot-v1/datasets/cache/mtg-jamendo-pilot-v1/private-pilot-report.json`.
- All task worker jobs are complete; none remain active. S-01 was already online
  before this task and must not be automatically powered off.

# Known unresolved issues

- The blind pilot still needs human ratings before condition identities can be
  decoded or the learned selector can claim musical value.
- The direct ALSA config is active and hardware capability was verified; its
  negotiated 44.1 kHz `hw_params` will appear only while playback opens the sink.
- Genuine Spotify Lossless requires a supported resolver/manifest path that this
  librespot client does not yet implement or receive.

# NEXT ACTION

Complete and freeze the 24-pair blind ratings from the local pilot, then decode
conditions and report paired validation/test preference before changing any
model or runtime selector.
