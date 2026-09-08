# Transition Operator Architecture: Research and Design Preparation

Date: 2026-09-08
Status: **approval draft, not an implementation specification**

This report prepares the detailed design for an offline-only transition operator
subsystem. It records repository findings, private-library technical results,
throwaway DSP measurements, and the proposed shape of the design. It does not
authorize production implementation. In particular, it does not change the
runtime `TransitionPlan`, `TransitionEngine`, player, queue ownership, decoder
lifetime, buffering, promotion, or reconnect behavior.

Architecture A has been selected in principle:

> Semantic musical templates compile into a flat, explicit, versioned
> `OperatorPlan` intermediate representation. Phase 1 uses an offline reference
> renderer. A future RPI-01 renderer, if justified by human evidence, is a
> separate lowering target.

The next required human decision is approval or revision of the detailed design
in this report. The formal design specification and implementation plan remain
behind that approval gate.

## 1. Scope and conservative assumptions

Phase 1 exists to learn which transition strategies improve blinded human
preference before paying for live playback integration. The following choices
were made conservatively while the owner was unavailable:

- FFmpeg is treated as a disposable Phase 1 backend, not as the IR.
- Private paths stay only in private artifacts outside Git. Repository documents
  contain aggregate technical facts and cryptographic artifact identities.
- A missing or unreliable musical feature disables a dependent template. The
  generator never invents a default that makes a candidate appear applicable.
- Dynamic, signal-dependent behavior is resolved into explicit automation when
  feasible. This makes rendering and later comparison more reproducible.
- Phase 1 does not use source separation, convolution, arbitrary effect graphs,
  learned parameter generation, or an arbitrary hybrid-template combinator.
- All advanced candidates may disappear; one ordinary crossfade is always
  emitted as the independent guaranteed fallback.

## 2. Current architecture diagnosis

The current separation of concerns is sound and must remain intact:

```text
Spotify/Connect -> queue/context -> preload -> secondary decoder
  -> PCM readiness -> transition -> promotion -> queue ownership

track information -> deterministic candidate generation
  -> candidate scoring -> TransitionPlan
```

The limitation is the transition-quality vocabulary, not the ownership
boundary:

- `connect/src/spotify_auto_mix.rs` generates geometrically legal overlaps from
  cues, beats, duration, tempo, vocals, key, and genre-related inputs. Candidate
  diversity is predominantly cue position, overlap length, and incoming speed.
- `connect/src/spotify_auto_mix_selection.rs` selects a ranked overlap and a
  preset, but the selected preset does not currently become differentiated DSP.
- `transition_plan_for_local_auto_transition` in
  `connect/src/spotify_mix.rs` lowers local Auto to outgoing and incoming linear
  gain curves plus optional incoming speed automation.
- `TransitionPlan` and `TransitionEngine` in `playback/src/transition.rs` support
  time placement, gain curves, mixing, and incoming speed automation. They do
  not provide filters, band handoffs, ducking, tails, or masking.
- `connect/src/spotify_materialized_transition.rs` can parse Spotify EQ/filter
  curve data, but deliberately rejects unsupported rendering requirements unless
  a development bypass ignores them. Unknown effect automation remains rejected.

Consequently, the existing candidate generator can find musically interesting
cues while the renderer still realizes most choices as a full-band overlap.
Pilot V1 changed selection intent and rendering geometry together, so its result
cannot attribute all losses to the learned selector.

The exact S-01 Pilot V1 offline feature schema is not in the current local Git
object store. S-01 was offline and was intentionally not woken for this design
work. Auditing that schema is a small deferred preparation task before Phase 1
implementation; it is not a reason to broaden the IR now.

## 3. Pilot V1 failure to design requirement

| Observed failure | Architectural limitation | Design response |
| --- | --- | --- |
| Safe candidates sounded like boring crossfades | One rendering vocabulary despite different cue intent | Emit a few named strategies with materially distinct audible behavior |
| Excessive full-band overlap | Overlap geometry has no spectral ownership | Shorten overlap or transfer bass/bands explicitly |
| Outgoing drums dragged into the new track | Long gain fade does not respect rhythmic handoff | Require strong beat/bar predicates for a short cut or rhythmic handoff |
| Bass, percussion, or vocals collided | No content-aware attenuation or band control | Use no-overlap cuts, complementary band handoffs, or resolved local ducking |
| Beat ideas rendered with beat mismatch | Effects cannot repair a bad timing predicate | Reject candidates with weak alignment; do not “fix in post” |
| Fade shapes sounded crude or lost energy | Only basic linear automation reaches playback | Add equal-power, smoothstep, and bounded asymmetric gain envelopes |
| Higher-intent candidates clipped or became dense | No operator-level headroom contract | Require analytic headroom, post-render measurement, and a bounded safety limiter |
| Learned critic lost held-out preference | Critic ranked a weak/confounded vocabulary | Test the vocabulary against fallback before testing critic value |
| Repeated ratings were noisy | Tie/rank interaction obscured identical controls | Use explicit tie groups, repeated controls, and predeclared interpretation gates |

The governing rule is that an operator must solve a named musical failure. An
effect does not enter the initial vocabulary merely because the backend can
render it.

## 4. Private 66-track technical inventory

The source directory was read in place. No original was modified or moved. The
private inventory is outside the repository:

```text
C:\Users\janni\Desktop\spotify-transition-private-v2\private-music-inventory-v1.json
SHA-256 30f8d5bef4bee0e2e53018b59e367a2a0295caeaf9ba41cafdad56442a38706a
```

The inventory schema is `private-music-technical-inventory/v1`. Each record
contains a deterministic opaque track ID, private relative and local paths,
file and canonical decoded-PCM SHA-256 values, size, codec/profile/sample
format, bitrate, sample rate, channels/layout, duration, normalized metadata for
duplicate detection, FFmpeg loudness/true-peak analysis, decode status, flags,
and suitability.

Verified aggregate results:

- 66 MP3 files; 66 probe, decode, and loudness analyses succeeded.
- 66 are technically suitable for Pilot V2 and have unique decoded audio.
- No exact-file, decoded-PCM, normalized-tag, or normalized-filename duplicate
  group was found.
