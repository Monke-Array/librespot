# Spotify live transition protocol contract

Updated 2026-09-20; examined official Windows client 1.3.0.277.g5441bb3e.
Repository baseline: `e9585c9`, branch `codex/m3a-live-auto-metadata`.

**Handoff status:** source selection, saved ingress, wire schemas and native
materialization boundaries are specified below. Full audible DSP equivalence is
NOT established. Sol can implement the bounded resolver and capability-gated
renderer without rediscovering these boundaries; the unresolved physical EQ,
filter and FX mappings remain blockers to claiming complete Mixer compatibility.
This distinction is deliberate, following the instruction to preserve a usable
handoff rather than exhaustively reverse every native DSP helper.

## Confidence and evidence

- **PROVEN:** direct captured field/schema or verified native branch/data copy,
  scoped to the recorded client. Code proof is labelled separately from live proof.
- **HIGH CONFIDENCE:** supported semantic interpretation of native dataflow.
- **MEDIUM:** plausible interpretation missing a causal/runtime link.
- **LOW:** weak inference; never an implementation assumption.
- **UNKNOWN:** not established. A safe implementation policy is not an upstream fact.

Durable evidence:

- [Native contract index](../tools/runtime-diagnostics/evidence/2026-09-20-native-contract.json):
  source order, config gates, function RVAs and hashes of private analysis.
- [Saved Blended recipe](../tools/runtime-diagnostics/evidence/2026-09-15-blended.json):
  extension-244 response, decoded recipe, official queue and RPI rejection.
- [Style contract](../tools/runtime-diagnostics/evidence/2026-09-19-style-contract.json):
  full machine-readable service/message field tables, numeric enums, all preset
  mappings, response hashes, dependency probes and jogwheel/looping blocks.
- [Numeric style responses](../tools/runtime-diagnostics/evidence/2026-09-19-style-responses.json):
  all69 completed diagnostic calls, numeric requests/responses only; no client code.
- [Auto observations](../tools/runtime-diagnostics/evidence/2026-09-19-auto-observations.json):
  deduplicated captured windows, local-calculator observations and experiment limits.
- [Style details](SPOTIFY_STYLE_CONTRACT.md): lookup and override semantics.
- `tools/spotify-automix-oracle/README.md`, its pinned input fixtures, config and
  `validate_input_fixtures.js`: existing local Auto oracle; not new live-route proof.

Native references below are RVAs in Spotify.dll SHA256
`65c131dc7f9dcce90eb3874033e6e22ef422a493f0877e3f031805198839b7c4`.
Private extracted client code, disassembly, full logs and raw captures remain
local under `.codex/`; do not commit them. RVAs identify evidence, not runtime APIs.

## 1. Authority and representations

**Required invariant:** Connect owns WHAT plays. A transition determines only HOW
its authoritative A becomes its authoritative B. Neither a recipe, preview,
transition URI nor Auto result can select/replace B or advance the queue.

Keep these representations distinct:

1. Connect context/rows/canonical identities and session/edge generation.
2. Base64 `spotify.automix.proto.Transition`: semantic recipe, possibly saved or
   backend-produced; timing, identities, preset and overrides.
3. Native calculated Auto overlap plus selected preset.
4. Resolved native automation: directional volume/EQ/filter/FX, timing and speed.
5. Materialized string-valued `audio.*` metadata: downstream player representation.

**PROVEN, native:** saved and local Auto results converge on materializer
`0x01040aa4`. **PROVEN, captured:** materialized attributes appear in native
ContextPlayer.GetQueue output before XPUI decoding. They are not evidence of a
backend audio-attribute service. Reproduce semantic resolution in librespot;
do not introduce a dependency on the official desktop's local Esperanto bridge.

## 2. Saved transition ingress and extension-244 hydration

**PROVEN, live:** outgoing A can carry `automix.auto_transition_recipe` and
`automix.transition_uri=spotify:transition:<id>:<revision>`. Despite the attribute
name, the former can contain a saved Blended recipe. Do not classify it as local
Auto merely because its name contains `auto`.

