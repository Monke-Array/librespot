# Deterministic Offline Transition Operator Design

Date: 2026-09-08

Status: **formal specification awaiting human approval**

## 1. Purpose, authority, and scope

This specification defines Architecture A for Phase 1 of the transition-quality
system:

```text
semantic musical template
  -> resolved, validated, canonical OperatorPlan
  -> offline reference-renderer program
  -> rendered artifact and measurements
```

It supersedes design choices in
`docs/TRANSITION_OPERATOR_DESIGN_PREPARATION.md` where this document is more
precise. That report remains the evidence and rationale record.

The words **MUST**, **MUST NOT**, **SHOULD**, and **MAY** are normative.

Phase 1 is offline-only. This specification does not authorize or define an RPI
renderer, runtime model integration, Pilot scheduling, queue/context ownership state,
decoder or buffer ownership, promotion, reconnect handling, or changes to the
existing `TransitionPlan`, `TransitionEngine`, or player. Spotify preset IDs and
opaque Spotify effect semantics are not inputs to this system.

The first implementation covers exactly nine templates:

1. `safe_crossfade/v1`;
2. `shaped_handoff/v1`;
3. `beat_cut/v1`;
4. `bass_handoff/v1`;
5. `spectral_handoff/v1`;
6. `ducked_overlap/v1`;
7. `echo_tail_handoff/v1`;
8. `energy_ramp/v1`;
9. `rhythmic_handoff/v1`.

Reverb, generated noise/riser masking, stems, arbitrary hybrids, arbitrary DSP
graphs, and learned parameter generation are out of scope. The IR reserves no
executable escape hatch for them.

## 2. Architectural boundaries

Each component has one owner and a serializable boundary:

| Component | Owns | Does not own |
| --- | --- | --- |
| Feature extractor | Deterministic track/cue/window analysis and feature snapshot | Cue choice, DSP, candidate score, split assignment |
| Cue/geometry proposer | Legal source cues, alignments, durations, tempo relation | Operator choice, DSP parameters, ranking |
| Template generator | Applicability and bounded named recipes | Feature computation, arbitrary parameter search, rendering |
| Plan validator | Schema, bounds, template conformance, safety fields, and derived capability requirements | Musical ranking or error recovery by mutation |
| Canonicalizer | One byte representation and identities | Validation repair or semantic defaults |
| Offline renderer | Exact source lookup, lowering, DSP, measurement, artifact bytes | Candidate generation, selection, playback state |
| Measurement/QC | Loudness, peaks, limiter activity, duration, hashes, rejection | Musical preference or plan changes |
| Candidate feature builder | Frozen comparable features for valid rendered candidates | DSP generation or access to human outcomes |
| Critic/ranker | Scores and selects IDs from one frozen valid set | Operations, parameters, cue positions, validation, runtime state |
| Human evaluator | Blind presentation and immutable ratings export | Unblinding, training, candidate regeneration |
| Future runtime lowering | Declared capability comparison and explicit lowering result | Silent approximation or playback ownership |

Normative data flow is:

```text
canonical source identities
  -> versioned FeatureSnapshot
  -> versioned GeometryProposal set
  -> template applicability and recipes
  -> candidate-local OperatorPlan validation
  -> canonical plan/hash and semantic deduplication
  -> reference rendering and post-render QC
  -> frozen CandidateRecord set
  -> deterministic scorer and/or critic scores
  -> blind Pilot V2 artifacts
```

Only the candidate-local path from template generation through post-render QC
may reject an individual rich candidate. No rich-candidate failure may change
audio playback or invalidate an already-valid fallback.

## 3. Common scalar conventions

### 3.1 Audio and timebase

`OperatorPlan/v1` uses a canonical processing format:

- sample rate: exactly 44,100 frames/second;
- channels: exactly two, ordered left then right;
- sample domain: real-valued linear PCM, nominal range `[-1, 1]`;
- time coordinate: signed integer sample frames;
- timeline origin: frame `0` is the resolved musical handoff anchor;
- intervals: half-open `[start_frame, end_frame)` unless explicitly stated.

The plan refers only to canonical decoded PCM identities. Container timestamps,
MP3 encoder delay, source time bases, and backend time units MUST be resolved by
feature extraction/source ingestion before plan generation.

Phase 1 source identity profile is `pcm_s16le_stereo_44100_v1`: decode the full
source with the frozen ingestion environment, resample to 44.1 kHz stereo, emit
interleaved signed 16-bit little-endian PCM with no header, and hash those exact
bytes. Internal binary64 sample value is signed integer divided by 32,768. Cue
frames index this byte stream. A future source provider may verify the declared
identity through a prevalidated content manifest instead of decoding the full
track at plan-consumption time, but it may not substitute different content.

A time in seconds is converted to frames only by the generator using
round-to-nearest, ties away from zero. No renderer reconverts seconds. Named v1
constants are exact:

| Duration | Frames |
| --- | ---: |
| 5 ms | 221 |
| 10 ms | 441 |
| 20 ms | 882 |
| 30 ms | 1,323 |
| 80 ms | 3,528 |
| 200 ms | 8,820 |
| 3 s | 132,300 |
| 5 s | 220,500 |
| 6 s | 264,600 |
| 10 s | 441,000 |
| 12 s | 529,200 |
| 16 s | 705,600 |

Beat-derived durations are calculated from resolved beat-frame positions, never
from a floating BPM conversion when beat positions exist. Where only tempo is
available, `beat_frames = div_round_nearest_away(2_646_000_000,
tempo_millibpm)`.

### 3.2 Fixed-point quantities

All JSON numeric values are integers in the interoperable range
`[-9_007_199_254_740_991, 9_007_199_254_740_991]`.

| Suffix | Meaning |
| --- | --- |
| `_frame`, `_frames` | Canonical 44.1 kHz sample frame |
| `_ppm` | Parts per million; unity is `1_000_000` |
| `_mdb`, `_mdbfs`, `_mdbtp` | One-thousandth decibel |
| `_millihz` | One-thousandth hertz |
| `_q_milli` | Filter Q multiplied by 1,000 |
| `_millibpm` | BPM multiplied by 1,000 |

Linear gain from millidecibels is `10^(mdb / 20_000)`. Rate
`source_rate_ppm` is source frames consumed per output frame: above one million
plays faster; below one million plays slower.

The signed source offset for timeline delta `d` is:

```text
div_round_nearest_away(d * source_rate_ppm, 1_000_000)
```

The source cue frame and timeline frame zero MUST align within one canonical
frame after time-stretch latency compensation.

### 3.3 Text, IDs, hashes, and seeds

Plan strings are restricted to printable ASCII. Enumerations and object member
names use lower snake case. SHA-256 values are 64 lowercase hexadecimal
characters. Seeds are 16 lowercase hexadecimal characters representing an
unsigned 64-bit bit pattern; they are strings to avoid JSON numeric truncation.

`op_id` values are stable template slot names such as
`outgoing.primary_gain`; they are not counters. `geometry_id`, template IDs,
source IDs, and feature IDs MUST NOT contain private paths or human-identifying
metadata.

Phase 1 DSP is non-stochastic. The generator seed still participates in
provenance and MUST be `0000000000000000`; nonzero is invalid. Pair/split and
blind-presentation shuffling use separate manifest seeds outside `OperatorPlan`.
A future stochastic operator would need a named PRNG algorithm and explicit
nonzero seed in a new schema version.

## 4. `OperatorPlan/v1`

### 4.1 Logical schema

The canonical JSON object has exactly these top-level members in the logical
model. The following is a field-shape illustration, not a conformance fixture;
hash values, operations, and safety values are abbreviated. Canonical byte
ordering is specified later.

```text
{
  "schema_version": "transition-operator-plan/1",
  "plan_id": "op1-<sha256>",
  "template": {
    "id": "safe_crossfade",
    "version": 1,
    "recipe_id": "five_second_linear"
  },
  "format": {
    "sample_rate_hz": 44100,
    "channels": 2,
    "channel_order": "stereo_lr"
  },
  "sources": {
    "outgoing": {
      "track_id": "private-track-...",
      "pcm_profile": "pcm_s16le_stereo_44100_v1",
      "pcm_sha256": "...",
      "pcm_frame_count": 12345678,
      "cue_id": "...",
      "cue_source_frame": 10000000
    },
    "incoming": {
      "track_id": "private-track-...",
      "pcm_profile": "pcm_s16le_stereo_44100_v1",
      "pcm_sha256": "...",
      "pcm_frame_count": 12345678,
      "cue_id": "...",
      "cue_source_frame": 500000
    }
  },
  "timeline": {
    "dry_start_frame": -220500,
    "dry_handoff_frame": 0,
    "effect_end_frame": 0
  },
  "operations": [<two or more complete typed operation objects>],
  "output_safety": {<all transition_output_safety_v1 members>},
  "feature_snapshot": {
    "schema_version": "transition-feature-snapshot/2",
    "sha256": "..."
  },
  "provenance": {
    "generator_id": "offline-transition-generator",
    "generator_version": "...",
    "generator_config_sha256": "...",
    "seed_hex_u64": "0000000000000000",
    "geometry_id": "..."
  }
}
```