- All are stereo 44.1 kHz MP3 decoded as planar float by FFmpeg.
- Duration range is 130.785–456.468 seconds; median is 231.085 seconds. None is
  shorter than 60 seconds.
- One file averages about 120.5 kbit/s. It is a warning, not a technical or
  musical exclusion. The rest are approximately 144–223 kbit/s VBR.
- Embedded identity tags are absent. Artist/album leakage avoidance therefore
  requires a small private annotation step before the split is frozen.
- Integrated loudness ranges from -16.75 to -5.26 LUFS; median is -9.30 LUFS.
- Measured true peak ranges from -2.86 to +3.16 dBTP; 57/66 tracks reach or
  exceed 0 dBTP. This is common decoder/inter-sample behavior, not corruption,
  but it makes explicit shared headroom and true-peak QC mandatory.

The target of 60 pilot tracks plus six reserves remains technically feasible.
No track should be rejected for genre, taste, loudness, or the single bitrate
warning alone.

## 5. Offline DSP feasibility and measurements

FFmpeg/ffprobe 9.0.1 is installed locally, including `librubberband`. Available
filters cover crossfades, gain automation, crossovers, EQ, low/high-pass filters,
delay/echo, side-chain compression, limiting, seeded noise, and time stretch.
SoX, Essentia, Chromaprint, SciPy, librosa, and standalone Rubber Band are not
installed; no software or system configuration was added.

The private feasibility record is outside the repository:

```text
C:\Users\janni\Desktop\spotify-transition-private-v2\offline-dsp-feasibility-v1.json
SHA-256 fd2c789edabba8ba7e850170921a8bcb30f1b604111f4c09dee4c641c6ef2317
```

The throwaway spike used synthetic signals where adequate and 16-second
excerpts from two deterministically selected private tracks when real spectral
and level behavior mattered. Excerpts and renders were temporary and are not
part of the repository or retained artifact set.

### 5.1 Operator findings

The capability class used below is:

- **A:** trivial future RPI capability using bounded gain/mix/timing work.
- **B:** probably practical on RPI with bounded real-time DSP and explicit
  latency/resource accounting.
- **C:** expensive, backend-specific, or likely offline-only unless simplified.

| Operator | Offline backend result | State/latency and hazards | Future class |
| --- | --- | --- | --- |
| Shaped/equal-power gain | Cleanly expressible; sample-deterministic | No lookahead; overlapping correlated audio can sum above unity | A |
| Asymmetric gain envelopes | Cleanly expressible | No lookahead; overly long outgoing tail recreates Pilot V1 drag | A |
| Low-pass handoff | Expressible; automation may require sampled commands | Filter state and short warm-up; resonance/automation zippering must be bounded | B |
| High-pass handoff | Expressible with the same caveat | DC/low-frequency phase behavior; thin or harsh output if overused | B |
| Complementary bass/EQ handoff | Cleanly expressible with fixed crossovers | Crossover/filter state; band recombination approached full scale in the spike | B |
| Frequency-selective overlap | Expressible with crossover/band gains | More filters and phase state; can hollow out mids or duplicate bands | B |
| Controlled ducking | Best represented as a resolved gain envelope | Explicit automation has no detector latency; dynamic side-chain would add state and backend variance | A initially; B if detector-driven later |
| Echo/delay tail | Cleanly expressible as bounded feed-forward taps | Tail buffers; feedback can run away or smear rhythm/vocals, so Phase 1 forbids feedback | B |
| Reverb-style tail | Dense multitap approximation was feasible; robust reverb is less portable | Algorithm/convolution state, tail duration, coloration, and cost vary | C; not an initial template |
| Beat/bar cut | Clean hard or click-softened cut | Needs reliable resolved cue; 5–30 ms ramp avoids clicks | A |
| Short rhythmic gating | Cleanly expressible as resolved gain breakpoints | Needs strong grid and minimum ramps; can stutter vocals or sound gimmicky | A |
| Seeded noise/riser | Deterministic seeded generation is feasible | Requires seed, band limit, low level, and purpose; easily masks rather than solves | B; not an initial template |
| Output limiter | Cleanly expressible | FFmpeg lookahead limiter requires a short delay; limiting can conceal a bad plan | B |
| Incoming time stretch | Cleanly expressible via `librubberband` | Buffered state, padding/delay compensation, and ratio constraints; dominant spike cost | B |

The per-operator contracts below make the comparison and fallback behavior
explicit. “Whole-file” means the renderer itself needs complete input, rather
than the analyzer needing a frozen feature computed earlier.

#### Shaped/equal-power gain

- **Purpose/inputs:** preserve perceived energy across a compatible overlap;
  needs only legal cues and source bounds. Parameters are duration, outgoing and
  incoming breakpoints, and interpolation enum.
- **IR/constraints:** two `gain_envelope` operations; monotonic 0–1 gain, ordered
  breakpoints, at most ten seconds for this template, no whole-file rendering
  knowledge and no latency.
- **Risk/cost/fallback/comparison:** correlated signals can exceed unity or sound
  phasey; analytic pre-gain and output QC apply. Cost is trivial offline and
  class A later. Compare against the same cue geometry with linear fallback;
  invalid plans fall back to `safe_crossfade`.

#### Asymmetric outgoing/incoming envelopes

- **Purpose/inputs:** stop outgoing material earlier or introduce incoming
  material later when drag or collision is predicted; needs cue, vocal,
  transient, and energy-window features.
- **IR/constraints:** two independent `gain_envelope` operations with resolved
  breakpoints; the outgoing curve must reach zero by the declared ownership
  point and neither curve may boost. No whole-file renderer knowledge or
  latency.
- **Risk/cost/fallback/comparison:** an aggressive shape can make an energy hole
  or sound like a cut. Cost is class A. Compare geometry-matched symmetric and
  asymmetric variants; reject to shaped symmetric or safe fallback.

#### Low-pass sweep

- **Purpose/inputs:** remove outgoing brightness/percussion while handing over
  spectral prominence; needs stable spectral occupancy and a useful cue.