Existing ingress is dealer/context metadata into `ProvidedTrack.metadata`.
`spotify_mix.rs` decodes the inline recipe. If only the real saved-transition URI
is available, retain existing `spotify_mix_hydration.rs`:

- POST `/extended-metadata/v0/extended-metadata` using existing SpClient protobuf
  request transport; `BatchedEntityRequest.entity_request[]` selects entity URI;
  its `ExtensionQuery.extension_kind=244` (`TRANSITION_DATA`).
- Official local MetadataService.Fetch exposes the same extension semantics;
  it is not an additional network endpoint librespot needs to call.
- Response `BatchedExtensionResponse.extended_metadata[]` groups extension kinds;
  matching entity contains protobuf Any of type
  `type.googleapis.com/spotify.playlistmixing.mixtransition.TransitionData`.
- Envelope fields: string transition_uri=1, latest_transition_uri=2,
  playlist_uri=3, bytes item_id=4, string creator_user_id=5, string transition=6,
  track_a_uri=7, track_b_uri=8. Field6 is base64 Transition, not raw protobuf.
  Existing `protocol/proto/transition_data.proto` is wire-compatible but lacks
  the now-observed upstream package. Creator identity is unnecessary for DSP.
- Validate requested URI/revision, playlist, outgoing row bytes and A/B. A newer
  `latest_transition_uri` does not authorize silently replacing the requested
  revision. Discard stale asynchronous results after edge/session replacement.
- Require the provider and entity headers and HTTP-style status `200`, then
  validate the Any type rather than accepting arbitrary Any bytes. Live
  extension-244 and the sanitized extended-metadata corpus both use `200` for
  success; proto3's absent-field default `0` is not success evidence.

**PROVEN, live:** the tested recipe is startA=150580ms, startB=5700ms,
D=12600ms, bars=4, speedA=1, speedB=0.95238173, BPM A/B=76.190475,
item speeds=1, beatmatched=true, preset=2, preset-ID override=true.
RPI decoded it and hydrated extension244 successfully; it rejected
`UnsupportedPresetStyle(2)`. Hydration is not the missing feature.

## 3. Transition schema and timing semantics

Canonical wire field table is `protocol/proto/automix_transition.proto` and the
style evidence's extracted schema. Preserve presence when adapting to Rust:
current native descriptors use proto3; the reconstruction uses proto2 optional
fields to retain absent-vs-explicit values. Missing is not universally zero.

| Message | Fields and meaning |
|---|---|
| Transition | overlap1, preset2, is_overlap_override3, is_preset_id_override4, beatmatch_preference5 |
| Overlap identity | row IDs A/B1/2, canonical URI A/B10/11, playable URI A/B12/13, playable beats hash A/B14/15 |
| Overlap geometry | int32 start_a_ms3, start_b_ms4, duration_ms5; float speed_a6/speed_b7; int32 duration_bars8; bool is_beatmatched9 |
| Overlap analysis | float bpm_a16/bpm_b17; double item_speed_a18/item_speed_b19 |
| Preset | int32 ID1; PresetType2; six style-ID overrides and directional curve/group overrides, exact fields in schema |

Preset ID is NOT PresetType. PresetType unspecified/fade/cut/beatmatched-fade/
beatmatched-cut is an independent enum. BeatmatchPreference AUTO/OFF/ON=0/1/2.
Do not infer a new calculation or overwrite explicit overlap from these flags
alone: their general editor recomputation policy is **UNKNOWN**. Accepted live
stored-recipe materialization consumes the supplied overlap and preset.

**HIGH CONFIDENCE, native:** overlap decoding `0x01043af0` builds directional
geometry; materialization uses separate outgoing and incoming source durations.
Do not collapse unequal source durations into a single fade length, or confuse
source-clock duration with wall-clock overlap. The exact geometry conversion
must be covered by the existing captured speed/timing oracle before extending
current `TransitionPlan` (which presently assumes one duration).