`plan_id` is computed as described in section 4.7. A real plan uses no angle
brackets or ellipses. Optional values are omitted,
never represented as JSON `null`. Unknown members are invalid.

### 4.2 Source and timeline semantics

Both sources index canonical decoded 44.1 kHz stereo PCM. At timeline frame
zero, the outgoing and incoming `cue_source_frame` values represent the musical
events being aligned. A source at normal rate maps timeline frame `t` to
`cue_source_frame + t`.

`dry_start_frame` is negative and marks the first frame at which any transition
operation, tail capture, or primary handoff may differ from steady state. It
MUST be at least `-705_600` and at most `-1`. `dry_handoff_frame` is always zero in v1.
`effect_end_frame` is between zero and `264_600`; it is nonzero only for a
feed-forward tail.

The primary outgoing gain MUST be one for all frames before
`dry_start_frame`, then reach zero no later than frame zero and remain zero. The
primary incoming gain MUST be zero before `dry_start_frame`, reach one exactly
at frame zero, and remain one. Templates may transfer perceptual ownership
earlier, but frame zero remains the common resolved anchor.

The plan describes transition behavior, not a listening-preview layout. A
`RenderRequest` supplies pre/post context without altering plan identity.

### 4.3 Operation union

Every object in `operations` contains:

```text
op_id          stable ASCII template slot ID
kind           one allowed v1 operation kind
target         outgoing or incoming
```

Allowed v1 kinds are:

1. `time_map`;
2. `gain_envelope`;
3. `filter_envelope`;
4. `crossover_band_gain`;
5. `duck_envelope`;
6. `feedforward_delay_tail`;
7. `rhythmic_gate`.

There is no `mix`, plugin, expression, callback, generic effect, binary payload,
backend syntax, preset, reverb, noise, stem, or graph-edge kind. Fixed source
summation and output safety are structural stages, not operations.

Each plan MUST contain exactly one `outgoing.primary_gain` and one
`incoming.primary_gain`. It MAY contain one incoming `time_map`. Other allowed
operations and combinations are determined by the template signature in
section 7. A plan contains at most 12 operation objects.

Envelope/resource limits are: eight points per primary gain, six per duck,
eight cutoff or wet points per filter, eight gain points per crossover band,
256 points in one rhythmic gate, three delay taps, two crossover frequencies,
and 512 total envelope points per plan. Exceeding any limit is invalid even when
the canonical JSON remains below its byte limit.

### 4.4 Normative operation order

Execution order is independent of JSON enumeration accidents. Operations run by
these fixed stages:

1. incoming pitch-preserving `time_map`;
2. `filter_envelope` or `crossover_band_gain` per source;
3. `duck_envelope` or `rhythmic_gate` per source;
4. `feedforward_delay_tail` capture and tap generation;
5. primary `gain_envelope` on each dry source;
6. sum outgoing dry, outgoing tail, incoming dry, and incoming tail;
7. apply pair-shared output pre-gain;
8. apply the reference limiter;
9. measure and quantize output.

Within a stage, outgoing precedes incoming. The validator requires the JSON
array to be sorted by `(stage, target_order, op_id)` and rejects rather than
reorders a noncanonical plan. No two operations may share an `op_id`.

Tail taps are generated from the target signal after stages 1–3 but before its
primary gain. Primary gain applies only to the dry branch, so a dry cut does not
erase its declared tail. Tap gains are the complete wet gain; no implicit wet
mix exists.

### 4.5 Combination rules

In addition to template signatures:

- one source cannot have both `filter_envelope` and
  `crossover_band_gain`;
- one source cannot have both `duck_envelope` and `rhythmic_gate`;
- a plan has at most one tail operation and it targets outgoing in v1;
- only incoming may have `time_map`; outgoing rate is exactly unity;
- all gain-affecting parameters are non-boosting (`0..=1_000_000` ppm);
- no operation may extend outside `[dry_start_frame, effect_end_frame)`, except
  primary envelope edge values which are clamped outside their point range;
- a template validator checks the exact allowed signature and rejects extra
  operations even when the generic operator validator would accept them.

### 4.6 Provenance versus audio semantics

Two hashes serve different purposes:

- `audio_semantics_sha256` is calculated over the canonical projection of
  `schema_version`, `format`, `sources`, `timeline`, `operations`, and
  `output_safety`. It excludes `plan_id`, feature snapshot, template labels, and
  provenance. It is used for pre-render semantic deduplication.
- `plan_sha256` is calculated over the full canonical plan with `plan_id`
  omitted. It includes template, feature snapshot, and provenance. It identifies
  the auditable plan production event.

`plan_id` is `op1-` followed by `plan_sha256`. A candidate ID is
`cand1-` followed by:

```text
sha256("transition-candidate/1\0" || plan_sha256_bytes)
```

Here `\0` is one zero byte and `||` is byte concatenation.

No identity includes a list index, insertion order, process ID, timestamp,
random UUID, absolute path, or renderer result. A generator version/config/seed
change may intentionally change candidate identity; mere iteration-order changes
may not.

### 4.7 Canonical JSON and hashing

Canonicalization is performed only after strict schema validation:

- UTF-8 without BOM;
- no insignificant whitespace and no trailing newline;
- object keys sorted by ascending ASCII byte sequence;
- arrays preserve schema-defined semantic order;
- integers encoded in shortest base-10 form, with zero written `0`;
- booleans written `true` or `false`;
- strings escaped only as required by JSON; plan strings are printable ASCII;
- no floats, exponent notation, `null`, duplicate keys, or unknown keys;
- maximum canonical plan length: 65,536 bytes.

Hash concatenation uses raw 32-byte SHA-256 values where `*_bytes` is stated,
not their hexadecimal text. All other hashes are over the exact canonical byte
sequence.

Parsing an untrusted document and serializing it canonically MUST produce the
same bytes as the input before it is accepted as canonical. Noncanonical input
may be diagnosed but is not silently normalized for rendering.

### 4.8 Versioning and compatibility

`transition-operator-plan/1` has immutable semantics. Adding a member, operation,
interpolation, changed bound, default, or processing rule requires a new schema
version. A v1 validator rejects every other version, including a hypothetical
minor version.

A future implementation MAY support several exact versions through separate
validators/lowerers. Migration is an explicit pure transform that emits a new
plan, new hashes, migration ID/version, and source plan hash. Old bytes are
never reinterpreted under new semantics. There is no forward-compatible
“ignore unknown field” behavior.

Renderer capability documents list exact supported plan versions and operation
profiles. Schema validity does not imply renderer support.

## 5. Exact operation semantics

### 5.1 Envelope evaluation

Envelope-bearing operations store `points`, each with signed `frame` and integer
`value_ppm`, plus one `interpolation` value for every segment. Points are
strictly increasing. Before the first point, the first value is held; at and
after the last point, the last value is held.

For frame `n` in `[f0, f1)`, let `x = (n - f0) / (f1 - f0)` and
`v = v0 + (v1 - v0) * p(x)`, evaluated in binary64:

| Interpolation | `p(x)` |
| --- | --- |
| `hold` | `0` |
| `linear` | `x` |
| `smoothstep` | `3*x*x - 2*x*x*x` |
| `quarter_sine` | `sin(pi*x/2)` |
| `quarter_cosine` | `1 - cos(pi*x/2)` |

The exact endpoint value is used at `f1`. `hold` is invalid for a segment whose
endpoint values differ unless the operation explicitly permits a step; the only
v1 primary-gain exception is `beat_cut/v1` recipe `hard_0ms`. Binary64
mathematical differences between backends are covered by renderer conformance
tolerances, while reference-environment PCM is required to be bit-repeatable.

Matched equal-power handoff uses outgoing values `1_000_000 -> 0` with
`quarter_cosine` and incoming `0 -> 1_000_000` with `quarter_sine`; their ideal
squared gains sum to one.

### 5.2 `gain_envelope`

Fields are `op_id`, `kind`, `target`, `points`, and `interpolations`.
`interpolations.len == points.len - 1`. Values are `0..=1_000_000` ppm.

Primary envelope requirements are stated in section 4.2. A source sample is
multiplied by the evaluated linear gain. There is no implicit normalization.
Asymmetry is represented by different point positions and curves on outgoing
and incoming envelopes, never by a separate effect.

### 5.3 `time_map`

Fields are:

```text
op_id: incoming.time_map
kind: time_map
target: incoming
source_rate_ppm: 920_000..=1_080_000
profile: pitch_preserving_balanced_transients_v1
```

Rate is constant over the complete rendered incoming window. Pitch must remain
at the original nominal pitch. The processor MUST produce exactly the requested
output frame count, compensate its algorithmic latency/padding, and align the
declared cue to timeline frame zero within one frame. It must process one
continuous source window; independently stretching fragments is invalid.

