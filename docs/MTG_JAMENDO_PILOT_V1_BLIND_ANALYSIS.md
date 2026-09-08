# MTG-Jamendo transition pilot V1: blind-evaluation analysis

Analysis date: 2026-09-08

Decision: **B. Useful learned behavior is visible, but the current transition
rendering/candidate vocabulary limits quality. Improve deterministic transition
operators and run another blind pilot before runtime integration.**

This is a one-rater, 24-pair pilot. Validation and test each contain only five
pairs. Results below are descriptive; no statistical-significance claim is
made. Training results do not count as evidence of generalization.

## Evidence freeze and integrity gate

The completed ratings were validated and frozen before any condition identity
was inspected or reconstructed. The original ratings were never modified.

- Source ratings:
  `C:\Users\janni\Desktop\spotify-transition-pilot-v1-20260907\ratings-completed.json`
- Exact ratings SHA-256:
  `0e1c7f4bf27ec0241af115ca55bc16541a3c86b423c866baa61189b2a923688c`
- Frozen artifact (exact 41,516-byte copy, hash-addressed and Windows read-only):
  `C:\Users\janni\Desktop\spotify-transition-pilot-v1-20260907\analysis\frozen\ratings-completed.sha256-0e1c7f4bf27ec0241af115ca55bc16541a3c86b423c866baa61189b2a923688c.json`
- Completed-ratings validator: zero errors; 24/24 pairs and 72/72 samples
  complete, in frozen order, with valid dense ranks, ratings, scores,
  confidence, flags, timestamps, derived groups, and exact sample hashes.
- Frozen source manifest SHA-256:
  `6370f69a00b10bf2359aedae42b5ed1ae319f239350e7d7ffe5209922b3909dd`
- Frozen sanitized evaluator-manifest SHA-256:
  `489fb0022a3c6972fdc5018075e85e5374164e38b737e8e840b6bf293e09d2b0`
- Frozen ratings-template SHA-256:
  `41d0f47ff50b4e0bf460cd5e785a3ad598e01e7c0a283452a96351591750eae6`
- All 72 FLAC hashes match the ordered aggregate and per-pair manifests. The
  evaluator integrity suite passed 37/37 tests with Node global Web Crypto
  enabled.

## Unblinding and condition mapping

Only after the freeze, the mapping was reproduced from the exact seeded
generator procedure: Python `random.Random("mtg-jamendo-pilot-v1:<pair-id>")`
shuffled the sorted names `learned`, `linear`, and `local_auto`; `learned` is
the small MLP. As an independent cross-check, the reconstructed MLP sample is
the unique audio hash in all 24 pairs, while linear and Auto have the same hash
in every pair.

| Pair | Split | Auto | Linear | MLP |
|---|---|---:|---:|---:|
| 001 | validation | 03 | 02 | 01 |
| 002 | test | 02 | 03 | 01 |
| 003 | train | 01 | 02 | 03 |
| 004 | train | 03 | 01 | 02 |
| 005 | train | 02 | 03 | 01 |
| 006 | train | 02 | 03 | 01 |
| 007 | train | 02 | 03 | 01 |
| 008 | test | 03 | 01 | 02 |
| 009 | validation | 01 | 02 | 03 |
| 010 | train | 02 | 03 | 01 |
| 011 | train | 01 | 02 | 03 |
| 012 | train | 01 | 03 | 02 |
| 013 | train | 02 | 01 | 03 |
| 014 | validation | 01 | 02 | 03 |
| 015 | validation | 02 | 03 | 01 |
| 016 | train | 02 | 03 | 01 |
| 017 | train | 03 | 01 | 02 |
| 018 | test | 02 | 03 | 01 |
| 019 | train | 03 | 02 | 01 |
| 020 | train | 03 | 02 | 01 |
| 021 | test | 03 | 01 | 02 |
| 022 | train | 03 | 02 | 01 |
| 023 | test | 01 | 03 | 02 |
| 024 | validation | 01 | 02 | 03 |

Split definitions are frozen and track-disjoint:

- Train (14): 003, 004, 005, 006, 007, 010, 011, 012, 013, 016, 017,
  019, 020, 022.
- Validation (5): 001, 009, 014, 015, 024.
- Test (5): 002, 008, 018, 021, 023.

## Duplicate/control consistency and noise floor

Every pair contains one identical-audio Auto/linear control: 24/24 pairs.

| Measure on identical audio | Exact agreement | Mean absolute difference |
|---|---:|---:|
| good/meh/bad | 19/24 (79.2%) | 0.208 category steps |
| smoothness 1–5 | 16/24 (66.7%) | 0.375 points |
| musical intent 1–5 | 14/24 (58.3%) | 0.667 points |
| rank / derived ranking group | 6/24 (25.0%) | 0.833 ranks |
| exact flag set | 14/24 (58.3%) | — |