## 4. Native source-selection state machine

**PROVEN, code structure; HIGH CONFIDENCE semantic mapping:** node helper
`0x00806274` implements equivalent current-edge and next-edge candidate loops.
Outgoing row metadata is read into saved-recipe and backend-recipe slots.
Startup constructors `0x000310c4`/`0x00031124` establish their keys; read sites
`0x00807ef7`/`0x00807f4a` and retained-current slots +0x718/+0x738 confirm mapping.

For an eligible Mixer/Auto edge, the order is:

```text
nonempty automix.auto_transition_recipe
    -> nonempty automix.backend_auto_transition IF backend gate enabled
    -> empty recipe input, meaning calculate local Auto
```

A real saved URI must first be hydrated into the saved semantic slot if inline
content is absent. The native loop orders recipe strings, not URI fetches.
Exact simultaneous inline-vs-hydrated revision conflict behavior is **UNKNOWN**;
librespot policy: validate both against the authoritative requested revision and
never let an older async response replace newer validated inline content.

The loop's result is more precise than a flat try-until-success chain:

```text
for candidate in [saved?, enabled_backend?, empty_local]:
    result = resolve(candidate, authoritative_edge, metadata, config)
    if result has automation AND has no auto_preset_id marker:
        accept recipe-derived automation; stop
    if result has automation AND has auto_preset_id marker:
        keep local result provisional; try next candidate if any
    if result has no automation AND candidate is nonempty:
        stop with no Automix result (includes known NONE)
    otherwise continue/end with the available local result or no result
```

Proof: next loop `0x008080f1..0x008082a5`; current loop
`0x00807a25..0x00807bda`. Returned optional present is +0x420; +0x418/+0x41c
is optional computed preset ID, not a generic failure boolean. Local calculation
sets it at `0x01040751`; accepted stored materialization passes it absent.
`0x00811210..0x00811245` publishes that optional value as
`automix.auto_preset_id`. Thus a provisional local result from rejecting saved
content does not incorrectly outrank a subsequent valid backend recipe.

### Gates and failure behavior

- **PROVEN, native:** backend candidate needs both nonempty content and node
  gate +0x4b3 (`0x00807f72` for next; `0x008078d8` current).
  `core-automix/use_backend_auto_transition` exists with default false in the
  config accessor `0x00481df8`. Config constructor0x00482491/0x00482496 stores it at+0xa3; node
  constructor0x007fa60d copies config to+0x410, hence gate+0x4b3.
  Account-effective override is **UNKNOWN**.
- **PROVEN, native:** resolver `0x0103f614` first requires both mixability records
  present/true and both duration metadata present. Otherwise no result; it does
  not promise a recipe-independent fallback. This precedes saved decoding.
- Empty/undecodable recipe, canonical/playable validation failure, item-speed
  mismatch or unknown preset falls through to local calculation in that resolver.
- **PROVEN, native:** known preset0 (`NONE`) reaches `0x01043b9c`, returns no
  automation before style/curve overrides, and is terminal for that nonempty
  candidate. It must NOT mean "missing recipe; generate an Auto transition".
- Unknown preset is different: `0x0103fc98..0x0103fcca` logs/rejects it and
  calculates Auto. Diagnostic style RPC fallback is not preset validity proof.
- Native late/unsupported renderer failure is not proven equivalent to unknown
  preset. **Librespot policy:** unsupported DSP rejects the whole candidate,
  records why, and takes a deterministic supported fallback; never silently
  drops EQ/filter/FX while labelling the result an exact Spotify recipe.

### Backend availability and live proof limits

**HIGH CONFIDENCE, captured:** zero backend attributes in 60 current/next row
observations across 30 deduplicated official queue-stream records, and zero in
236 RPI inventory observations. These are observations, not unique tracks.
The requested-Auto E4 run did not demonstrate backend delivery.