The profile fixes intent and conformance requirements, not a proprietary
algorithm. Distinct backends need not produce bit-identical PCM, but MUST pass
profile golden vectors with exact output length, cue alignment within one frame,
steady-tone pitch error at most one cent, transient displacement at most 221
frames (5 ms), and integrated-level error at most 100 mdb against the declared
fixture result. Phase 1 reproducibility is scoped to the pinned reference
renderer environment.

### 5.4 `filter_envelope`

Fields are:

```text
filter_kind: lowpass_biquad_v1 | highpass_biquad_v1
cutoff_points: [{ frame, cutoff_millihz }]
cutoff_interpolations: [linear | smoothstep]
q_milli: 500..=1_000
wet_points: envelope in 0..=1_000_000 ppm
wet_interpolations: [linear | smoothstep]
control_interval_frames: 64
```

Cutoff is `20_000..=18_000_000` millihertz and never above 45% of sample rate.
The filter is an RBJ biquad. For cutoff `f`, sample rate `Fs`, and Q `Q`:

```text
w0 = 2*pi*f/Fs
alpha = sin(w0)/(2*Q)

low-pass:
b0 = (1-cos(w0))/2; b1 = 1-cos(w0); b2 = b0
high-pass:
b0 = (1+cos(w0))/2; b1 = -(1+cos(w0)); b2 = b0

a0 = 1+alpha; a1 = -2*cos(w0); a2 = 1-alpha
```

All coefficients are divided by `a0`. Each channel uses direct-form II
transposed state in binary64. Cutoff envelopes are sampled at every timeline
frame `64*k` for any signed integer `k`; coefficients are linearly interpolated
per frame between adjacent control values. This rule prevents backend-specific
command-step and negative-modulo semantics.

The filter runs during renderer warm-up even while wet gain is zero. Output is
`dry*(1-wet) + filtered*wet`. Q never creates an intentional resonance boost.

### 5.5 `crossover_band_gain`

Fields are:

```text
profile: linkwitz_riley_4_v1
crossover_millihz: [one or two strictly increasing values]
bands: [low, high] or [low, mid, high]
band_gain_envelopes: one envelope per band
wet_points and wet_interpolations
control_interval_frames: 64
```

Crossovers are fixed, not swept, and each lies between 80 and 5,000 Hz. A
fourth-order Linkwitz-Riley split is two cascaded v1 RBJ second-order low-pass
filters and two cascaded high-pass filters at Q `707` milli.

For two split points `f1 < f2`, topology is fixed:

```text
low  = LP4(input, f1)
rest = HP4(input, f1)
mid  = LP4(rest, f2)
high = HP4(rest, f2)
```

Each band receives its declared non-boosting envelope, bands are summed, and
the result is mixed with dry through the wet envelope. Filter state and wet
handling match section 5.4. Alternative crossover topology is a new profile.

For `bass_handoff`, outgoing and incoming low-band gains MUST NOT sum above
`1_000_000` ppm at any breakpoint or midpoint used by the validator. For
`spectral_handoff`, the template-specific ownership constraints in section 7
apply.

### 5.6 `duck_envelope`

Fields are `op_id`, `kind`, `target`, `reason` (`vocal_collision` or
`transient_collision`), and a linear/smoothstep gain envelope. Values are
`251_189..=1_000_000` ppm (-12 dB through unity). Attack and release spans are
each 882–8,820 frames (20–200 ms).

The operation is a resolved multiplier. No detector, side chain, threshold,
knee, or signal-dependent state exists in v1. It runs before tail capture and
primary gain. The affected region MUST be within the feature window that made
the template applicable.

### 5.7 `feedforward_delay_tail`

Fields are:

```text
capture_start_frame
capture_end_frame
taps: [{ delay_frames, gain_ppm }]
```

There are one to three taps. Delays are 882–88,200 frames (20 ms–2 s), strictly
increasing. Gains are positive, monotonically decreasing, individually at most
`500_000` ppm, and sum to at most `800_000` ppm. There is no feedback.

Let `x[n]` be the post-spectral, post-dynamics target signal. A tap contributes
`gain*x[n-delay]` only when the originating frame `n-delay` is inside
`[capture_start_frame, capture_end_frame)`. All wet contributions outside the
capture-derived ranges are zero. `effect_end_frame` equals the latest possible
tap output end and is at most 264,600 frames.

### 5.8 Beat/bar cut

A cut has no dedicated operation. `beat_cut/v1` emits primary gains whose
transition is centered on frame zero:

- outgoing unity through `-ramp_frames`, then linear to zero at frame zero;
- incoming zero through `-ramp_frames`, then linear to unity at frame zero.

`ramp_frames` is 0, 441, 882, or 1,323. Zero is permitted only when the feature
snapshot marks both boundary windows `hard_cut_safe=true`; that boolean means
the extractor’s versioned click-risk test passed. Otherwise the zero-ramp recipe
is inapplicable. A zero-ramp outgoing envelope has points `(-1, 1_000_000)` and
`(0, 0)` with `hold`; incoming has `(-1, 0)` and `(0, 1_000_000)` with `hold`.
Thus the last pre-handoff frame has the old value and frame zero has the new
value without duplicate point positions.

### 5.9 `rhythmic_gate`

Fields are `op_id`, `kind`, `target`, and an explicit linear gain envelope. The
envelope, not a beat-expression language, is the normative pattern. Values are
zero through unity; every nonzero change spans at least 221 frames; pattern
duration is at most two resolved bars; transitions occur no faster than eight
per second.

The operation multiplies the outgoing signal before tail capture and primary
gain. The final gate value is zero at frame zero. No renderer receives BPM,
meter, duty cycle, or a request to synthesize a pattern.

### 5.10 Energy ramp

There is no `energy_ramp` operation. `energy_ramp/v1` emits resolved asymmetric
primary gains and, only in its two filter-assisted recipes, one
`filter_envelope`. It never applies gain above unity.

If incoming is louder, its primary gain uses an attenuated plateau equal to the
smaller of the absolute measured delta and 6,000 mdb: it remains zero at
`dry_start_frame`, rises to the plateau by 25% of the dry interval, holds
through 75%, then reaches unity at frame zero. If incoming is quieter, outgoing
ownership ends earlier; incoming is not boosted. The measured direction and
resolved attenuation are recorded in candidate features and recipe parameters.

### 5.11 Output pre-gain and limiter

`output_safety` has exactly:

```text
profile: transition_output_safety_v1
pair_output_gain_mdb: -24_000..=0
sample_peak_ceiling_mdbfs: -1_200
true_peak_target_mdbtp: -1_000
limiter_profile: lookahead_peak_limiter_v1
lookahead_frames: 221
release_frames: 4_410
maximum_gain_reduction_mdb: 3_000
maximum_active_fraction_ppm: 50_000
activity_threshold_mdb: 100
true_peak_measurement_profile: bs1770_4x_v1
```

`pair_output_gain_mdb` is computed once for the complete pair candidate set from
whole-source true peaks and conservative operator-summation bounds. Before final
plan canonicalization, let `P` be the greater whole-canonical-PCM source true
peak in mdbtp and let `M` be the greatest operation margin present in any draft
candidate: 0 mdb for gain/duck/gate, 1,000 mdb for time stretch, and 3,000 mdb
for any filter, crossover, or delay tail. The shared value is:

```text
min(0, -1_200 - P - 6_021 - M)
```

The 6,021 mdb term bounds coherent addition of two equal-amplitude sources; the
operation margin is deliberately conservative and not additive. This formula is
pure, renderer-independent, and removes a render/plan-hash cycle. The value is
byte-identical in every final plan for that pair, including fallback and all
experimental conditions. A formula result below -24,000 mdb makes the pair
ineligible. Missing or nonconformant whole-source true-peak measurements also
make the pair ineligible; the generator never substitutes zero or a window-local
peak. If post-render QC rejects one candidate, the shared gain remains
unchanged for every survivor; candidates are never selectively quieted or
silently regenerated.

After summation, pair gain is applied. For limiter input frame `n`, let `p[n]`
be the maximum absolute sample across both channels and the inclusive lookahead
window `[n, n + lookahead_frames]`, with zeros after the render end. Required
gain is `min(1, ceiling_linear / p[n])`, treating zero peak as unity. Gain
reduction attacks immediately. During release, gain may increase by at most the
constant binary64 multiplier `100^(1/release_frames)`, which recovers 40 dB over
`release_frames`. The smaller of required and released gain is applied equally
to both channels. Gain before the first rendered frame is unity, while the first
frame's lookahead still permits immediate attack.

Limiter activity is the fraction of frames in
`[dry_start_frame, effect_end_frame)` whose applied gain reduction exceeds
`activity_threshold_mdb`; if `effect_end_frame` is zero, the window ends at
frame zero. Maximum reduction and true peak are measured over the entire render
request. The limiter is a safety device and MUST run under the same profile for
all Pilot V2 conditions, even when it never activates.