The mean absolute difference across the two 1–5 scores is 0.521 points. In 18
of 24 pairs the two byte-identical samples were placed in different ranking
groups; two were separated by two ranks. Rank is therefore the noisiest
instrument here, consistent with the reported awkward tie-entry UX. It must not
override ratings, scores, comments, or audio-identity controls.

Disagreement was **not** concentrated in low-confidence pairs. Defining lower
confidence as 1–3, only 1/3 such pairs had any core disagreement, versus 18/21
confidence-4–5 pairs. The sole confidence-2 pair agreed on every control
measure. Among confidence-4–5 pairs, control agreement was 76.2% for category,
61.9% for smoothness, 52.4% for intent, and 19.0% for rank. Confidence appears
to describe confidence in the pair preference, not repeat reliability.

For this single-rater pilot, differences smaller than roughly **0.38
smoothness points, 0.67 intent points, or one rank** are below the observed
repeat inconsistency and are not strong evidence. Even larger differences are
uncertain with only five validation or test pairs. The Auto/linear comparison
is a direct negative control: any apparent difference between them is rater or
presentation noise, not a method effect.

## Primary held-out comparisons

W/L/T is from the first-named condition's perspective. Rating uses the ordered
good > meh > bad categories. Lower rank is better. Mean deltas are first minus
second; paired `dz` values are descriptive standardized effects only.

| Split / comparison | Rank W/L/T | Rating W/L/T | Smooth W/L/T | Intent W/L/T | Mean Δ smooth (`dz`) | Mean Δ intent (`dz`) | Mean Δ two-score composite |
|---|---:|---:|---:|---:|---:|---:|---:|
| validation MLP–Auto | 1/4/0 | 1/1/3 | 2/0/3 | 2/1/2 | +0.60 (+0.67) | +0.60 (+0.40) | +0.60 |
| validation MLP–linear | 3/2/0 | 2/1/2 | 3/1/1 | 3/0/2 | +0.60 (+0.53) | +1.00 (+0.82) | +0.80 |
| validation linear–Auto control | 1/4/0 | 0/1/4 | 1/1/3 | 0/1/4 | 0.00 (0.00) | -0.40 (-0.45) | -0.20 |
| test MLP–Auto | 1/4/0 | 1/2/2 | 0/5/0 | 2/1/2 | -1.80 (-4.02) | +0.40 (+0.35) | -0.70 |
| test MLP–linear | 1/4/0 | 1/3/1 | 0/4/1 | 2/2/1 | -1.40 (-1.57) | 0.00 (0.00) | -0.70 |
| test linear–Auto control | 3/1/1 | 1/0/4 | 0/2/3 | 1/0/4 | -0.40 (-0.73) | +0.40 (+0.45) | 0.00 |

Validation score evidence favors MLP, especially against linear, but rank tells
contradictory stories depending on which identical baseline presentation is
used. Indeed, linear versus Auto disagreed in rank on all five validation
pairs. Validation therefore suggests possible value, not a reliable preference
win.

Test is less ambiguous: MLP ranked last in four of five pairs against either
baseline and lost smoothness on every Auto comparison. Its +0.40 intent delta
against Auto is smaller than the 0.667-point control noise floor and disappears
against linear. The -1.4 to -1.8 smoothness deltas are much larger than the
0.375-point control noise floor. Held-out human preference improvement is not
demonstrated.

### Held-out condition distributions

Score distributions list counts at values `[1,2,3,4,5]`; rank distributions
list counts at `[1,2,3]`.

| Split | Condition | good/meh/bad | rank 1/2/3 | Smooth mean [distribution] | Intent mean [distribution] | Flagged samples |
|---|---|---:|---:|---:|---:|---:|
| validation | MLP | 1/3/1 | 1/2/2 | 3.00 [0,1,3,1,0] | 2.40 [1,2,1,1,0] | 2/5 |
| validation | linear | 0/4/1 | 1/1/3 | 2.40 [0,3,2,0,0] | 1.40 [4,0,1,0,0] | 1/5 |
| validation | Auto | 0/5/0 | 3/2/0 | 2.40 [0,3,2,0,0] | 1.80 [3,0,2,0,0] | 1/5 |
| test | MLP | 1/2/2 | 1/0/4 | 1.60 [2,3,0,0,0] | 1.80 [2,2,1,0,0] | 3/5 |
| test | linear | 1/4/0 | 3/2/0 | 3.00 [0,2,1,2,0] | 1.80 [3,0,2,0,0] | 2/5 |
| test | Auto | 0/5/0 | 1/4/0 | 3.40 [0,0,3,2,0] | 1.40 [4,0,1,0,0] | 3/5 |

Pair confidence was 4 for all validation pairs (mean 4.0). Test had two 3s,
two 4s, and one 5 (mean 3.8). These values do not repair the repeat-control
problem and should not be read as measurement precision.