**PROVEN, native representation:** the backend string enters the same resolver
as saved base64 Transition content. No account-delivered payload or dedicated
request endpoint was observed. Implement a guarded metadata adapter, not an
invented backend fetch. The default-disabled gate is meaningful; field presence
alone does not establish official enablement or service negotiation.

`0x00812aa4` is a virtual Automix node update method (constructor `0x007f9b5c`
installs vtable `0x019b52c0`, slot+0x30). Its reference to `automix-preview` does
not make all of `0x00806274` preview-only: it handles node state then invokes the
shared route helper. Captures prove native calculations and saved materialized
queue output; no controlled live backend-vs-saved collision was captured.
The precedence claim is code-derived, not an audible differential-test claim.

## 5. Local Auto and its fallback

**PROVEN, captured:** official native log September15 has16 calculation calls,
12 explicit cuepoint fallbacks. September19 has31 calls/29 cuepoint fallbacks
in a separate user-driven preview session. Counts do not prove audible playback.
RPI selected/scheduled existing local Auto, but those captures lack render-start/
promotion markers. Seeking near end occurred in the RPI comparison; it was not
a natural-transition-only experiment. Cumulative snapshots are not replicates.

Local inputs are already documented/captured by the existing Auto oracle:

| Input | Identity/source |
|---|---|
| Track descriptors (extension6), cuepoints(28), mixability(219) | Canonical requested track |
| Beats(217), vocal activity(218), Audio Attributes v2(222) | Playable track |
| Audio analysis duration/fade bounds/bars/segment loudness | Playable track |
| Row item speed, A/B duration, normalized downbeats, config | Authoritative edge plus analysis |

Check the fixture type URLs as well as numeric extension IDs. Existing local
Auto implementation and oracle tests should be reused, not replaced with ML.
Config fixture distinguishes compiled defaults from verified active values;
active maximum transition35000ms and beatmatch-opt-out duration5000ms were also
observed through the current native getters.

**PROVEN, native:** `0x0103be8c` skips ranked beatmatched candidates when BOTH
beatmatchability scores are below the configured threshold; otherwise it builds
and filters candidates. If no candidate survives, it invokes fallback
`0x0103b6f0`. That fallback uses cuepoints only when both are present, positive,
and in permitted outgoing/incoming end regions. Otherwise it uses an ordinary
end-A/start-B fade geometry. This is an **Auto-internal fallback**, not proof
that user-configured crossfade is unconditionally the next global route.

**UNKNOWN:** the complete outer node/user-crossfade eligibility policy after
Auto returns no result, and all platform preference overrides. Safe librespot
policy: preserve the existing separate normal configurable-crossfade route;
explicit NONE suppresses Mixer generation for that edge; unavailable/unsupported
Mixer data may use the existing deterministic safety fallback, clearly labelled
as local policy. If that policy or disabled crossfade yields no overlap, retain
normal sequential playback. Never claim an invented Spotify fallback order.

## 6. Preset/style resolution and DSP contract

Use [SPOTIFY_STYLE_CONTRACT.md](SPOTIFY_STYLE_CONTRACT.md) plus machine-readable
style evidence for every method, wire number, ID and probe. Key rules:

1. Validate preset against the pinned known set before lookup; NONE is terminal.
2. Expand preset to six style IDs; apply present style-ID overrides, including0.
3. Clamp native materializer numBars to[2,32]. Resolve effective BPM/item speeds.
4. Apply enabled directional curve overrides with native presence semantics.
5. Generate directional timing, volume, EQ/filter/FX parameters and incoming
   speed automation; validate renderer capability before scheduling anything.

Preset2 resolves volume7/EQ1/filter0/FX0/jogwheel0/looping0. Current standard
preset0..22 mappings are in evidence; bundled data alone covers fewer presets.
All queried standard presets disable jogwheel/looping. Explicit nonzero overrides
can activate them; reject these with an explicit unsupported reason initially.
There is no need to reverse their full DSP to support the ordinary preset set.