Applied reduction in mdb is
`round_nearest_away(-20_000 * log10(applied_gain))`; zero gain is invalid rather
than represented as an unbounded reduction. Active fraction is
`round_nearest_away(active_frames * 1_000_000 / analysis_window_frames)`.

Post-limiter true peak is measured with the declared profile. A candidate is
rejected if true peak exceeds -1,000 mdbtp, maximum reduction exceeds 3,000 mdb,
or active fraction exceeds 50,000 ppm. No second candidate-local normalization
or limiting pass is allowed.

`bs1770_4x_v1` means four-times oversampling using the coefficients and boundary
handling in ITU-R BS.1770-4 Annex 2, taking the maximum absolute oversampled
value across both channels. It is a measurement profile, not a backend-selected
meter. Conformance fixtures MUST include impulses at every input phase and
near-Nyquist tones.

## 6. Feature and geometry input contracts

### 6.1 Feature snapshot boundary

Templates consume `transition-feature-snapshot/2`; they do not call feature
extractors. Its frozen values include:

- source identity/duration and analysis identity;
- cue IDs, source frames, cue kinds, and confidence;
- beat/downbeat frames, meter where known, tempo, and confidence;
- per-window vocal activity and localized/sustained collision values;
- transient activity/collision and boundary click-safety result;
- bass occupancy/collision;
- low/mid/high spectral occupancy, stability, and overlap;
- short-term energy/loudness, energy variability, and pair delta;
- whole-canonical-PCM source sample peak and true peak, plus any window-local
  peaks needed as candidate features.

Probabilities/confidences/occupancies are `0..=1_000_000` ppm. Energy and
loudness deltas are signed millidecibels. Every derived value carries a valid
window ID whose source-frame range is in the snapshot. Missing values are
absent, not zero.

Feature algorithms and extractor versions live in the feature schema, not this
IR. Applicability formulas below are exact over a valid snapshot. The outstanding
Pilot V1 feature audit may change how values are produced, but may not silently
change these units or template rules; that requires a new feature or template
version.

Generic structural plan validation needs only the canonical plan. Contextual
template validation additionally receives the exact feature snapshot named by
the plan and fails on hash mismatch or missing data.

### 6.2 Geometry proposal

`transition-geometry-proposal/1` contains:

```text
geometry_id
outgoing/incoming cue_id and cue_source_frame
duration_mode: seconds | bars | cut
requested_dry_frames
resolved_bar_count: 0 | 1 | 2 | 4
incoming_source_rate_ppm
cue_confidence_ppm
beat_confidence_ppm
downbeat_confidence_ppm
alignment_error_ppm_of_beat
geometry_quality_ppm
feature_window_ids
```

`geometry_id` is `geom1-` plus SHA-256 of canonical JSON containing the two
source PCM hashes, feature-snapshot hash, cue IDs/frames, duration fields,
incoming rate, and referenced window IDs. It is not an input-list index. Cue and
window IDs follow the same rule: they hash their source PCM hash, analysis hash,
kind, and exact frame interval. Reordering extracted cues or geometry proposals
therefore cannot rename a semantic input.

Geometry is legal only if all source ranges required by its transition fit the
canonical PCM, incoming rate is 920,000–1,080,000 ppm, and every referenced
feature window exists. Beat/bar proposals require beat confidence at least
750,000, downbeat confidence at least 700,000, and alignment error at most
31,250 ppm of a beat. Cue confidence must be at least 700,000 for every rich
template.

The cue/geometry proposer MUST also provide one `fallback_geometry` supporting
exactly 220,500 dry frames. This is a precondition of a valid pair-generation
request. It may use a reliable cue pair or a deterministic bounds-safe outgoing
end/incoming beginning. If either source cannot supply that window, the pair is
`PAIR_INELIGIBLE`; this is not recursive fallback failure.

### 6.3 Geometry shortlist

Proposals are schema-validated and deduplicated by all render-affecting fields.
They are bucketed:

1. `cut`;
2. `short` (up to two bars or five seconds);
3. `medium` (over two and up to four bars or ten seconds);
4. `long` (over medium and up to 16 seconds).

Within a bucket, sort by descending `geometry_quality_ppm`, then ascending
`geometry_id`. Select in repeating bucket order `cut, short, medium, long`,
skipping empty buckets, until 12 proposals are retained or input is exhausted.
Input order cannot affect the shortlist.

## 7. Initial template specification

### 7.1 Shared predicate vocabulary

The following v1 thresholds are generator configuration constants and therefore
part of `generator_config_sha256`:

| Predicate | Exact rule |
| --- | --- |
| reliable cue | cue confidence >= 700,000 ppm |
| reliable rhythmic geometry | beat >= 750,000 ppm; downbeat >= 700,000 ppm; alignment error <= 31,250 ppm of beat |
| vocal free | activity <= 200,000 ppm |
| localized collision | collision 300,000–700,000 ppm and affected span <= two beats |
| sustained collision | collision > 700,000 ppm or affected span > two beats |
| bass present | occupancy >= 250,000 ppm |
| bass collision | collision >= 350,000 ppm |
| strong transient grid | transient activity >= 600,000 ppm |
| stable spectrum | stability >= 700,000 ppm |
| material spectral overlap | overlap >= 450,000 ppm |
| meaningful energy mismatch | absolute delta >= 3,000 mdb |
| stable energy | window variability <= 2,000 mdb |
| sparse exit | vocal <= 200,000 ppm and transient density <= 350,000 ppm |

If a required value is missing, the predicate is false. Template-specific
`need_score_ppm` is specified below and chooses the six rich families retained
when more are applicable. Optional inputs to a need-score formula are treated as
zero only for family priority; this never makes a missing required predicate
true.

### 7.2 Deterministic recipe-to-geometry binding

Every template owns an ordered recipe list whose length equals its quota. A
recipe is attempted at most once. For each recipe, filter to geometries whose
duration mode and exact seconds/bar/cut duration match that recipe and whose
referenced feature windows cover its complete dry/effect interval. Sort the
compatible geometries by descending geometry quality, then ascending geometry
ID. Recipe index `r` uses geometry at index `r mod compatible_count`. With no
compatible geometry the recipe records `NO_COMPATIBLE_GEOMETRY` and is skipped.
If later resolution or validation fails, the rejection is recorded and the
generator proceeds to the next recipe; it does not try the same recipe against
another geometry. This is bounded enumeration, not a geometry × parameter grid.

### 7.3 `safe_crossfade/v1`

| Item | Normative value |
| --- | --- |
| Purpose | Guaranteed ordinary control/fallback |
| Predicate | Valid pair request and valid five-second fallback geometry |
| Required features | Source identities/durations only |
| Quota | 1 |
| Recipe | `five_second_linear`: start `-220_500`, handoff `0`; outgoing linear 1→0, incoming linear 0→1 |
| Operations | Two primary `gain_envelope`; optional incoming `time_map` is forbidden |
| Control | It is the control |
| Failure | Pair-level `FALLBACK_RENDER_FAILED`; no recursive fallback |
| Future class | A musical core; full current-runtime lowering subject to section 12 |

It bypasses rich-template family selection and is inserted first logically, then
included in final ID sorting.

### 7.4 `shaped_handoff/v1`

Purpose is to prevent energy holes and outgoing drag without adding spectral or
tail effects. It requires reliable cues and rejects sustained vocal collision.
Need score is `max(250_000, spectral_overlap_ppm, bass_collision_ppm)`.

Quota is eight; recipes are:

| Recipe ID | Duration | Outgoing / incoming |
| --- | --- | --- |
| `eq_3s` | 132,300 frames | quarter-cosine / quarter-sine |
| `eq_5s` | 220,500 | quarter-cosine / quarter-sine |
| `eq_2bar` | resolved two bars | quarter-cosine / quarter-sine |
| `eq_4bar` | resolved four bars, max 441,000 | quarter-cosine / quarter-sine |
| `asym_early_3s` | 132,300 | outgoing reaches zero at -33,075; incoming smoothstep reaches one at 0 |
| `asym_early_5s` | 220,500 | outgoing reaches zero at -55,125; incoming smoothstep reaches one at 0 |
| `asym_early_2bar` | two bars | outgoing reaches zero at 75% of interval; incoming reaches one at 0 |
| `asym_early_4bar` | four bars, max 441,000 | same 75% rule |

Only two primary gains and optional incoming `time_map` are allowed. Bar recipes
require reliable rhythmic geometry. Second-based recipes require no beat grid.
Control is the five-second safe crossfade on its fallback geometry, plus a
geometry-matched linear render where available. Invalid recipes fall back at the
candidate-set level to safe crossfade. Class A, or B when time stretch is used.

### 7.5 `beat_cut/v1`

Purpose is immediate rhythmic ownership without full-band collision. It requires
reliable rhythmic geometry and rejects sustained outgoing vocal activity.
Need score is `max(vocal_collision_ppm, transient_collision_ppm,
bass_collision_ppm)`.