### Failure flags and explanatory comments

- Validation MLP: 2/5 flagged, one `awkward_fade`, one `too_long`. Linear:
  1/5 flagged with `awkward_fade` and `bad_cue`. Auto: 1/5 with
  `awkward_fade`.
- Test MLP: 3/5 flagged, with three `awkward_fade` plus one each of
  `energy_dip`, `too_long`, and `vocal_collision`. Linear: 2/5 flagged, with
  one each of `abrupt_cutoff`, `awkward_fade`, and `too_long`. Auto: 3/5,
  with two `awkward_fade` and one `too_long`.
- Across all 24 pairs, MLP had 14/24 flagged samples and 24 total flags,
  compared with 9/24 and 12 total flags for each baseline. Five MLP samples
  had `beat_error`; nine MLP comments explicitly described excessive overlap,
  simultaneous songs, or material dragging into the next song. No baseline
  sample comment used those overlap/drag descriptions.

Held-out comments distinguish at least two failure modes:

1. **Poor selection or weak intent.** Pair 002 put drums over a slow piano
   section and was called musically inappropriate (MLP smooth/intent 1/1).
   Pairs 008 and 021 were also below their control means on both scores.
2. **Promising intent with poor realization.** Pair 009 explicitly had better
   intent but sounded worse; pair 014 was described as a worthwhile idea that
   sounded like two songs playing simultaneously. Test pair 018 gained two
   intent points over both controls but lost two smoothness points. Pair 023
   similarly gained intent but lost smoothness and was described as two songs
   at once.

The clearest success was validation pair 015: MLP was good, rank 1, smoothness
4, intent 4; the vocal handoff made the start of the second song hard to locate.
Test pair 018 was also rated good and rank 1, though at smoothness 2 and lower
confidence because the source tracks differed strongly.

## Smoothness versus musical intent

The reported tradeoff is supported, but not as a universal law. Across all 24
pairs, MLP versus Auto had 13 intent wins, 6 losses, and 5 ties, while
smoothness had only 5 wins, 12 losses, and 7 ties. Against linear, MLP had
14/6/4 intent outcomes but 6/12/6 smoothness outcomes. Mean MLP scores were
2.54 smoothness and 2.67 intent; Auto was 2.96 and 2.04, and linear was 2.92
and 1.96. Baselines therefore occupy the smoother-but-lower-intent region;
MLP shifts toward intent at a smoothness cost.

Against the mean rating of the two identical controls, MLP was higher-intent
but rougher in 6/24 pairs, higher on both in 6, lower on both in 5, and tied on
at least one dimension in 7. No pair was strictly smoother but lower-intent.
Validation contained two both-higher MLP pairs and three with an equality;
test contained two higher-intent/rougher pairs, two both-lower pairs, and one
equal-intent/rougher pair.

The comments sharpen the mechanism. `crossfade` appeared in 11/24 linear and
11/24 Auto sample comments, and `boring` in five linear and four Auto comments.
MLP comments instead repeatedly mention an interesting idea or intent (eight),
beat issues (five), and overlap/dragging (nine). The learned selector is finding
different, sometimes musically promising candidates, but the available basic
fade/overlap realization often cannot execute those ideas cleanly. This is not
only a renderer problem: several candidates were simply ill-suited or badly
cued. The present pilot cannot fully separate cue/plan selection from operator
quality because both change together.

## Source-material caveat and confidence sensitivity

No pair was removed. The shared source tracks within each pair preserve the
relative comparison even when the MTG-Jamendo material is unpleasant or
unusual.

The lower-confidence pairs (confidence 1–3) were 011 (train, confidence 2),
018 (test, 3), and 023 (test, 3); there were no lower-confidence validation
pairs. Pair 018 explicitly says the dissimilar/inconvenient songs made judging
harder. Pair 002 noted a genre mismatch, pair 006 a strong key mismatch, and
pair 001 uncertainty over whether odd sounds came from the track or transition,
despite confidence 4.

Describing the two lower-confidence test pairs separately does not rescue the
gate. MLP split them 1 rank win and 1 loss, lost smoothness on both (mean -2.0),
and won intent on both (mean +1.5); their mean two-score effect was still -0.25.
On the three confidence-4–5 test pairs, MLP was rank-last 3/3 against both
baselines, had no rating or score wins, and averaged -1.67 smoothness/-0.33
intent versus Auto and -1.00/-1.00 versus linear. Thus the aggregate test intent
advantage over Auto comes from the lower-confidence tradeoff examples, while
the unfavorable preference conclusion becomes stronger at high confidence.

## Descriptive all-pair and training results

These are descriptive only. They mix train, validation, and test and must not
be interpreted as generalization evidence.