### Curves and physical values

**PROVEN:** service curve start/end are envelope coordinates; local points use
x/y; two/four-point line/cubic shapes occur. Native override converter copies
points/start/end and ignores CurveSet minimum/maximum. Service bounds often0/0
must NOT scale a nonzero curve to silence. Bundle and service bounds have
DIFFERENT wire numbers (bundle2/3, service3/4).

**HIGH CONFIDENCE:** line/cubic Bezier control interpretation; exact native
x-inversion/evaluation tolerances remain unverified. Reuse tested envelope
infrastructure only after fixture comparison, not by equating cubic parameter t
with x for arbitrary control points.

**UNKNOWN, release blockers:** EQ y-to-dB/gain law and band definitions; filter
y-to-cutoff/resonance/noise mapping; hidden effect feedback/delay/decay and wet/dry
laws; some speed-ramp configuration defaults. EQ/filter neutral-looking0.5 is a
control coordinate, not proof of0.5 gain or a frequency in Hz. Do not infer
physical units from names or normalize every family as volume amplitude.
Sol can implement the typed resolver and retain these controls losslessly now,
but must leave affected render capabilities disabled until verified. Saved
preset2 cannot be declared audibly compatible until EQ rendering is validated.

### BPM, bars, speed and item-speed adjustment

**PROVEN:** volume/EQ curves can depend on bar count, including3bars. Do not
round to a bundle's discrete2/4/8/16/32 entries. FX request has IDs field1,
float BPM field2, int32 numBars field3. The completed probes found identical
returned FX wet/dry envelopes at BPM0/60/120/240 and bars2/4/8; this does NOT
prove internal delay times independent of BPM.

**HIGH CONFIDENCE, native:** for each side use positive supplied BPM, else
`clampedBars*240/outgoingSourceDurationSeconds`, or120 if duration is unusable;
then multiply by that side's item speed (optional absent ->1).
Do not multiply item speed twice or treat it as overlap speed.

**PROVEN, native structure:** beatmatched incoming initial speed is
`incomingSourceDuration/outgoingSourceDuration * itemSpeedA`; target is
itemSpeedB. `0x01047af8` emits initial `{from_position:0,speed:initial}`, then
steps toward target using configured increment, interval and delay after the
supplied position. It clamps the last increment to target. Nonbeatmatched
materialization emits a constant initial speed when itemSpeedB differs from1.
Serializer `0x01047a98` emits JSON keys `from_position` and `speed`;
`0x01047c50` formats the array. Exact ramp config and source-position adjustment
must be verified against fixtures before enabling a newly supported ramp.

**HIGH CONFIDENCE, native AdjustTransitionRecipeForItemSpeeds:** helper
`0x0103f0dc` clones the recipe, takes positive current item speeds when present,
otherwise positive stored speeds or1, and writes item_speed_a/b. Beatmatched
positive overlap speeds cause a configured tolerance check on
`abs(speed_b * itemSpeedA/itemSpeedB - 1)`; failure returns no adjusted recipe.
It does not simply rescale every timestamp. Do not call the editor adjustment
RPC in playback or assume this relaxes edge identity checks. Wire schema is in
the style evidence; retain explicit adjustment provenance if implemented.

### Overrides and absence

**PROVEN:** six style-ID overrides are applied before materialization. Curve
extraction is gated by `core-automix/auto_transition_use_curve_overrides`, native
configuration byte+0xa0. Accessor0x0048163c defaults false; config constructor
0x00482470/0x00482475 stores it at+0xa0. Current effective override is unknown. Volume side override
replaces its curves even if explicitly empty.