- **IR/constraints:** one `filter_envelope` with low-pass kind, cutoff
  breakpoints/interpolation and bounded Q; cutoff/Q limits and non-resonance are
  validator-enforced. Stateful streaming filter, no lookahead or whole-file
  rendering knowledge.
- **Risk/cost/fallback/comparison:** can become muffled, zipper, or color sparse
  music; filters may change peak level. Cost is low offline and class B later.
  Compare to the same gain-only handoff; missing spectral data uses shaped
  fallback.

#### High-pass sweep

- **Purpose/inputs:** vacate outgoing bass or admit incoming upper rhythm without
  full-band stacking; needs bass/spectral occupancy and legal timing.
- **IR/constraints:** one high-pass `filter_envelope`; the same cutoff/Q and
  automation bounds apply. Stateful, no lookahead or whole-file knowledge.
- **Risk/cost/fallback/comparison:** can sound thin or expose harsh transients;
  coefficient changes and phase response can affect peaks. Cost is low/class B.
  Compare geometry-matched gain-only audio; invalid or purposeless use falls
  back to shaped handoff.

#### Complementary bass/EQ handoff

- **Purpose/inputs:** ensure only one track owns kick/bass during overlap; needs
  bass occupancy, onset density, cue, and preferably a reliable beat/bar grid.
- **IR/constraints:** fixed `crossover_band_gain` operations with one resolved
  crossover and complementary low-band envelopes; only the named crossover
  order is allowed and incoming/outgoing low bands may not both be at unity.
  Stateful, no lookahead or whole-file renderer knowledge.
- **Risk/cost/fallback/comparison:** crossover phase/coloration, a bass hole, or
  recombination peak. It measured low offline cost and is class B later. Compare
  against identical geometry rendered full-band; missing features or validation
  failure uses shaped handoff.

#### Frequency-selective overlap

- **Purpose/inputs:** divide broader spectral ownership when full-band overlap
  would be dense; needs low/mid/high occupancy and collision estimates.
- **IR/constraints:** one or two fixed crossover points and bounded complementary
  `crossover_band_gain` envelopes; at most three bands and no arbitrary EQ
  topology. Stateful, no lookahead or whole-file knowledge.
- **Risk/cost/fallback/comparison:** hollow mids, duplicated bands, phase shift,
  and near-clipping recombination. Cost remained small offline and is class B.
  Compare a geometry-matched full-band candidate; otherwise use bass, shaped,
  or safe fallback.

#### Controlled ducking

- **Purpose/inputs:** suppress one localized vocal or transient collision without
  discarding a short otherwise useful overlap; needs time-local vocal/onset
  features.
- **IR/constraints:** resolved `duck_envelope` depth and attack/hold/release,
  never backend detector settings in v1; -12 to 0 dB and 20–200 ms ramps. No
  lookahead, whole-file knowledge, or signal-dependent runtime state.
- **Risk/cost/fallback/comparison:** pumping or an audible level dent; it reduces
  rather than increases clipping risk. Cost is trivial/class A. Compare the
  same overlap with and without the resolved duck; sustained collisions choose
  beat cut or shaped fallback.

#### Echo/delay tail

- **Purpose/inputs:** cut outgoing dry audio promptly while leaving short rhythmic
  continuity; needs beat period, cue confidence, vocal probability, and density.
- **IR/constraints:** `feedforward_delay_tail` with one to three explicit delay
  and gain pairs, bounded total wet gain, no feedback, and a six-second/two-bar
  tail cap. Requires a bounded delay buffer but not whole-file knowledge.
- **Risk/cost/fallback/comparison:** lyric repetition, rhythmic smear, combing,
  and additive tail peaks. Cost is low offline/class B. Compare a dry beat cut
  at the identical cue; dense or invalid material uses that cut or shaped
  fallback.

#### Reverb-style tail

- **Purpose/inputs:** potentially soften an exposed dry cut on sparse tonal
  material; it would need density, transient, vocal, and estimated room/tail
  suitability features.
- **IR/constraints:** reserved `reverb_tail` semantics use predelay, RT60,
  damping, wet gain, seed/profile, and tail bound—never an FFmpeg algorithm
  string. A real implementation carries algorithm-specific state; convolution
  may need an impulse response and substantial buffering.
- **Risk/cost/fallback/comparison:** coloration, washed rhythm/vocals, long tails,
  backend mismatch, and wet/dry clipping. The spike only proved a multitap
  approximation, so the full operator is class C and has no initial template.
  Any future test must compare against echo and dry cut; fallback is echo/cut.

#### Beat/bar-aligned hard or softened cut

- **Purpose/inputs:** establish immediate ownership and stop rhythmic drag; needs
  high-confidence beat/downbeat/phrase cues and source bounds.
- **IR/constraints:** resolved timing plus two short `gain_envelope` ramps;
  click-softening is 10–30 ms and no renderer beat detection occurs. No latency
  or whole-file knowledge.
- **Risk/cost/fallback/comparison:** a wrong downbeat or sustained vocal makes the
  edit conspicuous; clipping risk is minimal. Cost is trivial/class A. Compare
  exact and softened cuts at the same cue; weak grids use shaped fallback.

#### Short rhythmic gating/cuts

- **Purpose/inputs:** turn a strong outgoing groove into a brief intentional exit;
  needs high beat confidence, transient strength, and a vocal-free window.
- **IR/constraints:** `rhythmic_gate` contains resolved gain breakpoints, minimum
  five-ms edges, quarter/eighth-beat pattern, maximum 8 Hz and two bars. No
  lookahead or whole-file knowledge.
- **Risk/cost/fallback/comparison:** clicks if ramps fail, vocal fragmentation,
  or novelty/stutter. Peaks do not increase. Cost is trivial/class A. Compare
  against a beat cut at the same cue; any predicate failure uses beat cut or
  shaped fallback.

#### Seeded noise/riser masking

- **Purpose/inputs:** only to cover a short unavoidable low-level spectral seam;
  requires a detected seam plus energy, spectrum, and vocal-free-region data.