| Condition, all 24 | good/meh/bad | rank 1/2/3 | Smooth mean [distribution] | Intent mean [distribution] |
|---|---:|---:|---:|---:|
| MLP | 7/11/6 | 10/5/9 | 2.54 [4,8,7,5,0] | 2.67 [4,8,4,8,0] |
| linear | 5/17/2 | 9/9/6 | 2.92 [1,6,11,6,0] | 1.96 [13,2,6,3,0] |
| Auto | 3/20/1 | 8/15/1 | 2.96 [1,5,12,6,0] | 2.04 [11,3,8,2,0] |

All-pair MLP–Auto outcomes were rank 9/13/2, rating 8/7/9,
smoothness 5/12/7, and intent 13/6/5; mean deltas were -0.42 smoothness,
+0.63 intent, and +0.10 composite. MLP–linear was rank 12/12/0, rating
9/9/6, smoothness 6/12/6, and intent 14/6/4; mean deltas were -0.38,
+0.71, and +0.17. These small composites are below or near control noise.

Training alone (14 pairs) looked more favorable to MLP: rank 7/5/2 versus
Auto and 8/6/0 versus linear, with intent deltas +0.71/+0.86 but smoothness
deltas -0.29/-0.36. This is useful for diagnosis only and is not evidence of
held-out performance.

## Decision and next experiment

**Decision B** is the best fit. Decision A is ruled out because test preference
and smoothness are worse, not improved. Decision D would ignore the consistent
intent shift, the genuine successes, and comments that identify promising ideas
undermined by primitive overlap/fade execution. Decision C understates the
diagnostic consistency of that mechanism, although uncertainty remains high.

Before any runtime integration, expand the deterministic, hard-constrained
candidate/rendering vocabulary. Candidate families should include EQ/spectral
handoffs, low/high-pass sweeps, bass handoff, beat/bar-aligned envelopes, short
rhythmic cuts, reverb or echo tails, controlled ducking, energy ramps,
musically appropriate noise/riser masking, feasible stem-aware handoffs, and
safe combinations. Each operator needs deterministic legality, headroom,
clipping, duration, alignment, and fallback rules. The critic should choose
among good, inspectable ideas; it should not be expected to compensate for a
generator offering only basic crossfades or crude simultaneous overlap.

The next blind experiment should compare the same richer candidate vocabulary
under (1) Auto, (2) a fixed deterministic richer-vocabulary selector, and (3)
the learned critic. This makes learned selection separable from vocabulary
quality. Freeze all code/model/operator versions, candidate sets, plans, audio
hashes, gains, and the answer key before rating. Do not integrate until a new
held-out test shows preference gains larger than its own repeat-control noise.

## Design for the later private 66-track pilot

Recommended size: **30 completely track-disjoint pairs from 60 tracks**, with
an 18/6/6 train/validation/test pair split (36/12/12 tracks). Keep six tracks as
QC/replacement reserves. This uses most of the library while holding the core
evaluation to 90 condition samples. Add an exact hidden repeat on 12
predeclared pairs (six train, three validation, three test), balanced across the
three conditions and presentation positions, for 102 total samples. Evaluate
in sessions of about five pairs to limit fatigue.

- Keep all source and rendered audio private on U-01. Never commit, upload, or
  redistribute it. Use a private, gitignored manifest containing local paths,
  opaque track IDs, and SHA-256 identities; keep the answer key separate.
- Before pairing, decode/probe every candidate file for corruption, bitrate,
  sample rate, channels, duration, peak/true-peak clipping, and obvious encoder
  anomalies. Record tool versions and results by SHA-256. Replace unsuitable
  files only from the six reserves under predeclared QC rules.
- Form 15 naturally compatible and 15 deliberately awkward pairs, balanced as
  9/3/3 of each type across train/validation/test. Use the representative
  library distribution rather than cherry-picking only easy genres, tempos,
  keys, energies, or production styles. Freeze compatibility strata before
  rendering and do not discard difficult pairs after rating.
- Keep tracks disjoint across all pairs and splits. Training is descriptive or
  development-only; validation guides one predeclared decision; test stays a
  lockbox until the final gate.
- Use fixed seeded blind labels and order, shared non-boosting headroom per
  pair, identical preview rules, no technical metadata in the UI, and a
  hash-bound export. The evaluator must validate completeness and hashes before
  export, and ratings must be frozen before unblinding.
- Replace numeric tie entry with explicit ordered groups: drag samples into
  best/middle/worst groups or choose “tie with previous,” preview the resulting
  groups, and require confirmation. Retain category, smoothness, intent,
  confidence, flags, and comments; treat rank as one signal rather than ground
  truth.
- Predeclare primary held-out outcomes and the repeat-derived noise rule. Report
  validation and test separately, preserve every observation, and describe
  lower-confidence/source-difficulty pairs separately without removing them.
