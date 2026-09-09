# Pilot V1 to transition feature snapshot v2 audit

This is the sanitized Task 20 evidence record for the M4 feature boundary. The
machine-readable disposition map is
`tools/transition-operator/tests/fixtures/features/pilot-v1-audit.json`.

## Frozen evidence

The audited repository was
`/home/profdrhuso/Projects/spotify-transition-ml-pilot-v1` on S-01. The feature
implementation revision is
`227c7d04c9831a6531d8ac5b552bf0559dc9ffff`; the Pilot V1 freeze revision is
`2afa90d3c864eca3dbf42bde4c59c688981db5f8`. A direct Git diff shows that the
later revision changes only `docs/CODEX_STATE.md` and the ratings template. The
audited extractor, candidate schema, vector schema, and tests have identical
blob IDs at both revisions.

The completed Pilot job used `pilot-analysis/v1` from `transition_ml/pilot.py`
through `tools/build_pilot_v1.py`. Its recorded command selected the Python
3.8 environment used for the run. Read-only inspection of that same environment
reported Python 3.8.20, NumPy 1.22.4, librosa 0.8.0, pyloudnorm 0.1.0, and
SoundFile 0.10.3. No private report, source manifest, audio, or rating content
was read or copied.

## Semantic findings

Pilot V1 analyzes a whole-track 22.05 kHz mono downmix. It emits rounded
milliseconds, floating values, and whole-track summaries. Snapshot v2 instead
uses exact frames from canonical 44.1 kHz stereo PCM, integer ppm/mdb units, and
values tied to explicit transition windows. Consequently, duration, cues,
rhythm, transients, bass, spectrum, energy, and peaks require v2 recomputation.

Three potentially misleading Pilot meanings are explicitly rejected:

- `true_peak_dbfs` is a mono sample peak converted to dB. It is not an
  intersample true-peak measurement.
- cue confidence is always `0.7`; cue distance fields are always zero. Neither
  is measured evidence.
- `downbeat_times_ms` is every fourth librosa beat, with its phase inherited
  from the first detected beat. Invalid tempo or too few points cause a regular
  synthetic grid. It is not independently detected downbeat evidence.

Pilot V1 declares `vocal_probability` as optional, never extracts it, and marks
it missing in generated candidate provenance. It is therefore omitted at the
v2 import boundary. A missing vocal measurement remains absent and may not be
converted to zero or “vocal free.”

Pilot V1 critic-vector normalization is also not a source-feature contract.
That vectorizer zero-fills optional numbers only alongside explicit presence
bits and applies model-specific clipping/scaling. Snapshot v2 stores measured
integer units; scorer normalization cannot be imported as measurement meaning.

## Disposition and contradiction gate

There are no `reuse_exact` fields. All measured categories are `recompute_v2`;
vocal evidence is `omit_v2` until supplied by a separately approved extractor
or import. This is compatible with the approved M4 plan: Task 22 defines local
recomputation and requires unavailable imports to remain absent. The formal v2
contract can represent every required measurement, so no specification
contradiction or architecture change is required.

S-01 was used read-only and remains an offline evidence source, never a runtime
dependency.