- **IR/constraints:** `seeded_noise_mask` fixes PRNG algorithm/seed, color/band,
  envelope, and level at or below -18 dBFS; duration is bounded and no external
  sample is referenced. Streaming generation has no lookahead or whole-file
  knowledge.
- **Risk/cost/fallback/comparison:** it can sound artificial, raise peaks, and
  conceal a bad plan rather than repair it. Compute is bounded/class B, but no
  initial template is proposed. A future ablation must compare the identical
  seam with no mask; fallback removes the mask or chooses another handoff.

#### Output limiter/headroom protection

- **Purpose/inputs:** enforce the preview safety ceiling after predicted and
  measured summation; needs source true peak, plan gain bounds, and post-render
  meters.
- **IR/constraints:** `output_safety` declares shared pre-gain, target, lookahead,
  release, and rejection thresholds. The limiter is mandatory policy but not a
  musical candidate differentiator. It requires 1–10 ms lookahead/buffering;
  true-peak verification may use a post-render pass.
- **Risk/cost/fallback/comparison:** excessive limiting changes punch and can
  make unsafe candidates look acceptable. Cost was low offline/class B. Apply
  the same policy to every condition and reject rather than compare plans beyond
  the reduction/activity bounds; fallback is independently safety-checked.

#### Incoming time stretch interaction

- **Purpose/inputs:** preserve beat alignment under a small tempo difference;
  needs reliable BPM/grid and a resolved tempo ratio.
- **IR/constraints:** one constant pitch-preserving incoming `time_map`, limited
  to 0.92–1.08 in v1. The render algorithm requires buffered state and explicit
  padding/delay compensation but not whole-file knowledge in a real-time mode;
  higher-quality offline modes may inspect more audio.
- **Risk/cost/fallback/comparison:** artifacts, transient smearing, latency, and
  backend differences. It was the dominant but still practical offline cost and
  is class B later. Candidate comparisons record ratio/profile and preferably
  share geometry; a ratio outside bounds rejects beat-dependent templates and
  falls back to a non-beat handoff.

Filter cutoff sweeps were realized with deterministic command sampling. That is
adequate for feasibility but is exactly why backend syntax must not be the IR:
the plan should describe a cutoff envelope and interpolation, while a backend
chooses safe command resolution or native coefficient interpolation.

Reverb and noise are technically possible but did not earn inclusion from Pilot
V1 evidence. A real reverb abstraction also hides materially different
algorithms. Both remain validated operator concepts or future experiments, not
initial candidate templates.

### 5.2 Headroom observations

Private-material band recombination and frequency-selective overlaps produced
peaks around -0.1 and -0.2 dBFS even with conservative source levels. The
combined stretch/filter/limiter case ended at its -1.0 dBFS ceiling. These
measurements support the following policy:

1. Apply one non-boosting, pair-shared preview gain so loudness is not a
   condition-specific cue in blinded evaluation.
2. Compute a conservative analytic bound before rendering.
3. Measure sample and true peak after rendering.
4. Use a recorded limiter as a safety net, not a loudness maximizer.
5. Reject a candidate requiring more than 3 dB of limiter reduction or showing
   sustained limiting on more than 5% of measured frames.

### 5.3 Performance bounds on U-01

Fifteen 16-second representative programs were timed on the Ryzen 7 9800X3D.
Single-threaded, bit-exact-oriented FFmpeg settings were used:

- simple gain, cut, filter, duck, echo, gate, noise, and limiter renders were
  typically 0.01–0.05 seconds;
- incoming time stretch took about 0.26 seconds;
- time stretch plus filter and limiter took about 0.27 seconds;
- median render/audio ratio was 0.000875, about 1,140 times real time;
- the maximum ratio was 0.016688, about 60 times real time;
- maximum measured resident set was 39,012 KiB (about 38.1 MiB);
- the largest 16-second FLAC artifact was 2,798,456 bytes (about 2.67 MiB).

Short-process CPU percentages were too quantized to be useful. Wall-time ratio
and resident-set size are the appropriate architecture-level bounds.

For 64 36-second candidates rendered serially, extrapolated render time is
about 2 seconds at the median, about 38.5 seconds if every candidate were as
expensive as the worst combination, and about 6.6 seconds for a plausible mix
of eight expensive and 56 simple candidates. Feature extraction will likely
dominate. Rendering at this scale is practical locally.

Persisting every candidate is less attractive: at the observed median density,
64 previews are roughly 346 MiB per pair and 30 pairs approach 10 GiB. The
pipeline should stream, hash, and discard unselected intermediates, retaining
the frozen blinded set and compact QC/provenance records.

## 6. Determinism and artifact identity

Three representative programs—equal-power gain, frequency-selective overlap,
and a time-stretch/filter/limiter combination—were each rendered three times.
Both FLAC file hashes and canonical decoded 24-bit, 44.1 kHz stereo PCM hashes
matched on every repeat.

The spike used:

```text
-threads 1
-filter_threads 1
-filter_complex_threads 1
-fflags +bitexact
-flags:a +bitexact
-map_metadata -1
```

This establishes repeatability on the same backend/build/machine. It does not
prove that floating-point DSP or encoded container bytes are identical across
FFmpeg versions, CPU architectures, or future renderers. Therefore:

- the canonical candidate identity is derived before rendering from input audio
  hashes, frozen feature snapshot hash, canonical `OperatorPlan`, generator
  version/configuration, and renderer capability target;
- the render record separately stores renderer identity/version, backend-program
  hash, encoded artifact hash, and canonical decoded-PCM hash;
- decoded-PCM identity is preferred for same-audio comparison;
- a new renderer build is a new rendering provenance even if plan identity is
  unchanged;
- pilot assets are frozen after rendering and hash-checked before rating.

## 7. Canonical data model

The model has three deliberately separate layers:

```text
MusicalTemplate
  applicability rules + bounded named variant recipes + explanation
                |
                v deterministic resolution
OperatorPlan
  fully resolved source identities, timing, typed operations, safety contract
                |
                v backend capability check and lowering
RenderProgram
  FFmpeg graph today; a possible RPI program later
```

The critic receives only already-generated, already-validated candidates. It
may assign a score and select a candidate ID. It cannot emit operations,
parameters, source positions, or a render program, and cannot bypass validation.