Quota four recipes are `hard_0ms`, `soft_10ms`, `soft_20ms`, and `soft_30ms` with
ramp frames 0, 441, 882, and 1,323. Hard is applicable only when
`hard_cut_safe=true` for both boundary windows. Operations are two primary gain
envelopes and optional incoming `time_map`; no overlap effect is allowed.

Control is the same-geometry `soft_20ms` candidate for hard-cut comparison and
the same-cue shaped handoff for template comparison. Failure falls back to an
applicable shaped handoff, otherwise safe crossfade. Class A, or B with stretch.

### 7.6 `bass_handoff/v1`

Purpose is exclusive bass/kick ownership during an otherwise useful overlap.
It requires reliable rhythmic geometry, bass present in at least one source,
and bass collision. It rejects sustained vocal collision. Need score is
`bass_collision_ppm`.

Quota eight recipes are fixed, not a product:

| Recipe | Bars | Crossover | Low-band transfer |
| --- | ---: | ---: | --- |
| `b140_2_early` | 2 | 140 Hz | complementary linear transfer from 25% to 50% of interval |
| `b180_2_early` | 2 | 180 Hz | same |
| `b220_2_early` | 2 | 220 Hz | same |
| `b180_2_center` | 2 | 180 Hz | complementary linear transfer from 40% to 60% |
| `b140_4_early` | 4 | 140 Hz | complementary linear transfer from 25% to 50% |
| `b180_4_early` | 4 | 180 Hz | same |
| `b220_4_early` | 4 | 220 Hz | same |
| `b180_4_center` | 4 | 180 Hz | complementary linear transfer from 40% to 60% |

The plan emits two primary equal-power gains and one two-band
`crossover_band_gain` per source, with wet gain smooth-ramped from zero to unity
over 882 frames at dry start and back to zero over 882 frames ending at frame
zero. High-band gain stays unity within the crossover operation; primary gains
control the complete source. Optional incoming time map is allowed.

Control is a geometry-matched equal-power shaped handoff. Failure uses that
candidate if present, otherwise safe crossfade. Class B.

### 7.7 `spectral_handoff/v1`

Purpose is staged spectral ownership instead of excessive full-band overlap. It
requires reliable cues, stable spectrum, and material spectral overlap, and
rejects sustained vocal collision. Need score is `spectral_overlap_ppm`.

Quota eight recipes are:

| Recipe | Duration | Spectral operation |
| --- | --- | --- |
| `lp_out_3s_500` | 3 s | outgoing LP wet 0→1; cutoff 18 kHz→500 Hz |
| `lp_out_5s_250` | 5 s | outgoing LP wet 0→1; cutoff 18 kHz→250 Hz |
| `lp_out_2bar_500` | 2 bars | same endpoint 500 Hz |
| `hp_in_3s_2000` | 3 s | incoming HP wet 1→0; cutoff 2 kHz→20 Hz |
| `hp_in_5s_1200` | 5 s | incoming HP wet 1→0; cutoff 1.2 kHz→20 Hz |
| `hp_in_2bar_1600` | 2 bars | incoming HP 1.6 kHz→20 Hz |
| `bands_2bar_high_then_low` | 2 bars | 180 Hz / 2.5 kHz three-band stagger, high then mid then low incoming ownership |
| `bands_4bar_low_then_high` | 4 bars | same splits, low then mid then high incoming ownership |

All filters use Q 707 milli, 64-frame control, and smoothstep wet envelopes.
Filter recipes emit one filter plus primary equal-power gains. Band recipes emit
one three-band crossover per source plus primary gains. Optional incoming time
map is allowed only for bar recipes.

For `high_then_low`, corresponding outgoing-to-incoming band gain transfers are
linear and occupy normalized dry-interval thirds: high `[0, 1/3]`, mid
`[1/3, 2/3]`, low `[2/3, 1]`. `low_then_high` reverses low and high while mid
remains the middle third. Third boundaries use round-nearest ties away. Before a
band's window outgoing/incoming gains are one/zero; after it they are zero/one.
Band-operation wet gain ramps from zero to unity over the first 882 frames and
from unity to zero over the final 882 frames.

For band recipes, corresponding outgoing and incoming band gains may sum to at
most unity at every envelope breakpoint and segment midpoint. Control is a
geometry-matched equal-power shaped handoff. Failure falls back to shaped
handoff, then safe crossfade. Class B.

### 7.8 `ducked_overlap/v1`

Purpose is localized attenuation of one collision inside a useful short
overlap. It requires reliable cues and exactly one localized vocal or transient
collision; sustained collision is incompatible. The target is the source with
the greater activity in the collision window, with outgoing winning an exact
tie. Need score is the localized collision value.

Quota eight fixed recipes are:

| Recipe | Depth | Attack | Release | Duration mode |
| --- | ---: | ---: | ---: | --- |
| `d6_fast_3s` | -6 dB | 20 ms | 80 ms | 3 s |
| `d9_fast_3s` | -9 dB | 20 ms | 80 ms | 3 s |
| `d6_smooth_5s` | -6 dB | 80 ms | 200 ms | 5 s |
| `d9_smooth_5s` | -9 dB | 80 ms | 200 ms | 5 s |
| `d6_fast_2bar` | -6 dB | 20 ms | 80 ms | 2 bars |
| `d9_fast_2bar` | -9 dB | 20 ms | 80 ms | 2 bars |
| `d6_smooth_4bar` | -6 dB | 80 ms | 200 ms | 4 bars |
| `d9_smooth_4bar` | -9 dB | 80 ms | 200 ms | 4 bars |

The hold region is the exact collision feature window, clipped to the dry
interval; attack/release surround it. Plans emit primary equal-power gains, one
resolved duck envelope, and optional incoming time map for bar recipes. Control
is the same plan with the duck operation removed. Failure uses shaped handoff or
safe crossfade. Class A, or B with stretch.

### 7.9 `echo_tail_handoff/v1`

Purpose is a prompt dry exit with bounded beat-timed continuity. It requires
reliable rhythmic geometry and sparse exit, and rejects vocal activity above
200,000 ppm or transient density above 350,000 ppm. Need score is
`1_000_000 - max(vocal_activity_ppm, transient_density_ppm)`.

Quota four recipes are:

| Recipe | Capture | Taps `(delay, gain)` |
| --- | --- | --- |
| `quarter_3tap` | final half beat before zero | 1/4 beat at 400,000 ppm; 1/2 at 240,000; 3/4 at 140,000 |
| `half_3tap` | final beat | 1/2 beat at 400,000 ppm; 1 beat at 220,000; 3/2 at 120,000 |
| `quarter_2tap` | final half beat | 1/4 beat at 360,000 ppm; 1/2 at 180,000 |
| `half_2tap` | final beat | 1/2 beat at 360,000 ppm; 1 beat at 180,000 |

Beat fractions are resolved to integer frames with round-nearest ties away.
The dry handoff is `beat_cut` with a 20 ms ramp. Plans emit two primary gains,
one outgoing feed-forward tail, and optional incoming time map. Control is the
same dry cut with no tail. Failure uses beat cut, shaped handoff, or safe
crossfade in that order. Class B.

For this template, `dry_start_frame` is the earlier of the tail capture start
and `-882`, even though primary dry gain remains unity until `-882`. This keeps
every operation and capture window inside the declared transition domain.

### 7.10 `energy_ramp/v1`

Purpose is a non-boosting bridge across stable energy mismatch. It requires
reliable cues, stable energy in both windows, and absolute delta at least 3,000
mdb. It rejects sustained vocal collision. Need score is
`min(1_000_000, abs(delta_mdb) * 100)`.

Quota six recipes are:

| Recipe | Duration | Shape |
| --- | --- | --- |
| `gain_3s` | 3 s | smoothstep asymmetric gain |
| `gain_5s` | 5 s | smoothstep asymmetric gain |
| `gain_2bar` | 2 bars | smoothstep asymmetric gain |
| `gain_4bar` | 4 bars | smoothstep asymmetric gain |
| `filter_2bar` | 2 bars | gain plus low-pass outgoing when incoming is louder, or high-pass incoming admission when incoming is quieter |
| `filter_4bar` | 4 bars | same direction rule |

Attenuation is capped at 6,000 mdb. When incoming is louder, it starts at zero,
reaches the resolved attenuated plateau at 25% of the interval, holds through
75%, and reaches unity at frame zero. When incoming is quieter, outgoing reaches
zero at 75% of the dry interval; incoming follows its normal smoothstep 0→1 and
is never boosted. Filter-assisted recipes use the exact parameters below.

The attenuated plateau in ppm is
`round_nearest_away(1_000_000 * 10^(-attenuation_mdb/20_000))`. All 25%, 50%,
75%, third, and similar normalized frame positions in template recipes use
integer multiplication/division with round-nearest ties away.

For filter-assisted recipes, incoming-louder uses an outgoing low-pass from 18
kHz to 500 Hz with
wet gain zero-to-one; incoming-quieter uses an incoming high-pass from 1.6 kHz
to 20 Hz with wet gain one-to-zero. Both span the complete dry interval with Q
707, 64-frame control, and smoothstep wet/cutoff interpolation.