**HIGH CONFIDENCE, native:** a present EQ/filter directional group bypasses the
whole corresponding default group; missing siblings become empty, not silently
inherited. UI per-channel nullish fallback is display behavior and differs.
FX style0 ignores FX curve overrides; nonzero FX replaces present wet/dry
channels while retaining generated internal effect parameters. Preserve presence
in the adapter and add focused tests; do not flatten absent/empty/zero together.
Exact effective curve-override gate needs runtime confirmation before claiming
arbitrary custom-curve parity. Safe initial policy is explicit capability rejection
of unverified override forms, rather than silently ignoring them.

## 7. Where materialized audio attributes are produced

**PROVEN, native producer chain:**

```text
stored Transition acceptance (103f614 -> 1043b9c)
OR local candidate + ranked preset (103f614)
    -> common materializer1040aa4
    -> directional serializers1049874 (in),104a158 (out)
    -> node writers810f90/8110ec
    -> metadata insertion808ea0
    -> ContextPlayer queue/state -> native bridge -> XPUI
```

Native directional struct fields: outgoing start+0/duration+8/curves+0x10;
incoming start+0x208/duration+0x210/curves+0x218. This supports separate clocks.
Timing is converted to milliseconds during materialization. Serializer also
emits speed JSON and channel-specific fields such as `audio.fade_*_curves`,
`audio.fade_*_duration`, `audio.fade_*_eq_*_gain_curves`, `audio.filter*`,
`audio.fade_overlap`, and `audio.fade_*_start_time`. Preserve exact names from
existing `spotify_materialized_transition.rs`/captured fixtures; reject unknown
nonempty effect attributes in a capability check, not by silently dropping them.

`spotify:core-auto-transition` is written by the native node as a marker for
calculated transitions; it is not an extension244 resource or authority for B.
`automix.auto_preset_id` is computed-preset output provenance. Neither should
be treated as an independent upstream recipe source.

`automix-preview` is a separate signal/editor transport; decode for diagnostics
only unless explicitly handling a preview operation with its own ownership.
Preview envelope and user-driven September19 activity cannot authorize a normal
live plan. Preview uses shared components, so method names alone do not prove
which route produced a particular queue snapshot.

## 8. Canonical/playable identity and staleness

**PROVEN, native checks:** canonical A/B mismatches reject stored content;
playable identity comparison is conditional on require-playable-match config,
with a beats-hash compatibility path when enabled. Present positive stored item
speeds are compared with available current values using tolerance. Do not erase
canonical/playable splitting by overwriting recipe URIs with decoder URIs.

**Required librespot validation (stricter safe policy):**

- Bind provenance to session, context URI, both row UIDs, canonical A/B,
  authoritative edge generation, requested transition revision and item speeds.
- Retain playable A/B and analysis hashes. Relinking requires explicit compatible
  analysis evidence; canonical match alone does not prove legal audio timing.
- Bound base64/protobuf/curve counts; reject nonfinite values, nonpositive speeds,
  negative/out-of-file positions, impossible durations and unsupported effects.
- Recheck ownership after hydration/analysis, before preload/arm, and at promotion.
  Retire stale decoder/DSP generations on seek, edge or session replacement.
- Cache by semantic identity plus revision/config/style-bundle version; use
  bounded negative caching and retry policy. A desktop cache hit is not a network
  response or a permanent entitlement result.
- Do not publish source selection as execution: carry plan ID/source/rejection
  through prepare, arm, start, cancel and promotion logs.

## 9. Exact current librespot gaps

- Existing saved ingress and extension244 hydration work; do not rewrite them.
- `SpotifyTransitionRecipe::ensure_renderable` unconditionally rejects preset2
  and most preset-only recipes; no general style resolver exists.
- Backend metadata is recognized but not decoded/enabled; add a conditional
  adapter using the same Transition representation, without inventing a service.
- Existing recipe adapter requires inline volume curves and a single duration;
  full directional timing/speed/effect representation is missing.
- Materialized parser rejects EQ/filter/unknown effects; development bypasses
  are not production support. Physical effect mappings remain unverified.
- Current precedence prefers renderable inline/materialized data and speculative
  local Auto; it does not encode the native candidate-result distinctions/NONE.