### 7.1 `OperatorPlan` envelope

Conceptual `transition-operator-plan/v1` fields are:

```text
schema_version
template { id, version }
sources {
  outgoing/incoming {
    opaque_track_id, audio_sha256, analysis_sha256,
    source_start_us, cue_position_us, source_duration_us
  }
}
timeline {
  transition_duration_us, handoff_offset_us, tail_end_offset_us,
  outgoing/incoming resolved time maps
}
operations [ bounded typed operations ]
output_safety { mix_pregain_mdb, true_peak_target_mdbtp, limiter policy }
feature_snapshot { schema_version, sha256 }
provenance { generator_id, generator_version, config_sha256, seed }
required_capabilities [ derived, not trusted input ]
```

The resolved plan contains no ranges, unresolved beat references, callback,
expression language, plugin name, filter string, or general graph edge.

### 7.2 Flat operations and fixed stage order

Each operation has an `op_id`, an enumerated `kind`, a target
(`outgoing`, `incoming`, or `mix`), an integer time window, and kind-specific
validated parameters. The schema fixes processing order:

1. source trim and time map;
2. source spectral shaping;
3. source dynamics, duck, and rhythm;
4. source tail generation;
5. source gain envelopes;
6. fixed source summation;
7. optional generated mask source;
8. output safety and measurement.

There are no arbitrary connections or references between operations. Suggested
v1 limits are 12 operations total, at most two time maps, at most two
filter/crossover operations per source, at most six gain/band envelopes, one
duck per source, one tail, one rhythmic gate, one noise mask, and exactly one
output-safety policy.

Typed operation variants are:

- `time_map`: constant, pitch-preserving incoming rate only in v1;
- `gain_envelope`: ordered breakpoints and named interpolation;
- `filter_envelope`: low-pass or high-pass cutoff automation;
- `crossover_band_gain`: fixed two- or three-band complementary gain;
- `duck_envelope`: resolved attenuation, never an opaque detector in v1;
- `feedforward_delay_tail`: explicit bounded taps, no feedback in v1;
- `reverb_tail`: reserved semantic fields, unsupported by the initial template
  set;
- `rhythmic_gate`: resolved click-softened gain pattern;
- `seeded_noise_mask`: seeded, band-limited signal and gain envelope;
- `output_safety`: shared pre-gain, true-peak target, and limiter policy.

### 7.3 Numeric and canonical representation

Canonical fields use integers: microseconds, hertz, milli-Q, parts-per-million
rate/gain, and millidecibels. This excludes NaN, infinity, locale formatting,
and ambiguous floating-point JSON. Envelope interpolation is an enum such as
`hold`, `linear`, `smoothstep`, or `equal_power_sine`; breakpoints must be
strictly ordered.

Canonical JSON uses UTF-8, sorted keys, no insignificant whitespace, explicit
defaults, and no unknown members. The plan SHA-256 excludes any self-hash field.
A validator version is recorded separately because accepting a plan and
identifying its semantic contents are different concerns.

### 7.4 Candidate audit record

`transition-candidate/v2` surrounds the plan with evidence required for fair
ranking and reproducibility:

- candidate ID and canonical plan SHA-256;
- generator ID/version/configuration/seed;
- input opaque IDs, audio hashes, and analysis hashes;
- feature snapshot schema/hash and the candidate feature vector;
- template ID/version and a plain-English explanation;
- resolved `OperatorPlan`;
- validator version, checks, capability result, and rejection reasons;
- heuristic and critic scores with scorer/model versions and artifact hashes;
- renderer ID/version/capability profile and backend-program hash;
- encoded artifact and decoded-PCM hashes, duration, format, peak, loudness,
  limiter activity, and render status;
- abstract compatibility with the current runtime plan:
  `LOSSLESS_CURRENT_TRANSITION_PLAN` or `REQUIRES_EXTENDED_RENDERER`.

The record may refer to a private feature manifest by hash. It must not leak a
local music path into a public or blind artifact.

Gain-only overlap and incoming speed can be classified as losslessly mappable to
the current `TransitionPlan`; all other operations require a future renderer.
Phase 1 does not perform that mapping or modify the current type.

## 8. Recommended initial semantic templates

Nine templates are enough to test distinct hypotheses without becoming a DSP
showcase. Parameter recipes are ordered, zipped presets—not Cartesian products.

### 8.1 `safe_crossfade/v1` — quota 1, class A

- **Purpose:** independent guaranteed fallback and experimental control.
- **Conditions:** any two decodable tracks with legal source bounds.
- **Slots:** outgoing/incoming linear gain and output safety.
- **Recipe:** one fixed, documented ordinary crossfade, nominally five seconds,
  shortened only to satisfy source bounds.
- **Failure:** can be boring or collide; it nevertheless remains available.
- **Explanation:** “A plain full-band crossfade used as the safety baseline.”

### 8.2 `shaped_handoff/v1` — quota 8, class A

- **Purpose:** prevent energy holes and outgoing drag without adding an effect.
- **Conditions:** reliable cues; beat grid optional for second-based variants.
- **Slots:** equal-power or smoothstep gain envelopes with asymmetric timing.
- **Recipes:** 3 or 5 seconds, or 2 or 4 bars when the grid is reliable;
  outgoing reaches silence no later than the incoming musical arrival; maximum
  overlap 10 seconds.
- **Incompatible:** cues too close to bounds or strongly predicted vocal clash.
- **Failure:** correlated material can sum loudly; long tails recreate V1 drag.
- **Fallback:** safe crossfade.
- **Explanation:** “A shaped, possibly asymmetric handoff that gives the new
  track ownership earlier than a linear crossfade.”

### 8.3 `beat_cut/v1` — quota 4, class A

- **Purpose:** avoid full-band collision and stop old percussion cleanly.
- **Conditions:** high-confidence bar/downbeat or phrase cues on both tracks;
  legal tempo relation is helpful but overlap is not required.