Control is a geometry-matched shaped handoff. Failure uses shaped handoff or
safe crossfade. Gain recipes are class A; filter recipes are class B.

### 7.11 `rhythmic_handoff/v1`

Purpose is a short deliberate rhythmic exit. It requires reliable rhythmic
geometry, strong outgoing transient grid, and vocal-free outgoing window. Need
score is outgoing transient activity.

Quota four recipes are:

| Recipe | Pattern |
| --- | --- |
| `quarter_1bar_even` | one bar, quarter-beat 50% duty gates |
| `eighth_1bar_even` | one bar, eighth-beat 50% duty gates |
| `quarter_2bar_decay` | two bars, quarter-beat gates with peak gains 1,000,000→700,000 ppm |
| `eighth_1bar_decay` | one bar, eighth-beat gates with peak gain 1,000,000→600,000 ppm |

Each pattern is resolved into an explicit linear envelope with 221-frame edges
and final zero at frame zero. Plans emit primary smoothstep gains, one outgoing
rhythmic gate, and optional incoming time map. Control is a 20 ms beat cut at the
same cue. Failure uses beat cut, shaped handoff, or safe crossfade. Class A, or
B with stretch.

Pattern cells use the resolved beat grid; no cell duration is inferred from
average BPM when beat frames exist. In each cell, gain rises from zero to the
cell peak over 221 frames, holds through the first half minus the fall ramp,
falls to zero over 221 frames ending at the cell midpoint, then remains zero to
the next cell. A recipe is inapplicable if a cell cannot contain both ramps.
Decay-recipe peak gains decrease linearly by cell index from the stated first to
last value, with integer ppm values rounded ties away.

## 8. Candidate generation algorithm

### 8.1 Family selection

The generator validates the request and independently creates a
`safe_crossfade/v1` draft. If its source/timing/template validation fails, it
returns `PAIR_INELIGIBLE` and does not invoke rich templates. The fallback is
finalized with the same safety-resolution step as the rich drafts.

It then evaluates all eight rich-template predicates. Applicable families are
sorted by descending `need_score_ppm`, then this immutable tie order:

```text
shaped_handoff
beat_cut
bass_handoff
spectral_handoff
ducked_overlap
echo_tail_handoff
energy_ramp
rhythmic_handoff
```

At most six rich families are retained. Applicability facts and pruned-family
reasons are recorded. `shaped_handoff` is not forcibly retained; the guaranteed
fallback is sufficient when six more situation-specific families have greater
need.

### 8.2 Enumeration, validation, and deduplication

For each retained family in the sorted order:

1. iterate its recipes using section 7.2;
2. resolve every beat, frame, gain, cutoff, collision window, and source bound;
3. build a draft containing all audio semantics except final `output_safety`;
4. run draft operation-bound, template-signature, and applicability validation;
5. append the draft until that family quota is reached.

After the retained draft set is known, calculate one pair output gain with the
section 5.11 formula, inject the identical complete `output_safety` object into
every draft, and then, for each candidate:

1. run complete generic and contextual `OperatorPlan` validation;
2. canonicalize and calculate both plan hashes;
3. discard an `audio_semantics_sha256` already accepted, recording the earlier
   candidate ID;
4. retain the final candidate.

A draft is an in-memory generator value, not a schema. It cannot be serialized
as `OperatorPlan/v1` or reach a renderer, feature builder, scorer, or critic.

Exceptions/errors are converted to one bounded rejection record and generation
continues with the next recipe. A rejection record has template, recipe,
geometry, validator/error code, and safe text; it has no stack dump or audio.

After generation, accepted candidates are sorted lexicographically by
`candidate_id`. Renderer input order and critic input order use this sort. The
fallback is located by template identity, not list position.

The candidate-set hash is SHA-256 of canonical JSON containing the exact
generator identity/config/seed, pair source hashes, and the final
lexicographically sorted list of `{candidate_id, plan_sha256}` objects.

### 8.3 Caps

The fixed v1 quotas sum to 51, but retaining at most six rich families makes the
normal mathematical maximum 43 including fallback. The generator still
implements defense-in-depth caps:

- maximum recipe attempts: 64 total, including fallback;
- soft accepted-candidate cap: 48;
- hard accepted-candidate cap: 64.

If accepted candidates exceed 48 under a future configuration of the same
generator, keep fallback and select rich candidates in deterministic rounds:
one lowest-candidate-ID candidate per family per round, families ordered by
family-selection order, until 47 rich candidates remain. Record all pruned IDs.

Attempting a 65th recipe or accepting a 65th candidate is
`GENERATOR_HARD_CAP_EXCEEDED`. The generator discards every rich result and
finalizes and returns only fallback using the fallback-only headroom margin,
plus the diagnostic. It never truncates an uncontrolled oversized set and never
recurses.

### 8.4 No-rich behavior

If no rich family applies, or every rich recipe fails, the valid candidate set
contains exactly `safe_crossfade/v1`. This is a normal result, not an error.

## 9. Candidate record and ranking interface

`transition-candidate-record/2` stores:

```text
schema/version and candidate_id
operator_plan and both plan hashes
input file/container hashes in a private manifest reference
feature snapshot schema/hash and candidate feature schema/vector
template/recipe and plain-English explanation
generator ID/version/config/seed and geometry ID
applicability, constraint, validator, and rejection audit
derived required-capability set and capability-derivation version
renderer ID/version/environment/capability hash and render-program hash
artifact container hash and canonical decoded-PCM hash
duration, sample/true peak, loudness, limiter reduction/activity, QC status
all scorer IDs/versions/model artifact hashes/raw scores/calibration
current-runtime compatibility classification
```

Backend program text is stored only in renderer-private provenance, never in
the `OperatorPlan`. Private paths remain in a gitignored/private source manifest;
blind/public records use opaque IDs and hashes.

Observational timestamps and wall-clock durations MAY appear in a noncanonical
run log, but never participate in plan, candidate-set, render-request, or audio
identity. Renderer CPU/memory measurements are labeled observations rather than
semantic fields.

Candidate feature construction is a pure function of the frozen feature
snapshot, resolved plan, and allowed pre-rating render measurements. It MUST NOT
read split-external ratings, condition labels, display positions, filenames, or
artist/title strings.

`transition-candidate-features/2` contains fixed-length groups with an explicit
presence bitmap:

- source-pair: tempo/rate relation, harmonic compatibility, energy delta,
  vocal/bass/transient/spectral compatibility;
- geometry: cue/beat/downbeat confidence, alignment error, dry duration, bar
  count, and source-bound margins;
- template/plan: one-hot template and recipe IDs, operation-presence bits, and
  normalized resolved gains, cutoffs, duck, tap, gate, and stretch values;
- predicted realization: pre-render collision/headroom estimates;
- technical render: peak/loudness, limiter reduction/activity, duration, and
  capability class.

The feature schema specifies every index, scale, clipping bound, and presence
bit. Missing optional values store integer zero with presence false; zero is not
used as an implicit missing marker. Categorical vocabularies are closed and
versioned. Corpus normalization parameters, if any, are separate hashed
artifacts fitted on development data only.

The critic API accepts an ordered list of valid candidate IDs and candidate
feature vectors, and returns one finite score per ID. It cannot return a new ID
or parameter. Invalid/missing scores disqualify that scorer and select the
deterministic scorer result; if that is unavailable, fallback is selected.
Scores are serialized as signed integer millionths after round-nearest ties away;
non-finite or out-of-range model output is invalid. Score ties resolve by
candidate ID.

## 10. Offline reference renderer contract

### 10.1 Inputs and source lookup

The renderer accepts one validated canonical plan plus a canonical
`transition-render-request/1` containing:

- plan hash and candidate ID;
- output start/end timeline frames;
- source-locator manifest hash;
- renderer capability-profile hash;
- artifact format (`flac_pcm24_stereo_44100_v1`);
- processing warm-up frames, exactly 4,096 in v1.

The request follows the same canonical JSON rules as the plan and has its own
SHA-256. `render_identity` is `render1-` plus SHA-256 of the ASCII domain
separator `transition-render/1`, one zero byte, raw plan-hash bytes, raw request-
hash bytes, and raw renderer-environment-hash bytes in that order. Absolute
source paths are locator-manifest data and do not participate.

For Pilot V2 the planned preview request is 36 seconds: start `-705_600`
(16 seconds before handoff), end `882_000` (20 seconds after), exactly 1,587,600
output frames. This presentation choice is not part of plan identity.

The source locator resolves each opaque ID/hash to a local file outside Git.
The renderer decodes and resamples to canonical PCM, checks the complete PCM
hash before DSP, and rejects identity mismatch. Required audible source windows
must exist; they are never silently zero-padded or looped. Only filter warm-up
before absolute source frame zero may use zeros.

### 10.2 Processing and state