- Existing hydration is playlist/authentic-row scoped, and must not be broadened
  to arbitrary contexts without matching envelope evidence.
- Add explicit response status/Any type, canonical/playable/generation validation
  and end-to-end provenance tests. Generic dealer/context JSON's allowance for
  unknown fields is not demonstrated to be the cause of missing style output.

## 10. Sol High implementation sequence and acceptance gates

1. **Pure contract layer:** keep existing Transition wrapper; introduce typed
   outcomes `RecipeAccepted`, `ExplicitNone`, `LocalProvisional`, `Unavailable`,
   `Unsupported`. Preserve proto presence, source, identity and config version.
   Tests: saved/backend/local ordering, disabled/absent backend, malformed and
   unknown recipe, NONE with overrides, unmixable/missing duration, stale result.
2. **Ingress hardening:** reuse inline/hydration; validate status/type/envelope;
   add guarded backend base64 adapter. Never fetch the sentinel or use preview
   as normal live ingress. Avoid pretending lack of captured backend means absent
   globally. Test canonical vs playable and conflicting revision responses.
3. **Deterministic style resolver:** generate schema/types from semantic field
   tables, pin verified ID expansion/curves with client/bundle hashes. Native
   Esperanto is diagnostic IPC, not a service available to RPI. Use the durable
   numeric response corpus for exact sampled curves; implement
   and test bar-dependent generation before enabling unsampled bar/ID combinations.
   Response hashes alone are not a resolver. Verify preset2, every enabled ID,
   bars3 and bounds,
   absent/empty/zero overrides, and distinguish lookup fallback from validity.
4. **Lossless render-plan model:** directional source geometry and volume,
   EQ/filter/FX controls plus speed events, with capability validation. Separate
   selection from render support. Keep unknown channel units as typed unresolved
   controls; never map0.5 to a guessed gain/frequency. Nonzero jogwheel/looping and
   unsupported custom override forms remain explicitly unsupported initially.
5. **DSP in independently reviewable increments:** volume/timing first; speed
   after clock/ramp oracle; EQ then filter/FX only after physical mapping and
   audible/reference validation. Exact Spotify compatibility is blocked for any
   enabled preset whose required channel remains unresolved. Fallback is honest
   supported local behavior, not purported exact rendering.
6. **Integrate through existing ownership path:** select against authoritative
   edge, preload secondary, verify PCM, arm, render, promote once. Preserve queue,
   decoder/session generations and current normal-crossfade route. Retain the
   existing deterministic Auto algorithm/oracle and fallback when data unavailable.
7. **Verification:** smallest targeted resolver/adapter/renderer tests, then
   fmt/workspace check/playback/connect tests as applicable; exact ARM candidate;
   one matched account/context/A-B reproduction with logs prepared over SSH.
   Record seeks, active endpoint, source/plan ID and audible outcome. Unit tests
   alone do not close playback/quality parity.

**Safe starting task for Sol:** implement steps1-3 as a diagnostic resolver with
explicit capabilities and fixture tests; do not promise preset2 audible parity
until step5 EQ mapping is verified. No more ingress/source-selection archaeology
is needed for this bounded task. Remaining DSP and outer-fallback uncertainties
above are explicit release gates, not hidden assumptions or permission to guess.

## Verification of this documentation/evidence change

September20: four new evidence JSON files parsed; all69 numeric style calls
match their two private source captures exactly; preset2 mapping and sampled FX
envelope invariance checked;10 document links resolve; credential-field scan
and `git diff --check` pass. No production or investigation-tool code changed.
Existing `validate_input_fixtures.js` completed:9/9 beats hashes and107/107 preset
matches;105/107 geometry and22/107 exact speed-bit matches, maximum speed delta
7.6294e-6. Its score status remains `not-yet-exact`. This is a consistency report,
not proof of complete local Auto parity; no new audible validation is claimed.