- **Slots:** time placement and 10–30 ms click-softening ramps.
- **Recipes:** exact cut, short softened cut, and at most one half-beat handoff.
- **Incompatible:** weak grid, sustained outgoing vocal, or cue uncertainty.
- **Failure:** a confidently wrong downbeat is more obvious than a crossfade.
- **Fallback:** shaped handoff, then safe crossfade.
- **Explanation:** “A clean bar-aligned handoff with almost no simultaneous
  audio.”

### 8.4 `bass_handoff/v1` — quota 8, class B

- **Purpose:** prevent kick and bass collision while permitting upper-band
  continuity.
- **Conditions:** reliable 2/4-bar grid and measurable low-band occupancy.
- **Slots:** fixed Linkwitz-Riley-style two-band crossover, complementary bass
  gain, shaped full-band gain, and output safety.
- **Recipes:** crossover at 140, 180, or 220 Hz in ordered variants; outgoing
  bass reaches zero before incoming bass reaches unity; overlap at most 12 s.
- **Incompatible:** unreliable bass estimate, weak timing, or tiny low-band
  content where the operation has no purpose.
- **Failure:** phase/coloration near crossover or a perceptible thin interval.
- **Fallback:** shaped handoff.
- **Explanation:** “The incoming track receives the bass lane before the rest
  of the handoff completes.”

### 8.5 `spectral_handoff/v1` — quota 8, class B

- **Purpose:** replace excessive full-band overlap with deliberate frequency
  ownership.
- **Conditions:** stable spectral occupancy and reliable cues; 2/4-bar grid for
  rhythmic variants.
- **Slots:** low/high-pass envelope or fixed three-band stagger, plus gain.
- **Recipes:** a small set of non-resonant cutoff trajectories or staggered
  band gains over at most 12 seconds.
- **Incompatible:** severe vocal clash, very sparse material where filtering is
  conspicuous, or missing spectral features.
- **Failure:** hollow mids, harshness, or zippering if backend automation is too
  coarse.
- **Fallback:** shaped handoff.
- **Explanation:** “Frequency ranges transfer in stages instead of both tracks
  occupying the whole spectrum.”

### 8.6 `ducked_overlap/v1` — quota 8, class A

- **Purpose:** retain a short compatible overlap while suppressing a localized
  vocal or transient collision.
- **Conditions:** localized collision predicted from vocal/transient features;
  otherwise the template has no purpose.
- **Slots:** resolved gain duck and shaped handoff.
- **Recipes:** -6 or -9 dB, 20–200 ms attack/release, 2 or 4 bars maximum.
- **Incompatible:** sustained dual vocals, pervasive rhythmic mismatch, or a
  need for detector-driven behavior.
- **Failure:** pumping or an audible level dent.
- **Fallback:** beat cut when reliable, otherwise shaped handoff.
- **Explanation:** “One track briefly steps back at the specific collision
  instead of both playing at full prominence.”

### 8.7 `echo_tail_handoff/v1` — quota 4, class B

- **Purpose:** end outgoing dry material promptly while preserving a short,
  intentional sense of continuity.
- **Conditions:** clean/sparse outgoing cue, reliable beat period, and low risk
  of vocal or dense-percussion smear.
- **Slots:** dry cut plus one to three explicit beat-synchronous delay taps.
- **Recipes:** quarter/half-beat tap spacings, monotonically decreasing wet
  gains, no feedback, tail at most two bars or six seconds.
- **Incompatible:** dense or vocal-heavy endings, weak tempo, or ambiguous cue.
- **Failure:** rhythmic clutter, repeated lyric fragments, or audible combing.
- **Fallback:** beat cut or shaped handoff.
- **Explanation:** “The outgoing track stops cleanly and leaves only a few
  quiet, beat-timed echoes.”

### 8.8 `energy_ramp/v1` — quota 6, class A/B

- **Purpose:** bridge a measured energy/loudness mismatch without boosting or
  crushing the incoming track.
- **Conditions:** stable pre/post cue energy estimates and meaningful mismatch.
- **Slots:** asymmetric non-boosting gain; optional non-resonant filter envelope.
- **Recipes:** 2/4 bars or 3/5 seconds, with bounded staged ownership changes.
- **Incompatible:** no mismatch, volatile dynamics, or a required gain boost.
- **Failure:** sounds like a level correction rather than a musical transition.
- **Fallback:** shaped handoff.
- **Explanation:** “The handoff changes density in stages so the energy step is
  less abrupt.”

### 8.9 `rhythmic_handoff/v1` — quota 4, class A

- **Purpose:** turn a strong outgoing rhythm into a short, deliberate handoff
  rather than letting it drag.
- **Conditions:** strong transient grid, high beat confidence, and a vocal-free
  outgoing window.
- **Slots:** resolved outgoing gain gate and final cut.
- **Recipes:** quarter- or eighth-beat gates for one, exceptionally two, bars;
  all edges have at least 5 ms ramps.
- **Incompatible:** vocals, weak rhythm, unstable tempo, or more than two bars.
- **Failure:** gimmicky stutter or loss of groove.
- **Fallback:** beat cut or shaped handoff.
- **Explanation:** “A very short click-safe rhythmic pattern signals the old
  track’s exit on the beat.”

### 8.10 Deferred vocabulary

- `seeded_noise_mask` remains available to experiments but has no initial
  template. Masking should not conceal a timing or collision defect.
- `reverb_tail` remains class C until a specific simplified algorithm and an
  audible use case outperform the echo-tail control.
- Stem-aware handoff is outside Phase 1. It adds a heavyweight learned analysis,
  new artifact lifecycle, separation failures, and a difficult runtime mapping
  before simpler band/vocal controls have been tested.
- No generic “combine any two operators” template is allowed. Every template
  already includes the minimum gain and safety operations needed to realize one
  musical strategy. A spectral-plus-duck hybrid can be proposed later only as a
  named, bounded ablation if both parents earn their place.

## 9. Bounded candidate generation

Generation is deterministic and finite:

1. Validate source identities and feature snapshot.
2. Build at most 12 cue/geometry entries, stratified across useful duration and
   cue types rather than selected only by one scalar score.