The renderer processes one continuous window from 4,096 frames before requested
output start through output end. Warm-up output is discarded. Time stretch is
performed before filter/dynamics state. Filter state starts at zero at warm-up
start and runs continuously even with zero wet mix. Delay buffers start zero;
only explicit capture frames create taps.

Operations execute in section 4.4 order. No FFmpeg auto-normalization, automatic
format negotiation after canonical decode, implicit dither, implicit fade,
implicit delay compensation, or shortest-input truncation is allowed.

Time-stretch output padding and latency are removed so the cue alignment and
requested output frame count hold. A backend that cannot demonstrate this
rejects the plan as unsupported.

### 10.3 Output length and quantization

Output contains exactly `end_frame - start_frame` stereo frames. The request
must include the complete `effect_end_frame`; otherwise it is invalid. Before
encoding, finite binary64 samples are converted to signed 24-bit PCM:

```text
q = clamp(round_nearest_ties_away(sample * 8_388_607),
          -8_388_608, 8_388_607)
```

No dither is used for the canonical identity stream. Frames are interleaved
left/right, each signed value stored as three little-endian two's-complement
bytes. SHA-256 of these bytes is `decoded_pcm_sha256`. The FLAC encoder receives
exactly this PCM with metadata stripped; its file hash is recorded separately.
Any sample that would require the clamp indicates a preceding QC failure; the
clamp defines conversion safety but does not make clipping acceptable.

### 10.4 Measurements and QC

Measurements occur at named points:

1. each canonical input window before DSP: sample peak and true peak;
2. summed bus before pair gain: sample peak;
3. after pair gain and before limiter: sample peak and predicted limiter demand;
4. after limiter and before PCM quantization: sample peak, true peak, loudness,
   maximum limiter reduction, and active fraction;
5. decoded encoded artifact: duration, format, PCM hash, sample and true peak.

Post-encode PCM hash MUST equal the pre-encode canonical PCM hash. A candidate is
rejected for invalid plan/capability, missing source, source hash mismatch,
decode error, non-finite sample, output-length mismatch, cue misalignment,
unexpected channel/rate/format, true peak above target, limiter reduction or
activity above bounds, encode/decode mismatch, or backend failure.

Rich-candidate render rejection removes only that candidate. If all rich renders
fail, the already-rendered fallback remains. If fallback rendering fails, the
pair becomes `FALLBACK_RENDER_FAILED`; no hard cut, silence, rerender with changed
settings, or recursive fallback is synthesized.

### 10.5 Determinism

The reference environment record freezes OS/architecture, renderer source
revision, FFmpeg/codec versions, filter/time-stretch backend versions, process
flags, locale, rounding mode, thread counts, capability profile, and encoder
configuration.

Within one reference environment, identical source PCM, feature snapshot,
geometry set, generator configuration/seed, plan, and render request MUST
produce identical candidate IDs, canonical plan bytes, rendered PCM bytes, and
measurements. Multithreaded or nondeterministic backend modes are forbidden.

Cross-renderer PCM equality is not promised for filters or time stretch. An
independent renderer claims `SUPPORTED` only after operation-profile conformance
tests show the same timing, envelope, frequency-response, level, tail, and safety
behavior within published tolerances. Its renderer identity remains part of
artifact provenance.

## 11. Renderer capability and future lowering

A `renderer-capabilities/1` document declares exact schema versions, formats,
operation/profile support, numeric limits, state/latency limits, and output
safety profiles. Capability is derived from the plan, never trusted from a
plan-supplied list.

`capability-derivation/1` emits a lexicographically sorted set containing the
format/source profile, every operation kind/profile actually present, maximum
points/bands/taps/rate required, time-stretch ratio if present, output-safety
profile, lookahead, true-peak meter, and maximum state span. This derived set is
stored in the candidate record and hashed; it is not an executable plan field.

Lowering yields:

- `SUPPORTED`: every semantic operation/profile is implemented within declared
  conformance;
- `SUPPORTED_WITH_SIMPLIFICATION`: a named pure transformation emits a new
  validated plan and hash, with original hash/capability/reason recorded;
- `UNSUPPORTED`: no lowering.

Simplification never occurs during rendering. A transformed plan is a distinct
candidate that must be reference-rendered, measured, and explicitly selected.
Dropping filters/tails/duck/gates, changing curves, or substituting algorithms
without a new plan is forbidden.

Capability expectations are:

| Operation/template | Later class |
| --- | --- |
| Linear/smoothstep/equal-power/asymmetric gain | A |
| Beat cut, resolved duck, rhythmic gate, gain-only energy ramp | A |
| Biquad sweeps, LR4 crossover, spectral handoff | B |
| Feed-forward delay tail | B |
| Constant incoming pitch-preserving time map | B |
| Lookahead limiter/true-peak safety | B |
| Reverb, noise/riser, stems | C/deferred, absent from v1 |

No RPI implementation is part of Phase 1.

## 12. Relationship to today's `TransitionPlan`

Today's runtime plan can represent outgoing/incoming start positions, a positive
overlap duration, two bounded gain curves, and optional incoming speed
automation. It cannot represent filters, crossovers, independent duck/gate
stages, tail buses, the reference output limiter, or post-render QC.

Two compatibility labels are therefore distinct:

- `MUSICAL_CORE_LOWERABLE_TODAY`: operations are only primary gain envelopes
  representable exactly as the existing linear/quadratic/cubic Bezier segments,
  plus at most one constant incoming time map compatible with current speed
  automation. A frame position converts to nanoseconds by round-nearest ties
  away on `frames * 1_000_000_000 / 44_100` and MUST map back to the same frame.
- `FULL_PLAN_LOWERABLE_TODAY`: the musical-core condition holds, pair output gain
  is zero mdb, and output-safety validation/limiting is explicitly not required.

The Phase 1 validator always requires `transition_output_safety_v1`; consequently
no Phase 1 Pilot V2 candidate is automatically `FULL_PLAN_LOWERABLE_TODAY`.
Some gain-only plans may be labeled `MUSICAL_CORE_LOWERABLE_TODAY` as information
only. This prevents the false claim that today's runtime can preserve the full
offline safety/render contract.

Equal-power sine/cosine curves are not exactly representable by today's finite
Bezier plan and are not labeled core-lowerable. Linear primary handoffs and
exactly representable smoothstep cubics may be. Every filter, crossover, tail,
duck, rhythmic gate, or filter-assisted energy plan requires an extended future
renderer.

Nothing in Phase 1 calls this lowering path. The existing ordinary runtime
crossfade remains the runtime fallback. A richer plan may not silently become a
plain crossfade; an explicit separate fallback selection must record that the
original plan was unsupported.

## 13. Pilot V2 artifact interface and leakage controls

The operator subsystem must produce these hash-linked artifacts before Pilot V2
can be built:

1. private source inventory/annotation manifest;
2. immutable feature snapshot and extractor provenance;
3. pair/split manifest with seed and compatibility stratum;
4. generator configuration and complete geometry/applicability audit;
5. per-pair candidate records, rejected-candidate audit, and candidate-set hash;
6. reference renderer environment/capability record;
7. render/QC manifest and retained artifact hashes;
8. frozen condition-selection manifest for fallback, deterministic richer, and
   critic richer conditions;
9. sanitized blind manifest with opaque sample IDs and no condition/track leak;
10. ratings template and completed export bound to exact blind-manifest hashes.

Private local paths and identity annotations never enter Git or sanitized blind
artifacts. Copyrighted audio remains local.

The pool design is 60 unique tracks, six reserves, and 30 track-disjoint pairs:
ten compatible, ten moderate, ten deliberately awkward but legal. Split is 18
development, six validation, and six test pairs, with 6/2/2 pairs from each
stratum. A track appears once total. Artist/album groups are split-disjoint where
private annotation makes this possible.

Feature extraction may run independently on every track, but may not fit
corpus-level normalization using validation/test data. Any learned normalization,
critic, calibration, threshold tuning, or template/configuration choice uses
development data only, then freezes. Validation permits one predeclared decision;
test remains sealed until that decision. No validation/test rating may regenerate
candidates, change recipes, fit a scorer, or update feature scaling.

Each pair has three private condition selections: safe fallback, deterministic
richer selection, and critic selection from the exact same frozen set including
fallback. A selector is never forced to choose a different candidate. Before
building the ordinary blind set, selections with the same decoded-PCM hash are
collapsed to one presentation whose private mapping names every represented
condition. This prevents accidental same-audio conditions from recreating Pilot
V1's comparison noise.

Twelve predeclared hidden exact-audio repeats—six development, three validation,
three test, balanced by condition and display position—measure evaluator noise.
A deliberate repeat has a new blind presentation ID but the same decoded PCM
hash. Deliberate controls are the only same-audio duplicates presented within a
pair. Thus 90 is the maximum core presentation count rather than a forced count;
the 12 controls remain fixed.