3. Evaluate all nine applicability predicates.
4. Retain the fallback plus at most six applicable non-fallback families.
5. Instantiate each retained template from its ordered zipped recipes and quota.
6. Validate, canonicalize, and deduplicate by `OperatorPlan` hash.
7. Render valid candidates; reject technical failures and deduplicate ordinary
   same-audio outputs by decoded-PCM hash.
8. Rank the exact surviving set with heuristics and, experimentally, the critic.

With the quotas above, the largest possible set from six optional high-quota
families is 43 candidates including fallback. The overall soft cap is 48 and
the validator hard cap is 64. The hard cap protects against configuration bugs,
not an invitation to fill every pair with 64 variants.

When more than six optional families apply, deterministic priority uses
applicability strength, then template ID. Recipe ordering, geometry tuples, and
plan hashes break later ties. No random choice occurs unless a seed is explicit
in the generator provenance.

Hard rejection order is:

1. schema, version, input identity, and source bounds;
2. required feature presence and validity;
3. cue, beat, tempo, and template applicability;
4. operation count, numeric, and capability limits;
5. analytic collision and headroom limits;
6. static CPU, latency, tail, and storage limits;
7. render health, finite output, duration, true peak, and limiter activity;
8. plan and decoded-audio deduplication.

The fallback is produced on a separate minimal path and cannot be pruned because
an advanced feature or backend capability is absent.

## 10. Feature requirements and YAGNI boundary

### Already available in the current repository path

- beats/downbeats and confidence;
- BPM and inferred tempo relation;
- fade-in/fade-out cue points;
- vocal activity;
- Camelot/key and harmonic compatibility inputs;
- genre concepts/mixability;
- duration and item speeds;
- per-bar confidence and peak-loudness-related analysis.

### Cheap offline additions for Phase 1

- integrated/short-term loudness, sample/true peak, crest factor;
- per-bar RMS/energy and energy slope;
- low/mid/high band energy fractions and bass occupancy;
- spectral centroid, rolloff, and flatness;
- onset/transient density and strength;
- plan-window collision estimates for vocals, bass, transients, and spectral
  occupancy.

These features are deterministic transforms of local audio and fit a frozen,
versioned feature snapshot. The exact algorithms and windows belong in the
formal specification after the local Pilot V1 feature schema is audited.

### Moderate/new, required only if not recovered from existing offline work

- private-track beat/downbeat extraction;
- private-track key estimation;
- private-track vocal probability.

These may require an existing S-01 environment or a deliberate tool choice.
They should not be reimplemented speculatively during architecture work.

### Unnecessary for Phase 1

- stem/source separation;
- neural genre or lyric semantics;
- dense musical embeddings;
- reverb-type classification;
- arbitrary effect recommendation.

## 11. Fair comparison by the critic and human pilot

The deterministic selector and learned critic must receive the identical valid,
deduplicated candidate set. Fairness requires:

- template identity and normalized resolved parameters in candidate features;
- separate input-musical, predicted-realization, and post-render-QC feature
  groups, with no human-rating leakage;
- per-template coverage, rejection, selection, and win-rate reporting;
- quotas so a family cannot win merely by emitting more parameter combinations;
- geometry-matched ablations where the same cues are rendered by fallback and a
  richer strategy;
- separate scorer ID/version/model artifact hash and raw/calibrated score;
- same-audio deduplication before ordinary human comparisons, except deliberately
  seeded identical controls.

The critic returns only `{candidate_id, score}` over the valid set. A missing,
invalid, incompatible, or rejected plan is never exposed to it.

## 12. Refined private Pilot V2 design

All 66 files are technically eligible, so retain the proposed 60-track design
with six reserves:

1. Create a frozen private manifest with opaque IDs/hashes, technical QC,
   derived features, and privately verified artist/album/style labels. Because
   embedded tags are absent, the identity annotation must be completed before
   splitting.
2. Use a recorded seed and constrained optimizer to choose 60 tracks that cover
   the acoustic/style clusters; select six stratified reserves. Do not listen
   and hand-pick for operator success.
3. Form 30 track-disjoint pairs: ten naturally compatible, ten moderate, and ten
   deliberately awkward but still legally renderable. Compatibility distance
   uses tempo/stretch, key, energy, bass/spectral occupancy, vocal overlap, and
   cue quality. “Awkward” requires at least two mismatches, not corruption or an
   impossible cue.
4. Split 18/6/6 pairs into development/validation/test, respectively 6/2/2
   pairs from each compatibility stratum. Keep artist/album groups disjoint
   across splits where the private annotation makes that practical.
5. Freeze tracks, pairs, splits, features, generator configuration, and candidate
   sets before rendering or rating.

The three core conditions should be:

1. plain `safe_crossfade` fallback;
2. deterministic selection from the richer vocabulary;
3. learned-critic selection from the exact same valid richer candidate set.

This isolates two questions: whether the vocabulary adds value, and whether the
critic adds value beyond deterministic selection.

The core workload is 90 pair-condition presentations. Add 12 exact hidden
duplicate controls chosen before rating—six development, three validation, and
three test, balanced by condition and UI position—for 102 total presentations,
only about 13% extra. Use sessions of roughly five pairs with natural breaks.

The UI should support ordered tie groups directly: drag samples into a shared
rank group or use an explicit “tie with previous” action, then show a compact
confirmation before submission. Retain:

- ordered preference with explicit ties;
- overall `good` / `meh` / `bad`;
- technical smoothness;
- musical intent;
- confidence;
- bounded issue flags and optional short comment.

Define smoothness as technical cleanliness and intent as a deliberate musical
handoff. Do not add many more rating dimensions. If a collision taxonomy is
useful, keep it as flags rather than another numeric score.

Predeclare the primary interpretations:

- richer deterministic selection versus fallback tests vocabulary value;
- critic versus richer deterministic selection tests learned ranking value;
- test is inspected only after one validation decision;
- any claimed improvement must exceed Pilot V2’s own duplicate-control noise.

With only six validation and six test pairs, results are directional evidence,
not a broad population-significance claim.

## 13. Future RPI lowering contract

A future renderer advertises a versioned `renderer-capabilities/v1` document:

```text
renderer identity/version
sample rates, channel layouts, block sizes
maximum total latency and operation count
gain_envelope { curves, max_points }
filter { kinds, cutoff_range, q_range, automation_resolution }
crossover_eq { max_bands, supported_orders }
duck { explicit_envelope, optional_detector_modes }
delay { max_taps, max_delay_us, max_wet_gain, feedback_support }
reverb { named_profiles and bounds }
rhythmic_gate { minimum_ramp_us, maximum_rate }
noise { colors, deterministic_seed_support }
time_stretch { ratio_range, profiles, latency }
limiter { modes, lookahead_range, true_peak_support }
```

Lowering the same `OperatorPlan` returns exactly one status:

- `SUPPORTED`: exact semantic lowering under the declared bounds;
- `SUPPORTED_WITH_SIMPLIFICATION`: a named, explicit transformation is
  proposed, with original plan hash, transformed plan/hash, capability hash,
  and reason;
- `UNSUPPORTED`: no valid lowering.

No renderer silently approximates a curve, drops an operation, changes a
filter, substitutes a reverb, or omits a limiter. A simplified plan is a new
plan that must be rendered offline, measured, and explicitly selected. Runtime
selection always retains the plain crossfade fallback.

This contract is intentionally designed now; no RPI backend, current playback
mapping, or runtime capability negotiation is part of Phase 1.

## 14. Validation, safety, and failure behavior

All serialized plans pass an independent strict validator before any renderer or
critic sees them. Proposed Phase 1 bounds are:

- reject unknown schema versions, unknown members, duplicate IDs, oversized
  documents, malformed hashes, and any non-integer numeric field;
- require nonnegative, monotonic microsecond timing within verified source
  durations; transition at most 16 seconds, tail at most six seconds, and pilot
  preview at most 36 seconds;
- gain 0–1,000,000 ppm and no boost in Phase 1;
- one constant pitch-preserving time-map ratio from 920,000–1,080,000 ppm;
- filter cutoff 20 Hz to the lesser of 18 kHz and 45% of sample rate, Q
  500–1,000 milli, no resonance boost;
- one or two crossover points and one fixed fourth-order complementary crossover
  family in Phase 1;
- duck depth -12 to 0 dB, attack/release 20–200 ms;
- at most three feed-forward delay taps, each 20–2,000 ms, total wet gain at
  most 0.8, no feedback, tail at most six seconds;
- reserved reverb bounds of RT60 at most three seconds, predelay at most 100 ms,
  and wet gain at most 0.25, but no initial reverb template;
- rhythmic gate edge ramps at least 5 ms, rate at most 8 Hz, duration at most
  two bars;
- noise requires an explicit seed, band limit, and level at or below -18 dBFS;
- limiter ceiling at or below -1 dBTP, lookahead 1–10 ms, release 20–200 ms;
- no more than 12 operations per plan, 48 candidates normally, 64 candidates
  as an absolute generator/validator ceiling.

Every envelope must have an allowed interpolation mode, ordered times, finite
integer values, and a defined value at both ends of its active interval. The
validator derives capabilities from operations; a plan cannot under-declare its
requirements.

Missing features suppress dependent templates. A renderer version or capability
mismatch fails closed. Backend errors, unexpected duration, non-finite samples,
excess peak, excessive limiter use, or hash mismatch reject the candidate. The
fallback uses a minimal separately tested rendering path.

## 15. Recommended delivery sequence after approval

This is sequencing guidance, not the implementation plan:

1. Audit the existing private/offline Pilot V1 feature schema when S-01 is next
   available; choose the minimum Phase 1 feature additions.
2. Write and approve the formal versioned schemas, validators, template recipes,
   canonicalization rules, and golden vectors.
3. Implement the offline generator and reference renderer only, test-first.
4. Run synthetic, deterministic, safety, and bounded-cost tests.
5. Perform a small private pre-pilot listening check to remove broken or
   purposeless templates; do not train or tune the critic on this check.
6. Freeze and build Pilot V2 only after the vocabulary and evaluation protocol
   are independently approved.
7. Consider an RPI renderer only for operators that survive blinded preference.

Rollback for Phase 1 is deletion or disablement of an isolated offline tool and
its private artifacts. Current runtime behavior is unaffected.

## 16. Unresolved questions requiring human approval

The following decisions should be made together at the detailed-design gate:

1. Approve the nine-template initial vocabulary and the explicit deferral of
   reverb, noise/riser, and stems.
2. Approve the flat fixed-stage `OperatorPlan` model, integer canonical units,
   v1 limits, and “no arbitrary graph” rule.
3. Approve the 48 soft / 64 hard candidate caps and per-template quotas.
4. Approve the non-boosting shared-gain, -1 dBTP safety, and limiter-rejection
   policy for blinded renders.
5. Approve the 60-track, 10/10/10 compatibility strata, 18/6/6 split, three
   conditions, and 12 duplicate controls for Pilot V2 planning.
6. Decide whether `energy_ramp/v1` is sufficiently distinct for the initial
   pilot or should be treated as a shaped-handoff recipe.
7. Decide whether the exact ordinary fallback should reproduce Pilot V1’s
   baseline duration or use one new fixed five-second baseline for Pilot V2.

Recommended decision: approve the architecture with `energy_ramp/v1` retained
as a separately reported family for ablation, and define one fixed five-second
`safe_crossfade/v1` as the Pilot V2 control. This yields a useful test of
vocabulary breadth while keeping every candidate bounded and explainable.

## 17. References

- Pilot evidence: `docs/MTG_JAMENDO_PILOT_V1_BLIND_ANALYSIS.md`
- FFmpeg filter capabilities and parameter semantics:
  <https://ffmpeg.org/ffmpeg-filters.html>
- Rubber Band offline/real-time integration and latency considerations:
  <https://www.breakfastquay.com/rubberband/integration.html>
- Existing repository architecture:
  `connect/src/spotify_auto_mix.rs`,
  `connect/src/spotify_auto_mix_selection.rs`,
  `connect/src/spotify_mix.rs`,
  `connect/src/spotify_materialized_transition.rs`, and
  `playback/src/transition.rs`