Repeat assignment targets 2/2/2 fallback/deterministic/critic selections in
development and 1/1/1 in each held-out split. Within a split and condition,
eligible presentations are ordered by SHA-256 of the split's repeat seed,
condition ID, pair ID, and PCM hash. At most one repeat is added per pair. A
collapsed multi-condition presentation is eligible for each condition but is
assigned only to the first still-unfilled condition in order
fallback/deterministic/critic. If the exact quotas cannot be met, pilot build
fails instead of silently changing the control design.

The evaluator uses ordered tie groups directly, not inferred equal numbers. It
also records `good/meh/bad`, smoothness, intent, confidence, bounded issue flags,
and optional comment. Export validation requires a complete dense ordering with
explicit group membership. Unblinding occurs only after immutable completed
ratings are hash-frozen.

This section is an interface requirement, not authorization to generate Pilot
V2.

## 14. Validation and security behavior

Validation proceeds in this fixed order and returns stable error codes:

1. byte size, UTF-8/JSON syntax, duplicate key, and canonical-form checks;
2. exact schema/version/member/type checks;
3. ASCII ID, hash, integer range, and source identity checks;
4. timeline/source-bound and feature-window checks;
5. operation generic bounds and ordering;
6. operation-combination and template-signature checks;
7. template applicability and recipe equality checks;
8. derived capability and resource bounds;
9. hash and ID recomputation;
10. candidate-set quota/cap and shared-safety checks.

The validator never clamps, reorders, fills a default, drops an unknown field,
changes a recipe, or repairs a hash. Malformed plans never reach the renderer or
critic. Renderer command construction uses typed validated values and does not
concatenate untrusted backend fragments.

Missing feature data suppresses a template. Invalid rich candidates are isolated.
Invalid fallback makes the pair ineligible. Candidate/rejection arrays are
bounded, and diagnostic strings never include private paths or source metadata.

## 15. Testable invariants and acceptance criteria

The later implementation is acceptable only when automated tests demonstrate:

### Schema and canonicalization

- canonical serialization golden vectors match byte for byte across at least
  two independent language implementations or one implementation plus a
  separately generated fixture set;
- object input order does not change canonical bytes or hashes;
- array semantic order is validated and cannot be silently rearranged;
- floats, NaN/infinity spellings, duplicate/unknown keys, nulls, non-ASCII plan
  strings, oversized plans, invalid hashes, and unsupported versions fail;
- plan/candidate IDs recompute exactly and contain no enumeration index;
- audio-semantic dedup ignores provenance but never ignores an audio-affecting
  field.

### Operators and templates

- every numeric boundary has accept-at-boundary and reject-outside tests;
- envelope endpoint, half-open interval, hold, and interpolation golden values
  match the equations;
- equal-power paired ideal squared gain sums to one within declared tolerance;
- RBJ/filter/crossover impulse and frequency-response vectors match the v1
  topology and control interpolation;
- duck has no detector dependence; delay has no feedback and cannot outlive its
  bound; rhythmic patterns resolve to explicit click-safe envelopes;
- every template emits only its allowed signature and exactly its documented
  recipes at most once;
- missing predicates suppress the dependent recipe/family;
- no v1 plan contains reverb, noise, stems, preset IDs, or arbitrary backend
  syntax.

### Candidate generation

- identical inputs/config/seed produce the identical sorted candidate IDs,
  plans, rejection audit, and set hash;
- permuting input geometry order produces the same output;
- a candidate ID does not change because another candidate was inserted or
  rejected;
- family and candidate quotas hold; accepted count never exceeds 48 normally or
  64 under any input;
- a simulated 65th attempt returns fallback only with the hard-cap diagnostic;
- a valid five-second fallback is always present for a valid pair request;
- no-rich and all-rich-failed cases return exactly fallback;
- one template exception cannot break other templates or fallback;
- duplicate render semantics are removed deterministically.

### Renderer and safety

- invalid/unvalidated plans cannot invoke backend execution;
- operation stage order is independent of JSON/input iteration order;
- source hash, bounds, window, cue alignment, latency compensation, exact output
  frame count, and post-encode PCM identity are checked;
- the pinned environment renders repeated gain-only, filter/crossover,
  echo-tail, gate, limiter, and time-stretch combinations to identical canonical
  PCM hashes;
- non-finite output, peak violation, >3 dB limiter reduction, >5% activity, or
  measurement/hash mismatch rejects the candidate;
- all candidates for one pair contain identical output-safety fields and the
  same pair pre-gain;
- a candidate-local safety failure never triggers candidate-local gain repair;
- rich-render failure preserves fallback; fallback-render failure terminates
  with no recursion or synthesized hard cut.

### Boundaries and pilot artifacts

- critic APIs cannot submit operations/parameters or select an unknown/invalid
  ID;
- no Phase 1 module imports or calls playback ownership/state-machine code;
- unsupported rich plans cannot lower silently to current `TransitionPlan`;
- private paths/names cannot enter public/blind manifests or diagnostics;
- every selected/rendered/rated sample is hash-bound through source, feature,
  plan, renderer, PCM, and blind-manifest identities;
- pair and split validation proves one track per pair, one pair per track, exact
  18/6/6 split, exact 10/10/10 strata, and no trainable transform fitted from
  validation/test ratings;
- repeat controls share PCM hashes while retaining distinct blinded presentation
  IDs; tie groups round-trip without converting ties to arbitrary ranks.

## 16. Evidence gaps and non-blocking audit items

Two known evidence gaps remain and are not hidden by defaults:

1. The exact S-01 Pilot V1 offline feature schema is not in the local Git object
   store. Before feature-extractor implementation, audit it and map reusable
   fields explicitly into `transition-feature-snapshot/2`. This may change the
   extractor work, not `OperatorPlan/v1` semantics.
2. The private files lack embedded identity tags. Complete a private
   artist/album/style annotation before Pilot V2 pair and leakage manifests are
   frozen. This does not affect plan rendering.

Operator-specific human benefit remains unknown by design. Phase 1 and Pilot V2
exist to produce that evidence. No RPI performance evidence is claimed.

## 17. Deliberately excluded complexity

The following require a future design/version rather than an implementation-time
shortcut:

- dynamic side-chain detectors;
- resonance boosts or arbitrary parametric EQ;
- filter types/topologies other than named v1 profiles;
- delay feedback;
- reverb or convolution;
- generated noise or external riser samples;
- stems/source separation;
- more than two sources;
- arbitrary operation graphs or plugin code;
- learned cue, operator, or parameter generation;
- per-candidate loudness normalization;
- silent runtime simplification.

This scope is intentionally sufficient to test the approved vocabulary and no
larger.

## 18. Adversarial design-review resolutions

The pre-commit architecture review made these ambiguities explicit and resolved
them normatively:

| Review risk | Resolution in this specification |
| --- | --- |
| Hidden ordering/randomness | Semantic cue/geometry IDs, fixed input sorting, fixed template tie order, ID-sorted output, zero Phase 1 DSP seed, and timestamps outside identities |
| Microsecond/floating JSON disagreement | Signed 44.1 kHz frames and integer fixed-point JSON; binary64 exists only in specified DSP evaluation |
| Backend-specific curves/filters | Explicit interpolation equations, RBJ topology, LR4 topology, signed 64-frame control grid, and conformance profiles |
| Plan/headroom hash cycle | Bounded drafts first, then one analytic pair-gain calculation, then final validation/canonicalization/hashing |
| Combinatorial growth | Each named recipe is attempted once against one deterministically bound geometry; six rich families and 43 normal maximum |
| Unsafe arbitrary combinations | Closed operation union plus exact per-template signatures and generic incompatibility rules |
| Limiter concealing bad audio | Conservative shared pre-gain; limiter reduction/activity rejection; no candidate-local repair; activity measured only across the transition/effect window |
| Invalid/fallback recursion | Independently constructed fallback; fallback failure is a terminal pair error; hard-cap failure returns finalized fallback only |
| Accidental same-audio condition noise | Ordinary identical selected PCM is collapsed; only predeclared repeats intentionally duplicate audio |
| Split/critic leakage | Frozen hash chain, development-only fitting/tuning, one validation decision, sealed test, critic restricted to frozen valid IDs |
| Playback-state coupling | Phase 1 modules end at offline artifacts; current-runtime compatibility is descriptive and no lowering is invoked |
| Opaque Spotify semantics | No preset ID or Spotify effect identifier exists in templates, plan kinds, features, or lowering |
| FFmpeg-only assumptions | Canonical plan contains semantic operations only; renderer syntax is provenance; future support requires profile conformance or explicit transformed plan |
| Incomplete operator meaning | Hard-cut step encoding, energy plateau, spectral band thirds, rhythmic cell edges, echo capture, filter state, limiter math, and time-stretch alignment are defined explicitly |
| Unnecessary effect scope | Reverb, noise/riser, stems, arbitrary graphs/hybrids, detector ducking, feedback delay, and per-candidate normalization are absent from v1 |

The remaining items in section 16 are evidence audits, not semantic holes in the
operator plan.

## 19. Next gate

This formal specification must receive explicit human approval before the
Superpowers writing-plans workflow is invoked. Approval authorizes an
implementation plan, not implementation itself.
