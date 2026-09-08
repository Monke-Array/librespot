# Offline Transition Operator Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the approved deterministic transition-template system, canonical
`OperatorPlan/v1`, and offline reference-render/evaluation pipeline through a
completed private Pilot V2 analysis without changing live playback.

**Architecture:** A standalone Rust tool under `tools/transition-operator`
owns typed plans, strict validation, deterministic candidate generation, native
binary64 DSP, typed FFmpeg/Rubber Band lowering, rendering, QC, and private
manifest preparation. A dependency-free Node application under
`tools/transition-pilot-v2` owns blind-set freezing, the local browser evaluator,
ratings freezing, and analysis. The two tools exchange canonical, versioned,
hash-linked JSON; neither is a dependency of librespot playback.

**Tech Stack:** Rust 1.85, `serde`, `serde_json`, `sha2`, `thiserror`, `clap`,
Node.js 18 built-ins, FFmpeg/ffprobe 9.0.1, FFmpeg `rubberband`, HTML/CSS, and
synthetic PCM fixtures.

**Spec:** `docs/superpowers/specs/2026-09-08-transition-operator-design.md` at
commit `25da884a0c2e648d9cf86fc938d3efade827f389`.

**Status:** Documentation plan awaiting human approval. No implementation is
authorized by this document's existence.

## Global constraints

- Phase 1 is offline-only. Do not modify `playback/`, `connect/`, spotifyd, the
  queue/context state machine, decoder lifetime, buffering, promotion, reconnect,
  or scheduling ownership.
- The offline Rust package is a standalone workspace. Do not add it to the root
  Cargo graph and do not add a path dependency on any librespot crate.
- Do not run `cargo update`. Commit the standalone tool's generated `Cargo.lock`.
- `OperatorPlan/v1` contains only the seven approved operation kinds and the nine
  approved templates. No reverb, noise/riser, stems, arbitrary graph, detector
  ducking, feedback delay, arbitrary hybrid, backend syntax, or Spotify preset ID.
- All semantic JSON uses signed 44.1 kHz frames and the spec's integer units,
  canonical bytes, strict schemas, domain-separated hashes, and fixed operation
  order. Invalid plans cannot construct `ValidatedPlan` or reach a renderer.
- The generator seed is exactly `0000000000000000`; maximum normal accepted
  candidates is 48, absolute hard cap is 64, and `safe_crossfade/v1` is mandatory.
- Every pair uses one shared non-boosting pre-gain. Output safety is the exact
  `transition_output_safety_v1` profile with a -1,200 mdbFS sample ceiling,
  -1,000 mdbTP target, and rejection above 3,000 mdb reduction or 50,000 ppm
  activity.
- Private source paths, annotations, rendered music, answer keys, ratings, and
  private manifests stay under
  `C:\Users\janni\Desktop\spotify-transition-private-v2` or another explicitly
  supplied directory outside Git. Tests create only synthetic audio in temporary
  directories.
- A critic receives the frozen ordered set of valid candidate IDs and feature
  vectors. It cannot create IDs, cues, operations, parameters, or runtime state.
- A schema-compatible frozen offline critic artifact is an input to Pilot V2
  freezing. This plan implements and validates that interface; it does not train
  or integrate a runtime MLP. If no approved compatible artifact exists at M6,
  the freeze command must fail closed and a separate model-training decision is
  required.
- Do not generate the final blind Pilot V2 set until M1-M5 and the evaluator's
  synthetic-fixture tests are complete.
- Every implementation task follows red-green-refactor: write the named test,
  observe the stated failure, add only the specified behavior, rerun the focused
  test, then run the task's regression command before committing.
- For every numbered task, acceptance means its named focused tests pass, its
  `Produces` interface and task-specific assertions are present, all global
  constraints still hold, `git diff --check` passes, and its commit contains
  only the listed files. A milestone gate adds requirements; it never weakens
  this per-task definition.

## Proposed module and file layout

```text
tools/transition-operator/
  Cargo.toml
  Cargo.lock
  README.md
  schemas/
    operator-plan-v1.schema.json
    feature-snapshot-v2.schema.json
    geometry-proposal-v1.schema.json
    renderer-capabilities-v1.schema.json
    render-request-v1.schema.json
    candidate-set-v1.schema.json
    candidate-record-v2.schema.json
    candidate-features-v2.schema.json
  src/
    lib.rs
    error.rs
    scalar.rs
    canonical.rs
    model.rs
    identity.rs
    validation/{mod.rs,structural.rs,contextual.rs,template.rs}
    capability.rs
    compatibility.rs
    geometry.rs
    safety.rs
    candidate.rs
    generator.rs
    templates/{mod.rs,common.rs,safe.rs,gain.rs,spectral.rs,dynamics.rs,tail.rs}
    dsp/{mod.rs,pcm.rs,envelope.rs,filter.rs,crossover.rs,dynamics.rs,delay.rs,time_stretch.rs,limiter.rs,meter.rs}
    render/{mod.rs,request.rs,source.rs,environment.rs,ffmpeg.rs,artifact.rs,qc.rs}
    features/{mod.rs,snapshot.rs,extract.rs,import.rs,vector.rs,scorer.rs}
    pilot/{mod.rs,inventory.rs,pairing.rs,pipeline.rs}
    bin/transition-operator.rs
  tests/
    boundary.rs
    canonical.rs
    validation.rs
    identity.rs
    geometry.rs
    templates.rs
    generator.rs
    dsp_envelope.rs
    dsp_filter.rs
    dsp_time_stretch.rs
    dsp_safety.rs
    renderer.rs
    performance.rs
    feature_audit.rs
    features.rs
    candidate_features.rs
    pipeline.rs
    corpus.rs
    fixtures/{plans,features,geometry,pcm}/
    node/canonical-golden.test.js

tools/transition-pilot-v2/
  README.md
  package.json
  bin/pilot-v2.js
  lib/{stable-json.js,schema.js,selection.js,repeats.js,freeze.js,unblind.js,analysis.js}
  evaluator/{index.html,evaluator.css,browser.js,core.js,controller.js,ui.js,server.js}
  test/{schema.test.js,selection.test.js,freeze.test.js,evaluator-core.test.js,evaluator-controller.test.js,evaluator-page.test.js,evaluator-ui.test.js,evaluator-server.test.js,unblind.test.js,analysis.test.js}
  test/fixtures/

docs/
  TRANSITION_FEATURE_SCHEMA_AUDIT.md
  PILOT_V2_ANALYSIS_PROTOCOL.md
  PRIVATE_PILOT_V2_BLIND_ANALYSIS.md
```

Generated private manifests and audio are never placed in either tool directory.
The only committed fixtures containing samples are programmatically generated
synthetic vectors or small integer arrays represented as JSON.

## Dependency chain and parallel lanes

```text
M1 canonical contracts
  -> M2 generator
  -> M3 renderer and QC
  -> M4 end-to-end offline pipeline
  -> M5 private corpus freeze
  -> M6 blind render set and evaluator
  -> human ratings
  -> M7 analysis
```

- Tasks 9-12 may run in parallel after Task 8 because they own separate template
  modules and use the frozen template interface.
- Tasks 15-17 may run in parallel after Task 14 because filter, time-stretch,
  and safety DSP have separate modules and golden fixtures.
- Task 20's S-01 audit may run in parallel with Tasks 15-18 after M2, but no
  feature extractor/import implementation may start before the audit passes.
- Tasks 29-31 may run in parallel with private annotation work in Tasks 27-28
  after the Pilot V2 schemas are frozen; final dataset generation still waits for
  M5, the renderer stability gate, the evaluator gate, and a frozen critic.
- Canonical types, schemas, ID formulas, template interfaces, feature vector
  indices, and blind-manifest schemas are dependency-critical and must not be
  edited by parallel workers.

## Planning assumptions and evidence gates

No contradiction was found in the approved specification. If an implementation
detail in this plan conflicts with the specification, the specification wins and
execution stops for review rather than silently changing either document.

Three planned evidence inputs remain deliberately external to the core IR:

1. Task 20 audits the exact Pilot V1 feature schema before serialized feature
   extraction/import is implemented.
2. Task 25 requires human-reviewed private artist/album/style annotations before
   pairing can freeze.
3. Task 32 requires a separately frozen critic artifact whose input schema is
   exactly `transition-candidate-features/2`. Core implementation supplies the
   scorer boundary but does not invent or train this artifact.

Each absence is a fail-closed milestone prerequisite, not permission to add a
default, leak held-out data, or broaden the operator vocabulary.

---

## M1 — Canonical `OperatorPlan`, validators, and identities

### Task 1: Create the isolated offline-tool boundary

**Files:**
- Create: `tools/transition-operator/Cargo.toml`
- Create: `tools/transition-operator/Cargo.lock`
- Create: `tools/transition-operator/src/lib.rs`
- Create: `tools/transition-operator/src/bin/transition-operator.rs`
- Create: `tools/transition-operator/tests/boundary.rs`
- Create: `tools/transition-operator/README.md`
- Modify: `.gitignore`

**Interfaces:**
- Produces: standalone crate `transition-operator`, library name
  `transition_operator`, and CLI binary `transition-operator`.
- Depends on: approved spec only.

- [ ] **Step 1: Write the failing repository-boundary test.** Assert that the
  tool manifest contains its own empty `[workspace]`, has no `librespot-*` path
  dependency, and that no Rust source under the tool imports `librespot_playback`
  or `librespot_connect`.

```rust
#[test]
fn offline_tool_has_no_runtime_dependency() {
    let manifest = std::fs::read_to_string("Cargo.toml").unwrap();
    assert!(manifest.contains("[workspace]"));
    assert!(!manifest.contains("librespot-playback"));
}
```

- [ ] **Step 2: Run the test and observe failure.**

Run: `cargo test --manifest-path tools/transition-operator/Cargo.toml --test boundary`

Expected: FAIL because the standalone package and test target do not exist.

- [ ] **Step 3: Add the minimal standalone package.** Pin Rust 1.85-compatible
  `serde`, `serde_json`, `sha2`, `thiserror`, `clap`, and test-only `tempfile`;
  add an empty `[workspace]`; add a CLI whose only initial command is `version`.
  Ignore only tool-local `target/`, `.private/`, and `artifacts/` defense-in-depth
  paths. Do not touch root `Cargo.toml` or root `Cargo.lock`.

- [ ] **Step 4: Verify the focused deliverable.**

Run: `cargo test --manifest-path tools/transition-operator/Cargo.toml --test boundary`

Expected: PASS; `cargo run --manifest-path tools/transition-operator/Cargo.toml -- version`
prints a static tool/schema version without probing audio or runtime state.

- [ ] **Step 5: Commit.**

```powershell
git add .gitignore tools/transition-operator
git commit -m "tools: scaffold offline transition operator"
```

**Acceptance:** The root workspace graph is unchanged and the new tool cannot
link against playback accidentally.

### Task 2: Implement scalar arithmetic and canonical JSON

**Files:**
- Create: `tools/transition-operator/src/scalar.rs`
- Create: `tools/transition-operator/src/canonical.rs`
- Create: `tools/transition-operator/src/error.rs`
- Create: `tools/transition-operator/tests/canonical.rs`
- Create: `tools/transition-operator/tests/fixtures/plans/canonical-object.json`
- Modify: `tools/transition-operator/src/lib.rs`

**Interfaces:**
- Produces: `div_round_nearest_away(i64, i64) -> Result<i64>`,
  `canonical_json<T: Serialize>(&T) -> Result<Vec<u8>>`, and
  `require_canonical_json<T: DeserializeOwned + Serialize>(&[u8]) -> Result<T>`.
- Depends on: Task 1.

- [ ] **Step 1: Write failing table tests.** Cover positive and negative ties,
  the JavaScript-safe integer limits, ASCII key ordering, no whitespace/newline,
  and rejection of BOM, floats, exponent notation, `null`, duplicate keys,
  unknown keys, and noncanonical key order.

```rust
assert_eq!(div_round_nearest_away(-3, 2).unwrap(), -2);
assert_eq!(canonical_json(&fixture).unwrap(), br#"{"a":1,"z":0}"#);
assert_eq!(require_canonical_json::<StrictFixture>(br#"{"z":0,"a":1}"#)
    .unwrap_err().code(), "NON_CANONICAL_JSON");
```

- [ ] **Step 2: Run and observe missing-module/test failures.**

Run: `cargo test --manifest-path tools/transition-operator/Cargo.toml --test canonical`

- [ ] **Step 3: Implement checked integer arithmetic and a canonical serializer.**
  Convert typed values through `serde_json::Value`, recursively reject noninteger
  numbers, sort schema keys by ASCII bytes, emit shortest integers, preserve
  UTF-8 string values, and compare accepted input bytes exactly with
  reserialization. Typed deserializers use `deny_unknown_fields` so duplicates
  and unknown members fail; plan-specific printable-ASCII enforcement belongs to
  Tasks 3-4 so private annotation text is not corrupted.

- [ ] **Step 4: Run focused and library tests.**

Run: `cargo test --manifest-path tools/transition-operator/Cargo.toml canonical`

- [ ] **Step 5: Commit.**

```powershell
git add tools/transition-operator
git commit -m "feat(transition-operator): add canonical integer JSON"
```

**Acceptance:** Canonical bytes are independent of input map order; invalid JSON
is rejected rather than repaired.

### Task 3: Define the complete `OperatorPlan/v1` type system

**Files:**
- Create: `tools/transition-operator/src/model.rs`
- Create: `tools/transition-operator/schemas/operator-plan-v1.schema.json`
- Create: `tools/transition-operator/tests/fixtures/plans/safe-crossfade.json`
- Create: `tools/transition-operator/tests/fixtures/plans/all-operations.json`
- Create: `tools/transition-operator/tests/validation.rs`
- Modify: `tools/transition-operator/src/lib.rs`

**Interfaces:**
- Produces: `OperatorPlan`, `OperatorPlanBody`, `TemplateRef`, `AudioFormat`, `SourceRef`,
  `Timeline`, `Operation`, all seven typed operation structs, `OutputSafety`,
  `FeatureSnapshotRef`, and `Provenance` using `i64`/closed enums only.
- Depends on: Task 2.

- [ ] **Step 1: Write compile-failing construction and strict-parse tests.** The
  safe fixture must contain exactly two primary gains; the all-operations fixture
  exercises every approved tagged union variant. Add rejection fixtures for
  unknown kinds, backend command text, Spotify preset fields, non-ASCII plan
  strings, floats, and nulls.

```rust
let plan: OperatorPlan = require_canonical_json(SAFE_BYTES).unwrap();
assert_eq!(plan.schema_version, "transition-operator-plan/1");
assert_eq!(plan.operations.len(), 2);
```

- [ ] **Step 2: Run and observe unresolved type failures.**

Run: `cargo test --manifest-path tools/transition-operator/Cargo.toml --test validation model`

- [ ] **Step 3: Add closed serde types and the documentation schema.** Use
  `#[serde(deny_unknown_fields)]` on every object, a closed tagged `Operation`
  enum, and exact v1 field names. Do not add an extension map or generic effect.

- [ ] **Step 4: Verify round-trip and rejection tests.**

Run: `cargo test --manifest-path tools/transition-operator/Cargo.toml --test validation`

- [ ] **Step 5: Commit.**

```powershell
git add tools/transition-operator
git commit -m "feat(transition-operator): define operator plan v1"
```

**Acceptance:** Every approved plan field is typed; every excluded operator is
unrepresentable.

### Task 4: Enforce structural and contextual validation

**Files:**
- Create: `tools/transition-operator/src/validation/mod.rs`
- Create: `tools/transition-operator/src/validation/structural.rs`
- Create: `tools/transition-operator/src/validation/contextual.rs`
- Create: `tools/transition-operator/src/validation/template.rs`
- Modify: `tools/transition-operator/tests/validation.rs`
- Modify: `tools/transition-operator/src/model.rs`

**Interfaces:**
- Produces: `PlanValidator::validate_body(body, context) ->
  Result<ValidatedPlanBody>` and `PlanValidator::validate_structure(plan,
  context) -> Result<StructurallyValidatedPlan>`. Both newtypes have private
  fields and read-only plan/report accessors; final hash/ID validation is added
  in Task 5.
- Consumes: `OperatorPlanBody` or `OperatorPlan`; `ValidationContext` supplies source bounds and a
  `TemplateFeatureView` without defining the serialized feature schema yet.
- Depends on: Task 3.

- [ ] **Step 1: Write failing boundary matrices.** Add accept-at-bound and
  reject-one-outside cases for timeline, points, gains, filter frequency/Q,
  crossover count/order, duck depth/ramp, delay taps/sum/tail, rhythmic limits,
  time-map rate, operation count/point count/byte size, ordering, combinations,
  source bounds, safety constants, feature hash, and exact template signatures.

- [ ] **Step 2: Run and confirm invalid plans currently pass or lack APIs.**

Run: `cargo test --manifest-path tools/transition-operator/Cargo.toml --test validation`

- [ ] **Step 3: Implement the ten-stage validation order from spec section 14.**
  Return stable codes and bounded safe messages; never reorder, clamp, default,
  or mutate. Make renderer-facing APIs accept only `&ValidatedPlan`.

- [ ] **Step 4: Run validation tests.**

Run: `cargo test --manifest-path tools/transition-operator/Cargo.toml --test validation`

- [ ] **Step 5: Commit.**

```powershell
git add tools/transition-operator
git commit -m "feat(transition-operator): validate operator plans"
```

**Acceptance:** Invalid or context-mismatched plans cannot obtain the renderer
capability token represented by `ValidatedPlan`.

### Task 5: Implement plan, candidate, and capability identities

**Files:**
- Create: `tools/transition-operator/src/identity.rs`
- Create: `tools/transition-operator/src/capability.rs`
- Create: `tools/transition-operator/src/compatibility.rs`
- Create: `tools/transition-operator/schemas/renderer-capabilities-v1.schema.json`
- Create: `tools/transition-operator/tests/identity.rs`
- Modify: `tools/transition-operator/src/model.rs`

**Interfaces:**
- Produces: `finalize_plan(ValidatedPlanBody) -> Result<ValidatedPlan>`,
  `PlanValidator::validate_bytes(bytes, context) -> Result<ValidatedPlan>`,
  `candidate_id(&PlanHash) -> CandidateId`,
  `derive_capabilities(&OperatorPlan) -> CapabilityRequirements`,
  `compare_capabilities(req, profile) -> SupportResult`, and
  `classify_current_transition_plan(&OperatorPlan) -> CurrentCompatibility`.
- Depends on: Tasks 2-4.

- [ ] **Step 1: Write failing identity vectors.** Assert exact SHA-256 values for
  the full plan projection, audio-semantics projection, domain-separated
  candidate ID, capability ordering, and frame/nanosecond exact-round-trip
  compatibility. Assert equal-power/filter/duck/tail/gate plans are not silently
  labeled current-lowerable.

- [ ] **Step 2: Run and observe missing identity functions.**

Run: `cargo test --manifest-path tools/transition-operator/Cargo.toml --test identity`

- [ ] **Step 3: Implement raw-byte domain separation, full ID recomputation, and
  derived capabilities.** `ValidatedPlan` has private fields and exposes only
  read-only plan, canonical bytes, hash, capability, and validation-report
  accessors.
  Capability comparison returns only `Supported`,
  `SupportedWithSimplification { transform_id }`, or `Unsupported`; no transform
  is executed here. Compatibility is descriptive and creates no runtime object.

- [ ] **Step 4: Verify identities and compatibility.**

Run: `cargo test --manifest-path tools/transition-operator/Cargo.toml --test identity`

- [ ] **Step 5: Commit.**

```powershell
git add tools/transition-operator
git commit -m "feat(transition-operator): add stable plan identities"
```

**Acceptance:** IDs contain no enumeration position or timestamp, and rich plans
never degrade into current `TransitionPlan` semantics. This task deliberately
implements compatibility classification and exact frame/nanosecond proof only;
it creates no `TransitionPlan` converter and imports no playback crate.

### Task 6: Freeze cross-language golden vectors

**Files:**
- Create: `tools/transition-operator/tests/node/canonical-golden.test.js`
- Create: `tools/transition-operator/tests/fixtures/plans/golden-identities.json`
- Modify: `tools/transition-operator/tests/canonical.rs`
- Modify: `tools/transition-operator/README.md`
- Modify: `docs/CODEX_STATE.md`

**Interfaces:**
- Produces: immutable golden canonical bytes and expected hashes consumed by
  both Rust and an independent Node implementation.
- Depends on: Task 5.

- [ ] **Step 1: Write the Node test against deliberately wrong expected hashes.**
  The Node implementation recursively sorts ASCII keys and hashes with Node
  `crypto`; it must not shell out to the Rust binary.

- [ ] **Step 2: Run and observe hash mismatch.**

Run: `node --test tools/transition-operator/tests/node/canonical-golden.test.js`

- [ ] **Step 3: Replace wrong values with reviewed Rust/Node-agreed vectors.**
  Include safe crossfade, every operation kind, negative frames, maximum safe
  integer, audio-semantics projection, plan ID, candidate ID, and capability set.

- [ ] **Step 4: Run both independent implementations.**

```powershell
cargo test --manifest-path tools/transition-operator/Cargo.toml --test canonical --test identity
node --test tools/transition-operator/tests/node/canonical-golden.test.js
```

- [ ] **Step 5: Commit.**

```powershell
git add tools/transition-operator docs/CODEX_STATE.md
git commit -m "test(transition-operator): freeze canonical golden vectors"
```

**M1 gate:** Run all Rust tests plus the Node golden test. Review the schema and
fixture hashes before allowing generator work. M1 is complete only if the root
Cargo manifests and every playback/connect file are unchanged.

## M2 — Deterministic candidate generator

### Task 7: Add cue, window, and geometry identity/shortlisting

**Files:**
- Create: `tools/transition-operator/src/geometry.rs`
- Create: `tools/transition-operator/schemas/geometry-proposal-v1.schema.json`
- Create: `tools/transition-operator/tests/geometry.rs`
- Create: `tools/transition-operator/tests/fixtures/geometry/geometry-set.json`

**Interfaces:**
- Produces: `CueId`, `FeatureWindowId`, `GeometryProposal`,
  `geometry_id(&GeometryProposalCore)`, and
  `shortlist_geometries(Vec<GeometryProposal>) -> Result<Vec<GeometryProposal>>`.
- Depends on: M1.

- [ ] **Step 1: Write failing tests for semantic IDs and shortlist order.**
  Permute input order, duplicate semantic proposals, omit windows, exceed source
  bounds, and exercise the `cut, short, medium, long` round-robin up to 12.

- [ ] **Step 2: Run and observe missing geometry APIs.**

Run: `cargo test --manifest-path tools/transition-operator/Cargo.toml --test geometry`

- [ ] **Step 3: Implement canonical content-derived IDs, validation, buckets,
  and stable shortlist selection.** Add the exact five-second
  `fallback_geometry` precondition and no random/index-derived identity.

- [ ] **Step 4: Run geometry tests.**

Run: `cargo test --manifest-path tools/transition-operator/Cargo.toml --test geometry`

- [ ] **Step 5: Commit.**

```powershell
git add tools/transition-operator
git commit -m "feat(transition-operator): identify transition geometries"
```

### Task 8: Freeze template registry, predicates, and recipe binding

**Files:**
- Create: `tools/transition-operator/src/templates/mod.rs`
- Create: `tools/transition-operator/src/templates/common.rs`
- Create: `tools/transition-operator/src/templates/safe.rs`
- Create: `tools/transition-operator/tests/templates.rs`
- Modify: `tools/transition-operator/src/model.rs`

**Interfaces:**
- Produces: closed `TemplateId`, `RecipeId`, `TemplateDraft`, `RejectionRecord`,
  `TemplateFamily::applicability(&TemplateInputs)`, and
  `bind_recipe(recipe, geometries) -> Result<&GeometryProposal>`.
- Depends on: Task 7.

- [ ] **Step 1: Write failing registry tests.** Assert exactly nine template IDs,
  immutable family tie order, exact predicate thresholds, missing-as-false,
  recipe-index modulo binding, one attempt per recipe, and safe fallback's exact
  two linear gain envelopes over 220,500 frames.

- [ ] **Step 2: Run and observe missing registry failures.**

Run: `cargo test --manifest-path tools/transition-operator/Cargo.toml --test templates registry`

- [ ] **Step 3: Implement the sealed registry and common resolver.**
  `TemplateInputs` is an internal typed view populated by synthetic fixtures for
  M2; serialized feature extraction remains blocked until Task 20.

- [ ] **Step 4: Run registry tests.**

Run: `cargo test --manifest-path tools/transition-operator/Cargo.toml --test templates registry`

- [ ] **Step 5: Commit.**

```powershell
git add tools/transition-operator
git commit -m "feat(transition-operator): define template registry"
```

### Task 9: Emit shaped, beat-cut, and energy-ramp plans

**Files:**
- Create: `tools/transition-operator/src/templates/gain.rs`
- Modify: `tools/transition-operator/tests/templates.rs`

**Interfaces:**
- Produces all eight `shaped_handoff/v1`, four `beat_cut/v1`, and six
  `energy_ramp/v1` recipes as `TemplateDraft` values.
- Depends on: Task 8.

- [ ] **Step 1: Write failing recipe golden tests.** Assert exact frames,
  quarter-sine/cosine pairing, smoothstep asymmetry, hard-step `(-1,0)` encoding,
  click-safety suppression, non-boosting energy plateau, 25/75% positions, and
  filter-assisted energy direction.

- [ ] **Step 2: Run and confirm recipe IDs are unimplemented.**

Run: `cargo test --manifest-path tools/transition-operator/Cargo.toml --test templates gain_`

- [ ] **Step 3: Implement only the named recipes and exact signatures.** Use
  checked integer rounding helpers; emit optional time map only where allowed.

- [ ] **Step 4: Run focused template tests.**

Run: `cargo test --manifest-path tools/transition-operator/Cargo.toml --test templates gain_`

- [ ] **Step 5: Commit.**

```powershell
git add tools/transition-operator
git commit -m "feat(transition-operator): add gain handoff templates"
```

### Task 10: Emit bass and spectral handoff plans

**Files:**
- Create: `tools/transition-operator/src/templates/spectral.rs`
- Modify: `tools/transition-operator/tests/templates.rs`

**Interfaces:**
- Produces eight `bass_handoff/v1` and eight `spectral_handoff/v1` drafts.
- Depends on: Task 8; may run parallel with Tasks 9, 11, and 12.

- [ ] **Step 1: Write failing recipe tests.** Assert the fixed 140/180/220 Hz
  bass recipes, 25-50% and 40-60% complementary ownership, 882-frame wet edges,
  the six LP/HP recipes, two three-band recipes, exact third rounding, and no
  corresponding-band sum above unity at validator sample points.

- [ ] **Step 2: Run and observe missing spectral recipes.**

Run: `cargo test --manifest-path tools/transition-operator/Cargo.toml --test templates spectral_`

- [ ] **Step 3: Implement fixed recipe tables and typed crossover/filter ops.**
  Do not expose generic EQ or arbitrary crossover topology.

- [ ] **Step 4: Run focused tests.**

Run: `cargo test --manifest-path tools/transition-operator/Cargo.toml --test templates spectral_`

- [ ] **Step 5: Commit.**

```powershell
git add tools/transition-operator
git commit -m "feat(transition-operator): add spectral handoff templates"
```

### Task 11: Emit resolved duck and rhythmic handoff plans

**Files:**
- Create: `tools/transition-operator/src/templates/dynamics.rs`
- Modify: `tools/transition-operator/tests/templates.rs`

**Interfaces:**
- Produces eight `ducked_overlap/v1` and four `rhythmic_handoff/v1` drafts.
- Depends on: Task 8; may run parallel with Tasks 9, 10, and 12.

- [ ] **Step 1: Write failing golden tests.** Cover localized versus sustained
  collision, deterministic target/tie choice, clipped hold window, all depth and
  attack/release recipes, beat-cell ramps, 50% duty, decay endpoints, eight-per-
  second rejection, vocal/transient predicates, and explicit final zero.

- [ ] **Step 2: Run and observe missing dynamics recipes.**

Run: `cargo test --manifest-path tools/transition-operator/Cargo.toml --test templates dynamics_`

- [ ] **Step 3: Implement resolved envelopes only.** No detector fields or
  signal-dependent behavior may enter a draft.

- [ ] **Step 4: Run focused tests.**

Run: `cargo test --manifest-path tools/transition-operator/Cargo.toml --test templates dynamics_`

- [ ] **Step 5: Commit.**

```powershell
git add tools/transition-operator
git commit -m "feat(transition-operator): add bounded dynamics templates"
```

### Task 12: Emit feed-forward echo-tail plans

**Files:**
- Create: `tools/transition-operator/src/templates/tail.rs`
- Modify: `tools/transition-operator/tests/templates.rs`

**Interfaces:**
- Produces four `echo_tail_handoff/v1` drafts.
- Depends on: Task 8; may run parallel with Tasks 9-11.

- [ ] **Step 1: Write failing tests for all four tap tables.** Assert beat-fraction
  round-away resolution, capture bounds, monotone delay/gain, gain sum, 20 ms dry
  cut, exact `effect_end_frame`, sparse-exit predicates, and `dry_start_frame`
  including capture.

- [ ] **Step 2: Run and observe missing tail recipes.**

Run: `cargo test --manifest-path tools/transition-operator/Cargo.toml --test templates tail_`

- [ ] **Step 3: Implement fixed feed-forward recipes.** Do not add feedback,
  reverb, damping, or a generic delay network.

- [ ] **Step 4: Run focused tests.**

Run: `cargo test --manifest-path tools/transition-operator/Cargo.toml --test templates tail_`

- [ ] **Step 5: Commit.**

```powershell
git add tools/transition-operator
git commit -m "feat(transition-operator): add echo tail template"
```

### Task 13: Assemble bounded candidate sets and shared safety

**Files:**
- Create: `tools/transition-operator/src/safety.rs`
- Create: `tools/transition-operator/src/candidate.rs`
- Create: `tools/transition-operator/src/generator.rs`
- Create: `tools/transition-operator/schemas/candidate-set-v1.schema.json`
- Create: `tools/transition-operator/tests/generator.rs`
- Modify: `tools/transition-operator/src/lib.rs`
- Modify: `docs/CODEX_STATE.md`

**Interfaces:**
- Produces: `generate_candidates(GenerationRequest) -> PairGenerationResult`,
  `CandidateSet`, private in-memory `CandidateRecordDraft`, `CandidateSetHash`, and stable rejection
  diagnostics.
- Depends on: Tasks 7-12.

- [ ] **Step 1: Write failing end-to-end generator tests.** Cover family need
  ordering and six-family retention, recipe order, exception isolation, semantic
  dedup, final candidate-ID sort, candidate-set hash, 51 quota sum/43 normal max,
  soft round-robin pruning, simulated attempt 65, accepted candidate 65, no-rich,
  all-rich-failed, invalid fallback, and guaranteed five-second fallback.

- [ ] **Step 2: Add safety tests before implementation.** Assert
  `min(0, -1200 - P - 6021 - M)`, whole-source true-peak requirement, margins
  0/1000/3000, below -24,000 rejection, byte-identical safety in every surviving
  plan, and fallback-only recomputation after hard-cap failure.

- [ ] **Step 3: Run and observe generator failures.**

Run: `cargo test --manifest-path tools/transition-operator/Cargo.toml --test generator`

- [ ] **Step 4: Implement the spec section 8 algorithm exactly.** Drafts are
  private nonserializable values. Resolve one pair gain after bounded drafts,
  validate/canonicalize/hash, deduplicate by audio semantics, enforce caps, and
  record safe bounded diagnostics without paths or stack dumps.

- [ ] **Step 5: Run generator plus M1 regressions.**

```powershell
cargo test --manifest-path tools/transition-operator/Cargo.toml --test generator
cargo test --manifest-path tools/transition-operator/Cargo.toml
node --test tools/transition-operator/tests/node/canonical-golden.test.js
```

- [ ] **Step 6: Commit.**

```powershell
git add tools/transition-operator docs/CODEX_STATE.md
git commit -m "feat(transition-operator): generate bounded candidate sets"
```

**M2 gate:** Repeated generation and permuted geometry inputs produce identical
candidate IDs, plans, rejection audits, and set hashes. No valid request returns
without a fallback; no input yields more than 64 accepted candidates.

## M3 — Offline reference DSP renderer and QC

### Task 14: Add canonical PCM, render request, and typed source/backend boundaries

**Files:**
- Create: `tools/transition-operator/src/dsp/mod.rs`
- Create: `tools/transition-operator/src/dsp/pcm.rs`
- Create: `tools/transition-operator/src/render/mod.rs`
- Create: `tools/transition-operator/src/render/request.rs`
- Create: `tools/transition-operator/src/render/source.rs`
- Create: `tools/transition-operator/src/render/environment.rs`
- Create: `tools/transition-operator/src/render/ffmpeg.rs`
- Create: `tools/transition-operator/schemas/render-request-v1.schema.json`
- Create: `tools/transition-operator/tests/renderer.rs`

**Interfaces:**
- Produces: `PcmBuffer`, `RenderRequest`, `ValidatedRenderRequest`,
  `SourceLocator` trait, `PrivateManifestLocator`, `RendererEnvironment`,
  `FfmpegBackend`, and `render_identity(plan, request, environment)`.
- Depends on: M2.

- [ ] **Step 1: Write failing synthetic source tests.** Assert s16le stereo
  ingestion values, complete PCM hash, exact frame count, private path exclusion
  from identity, request canonicalization, 4,096 warm-up, output/effect bounds,
  missing source/hash mismatch rejection, and exact render identity domain bytes.

- [ ] **Step 2: Write a fake-backend injection test.** It must prove invalid
  `OperatorPlan` bytes cannot call a `SourceLocator` or spawn FFmpeg.

- [ ] **Step 3: Run and observe missing render infrastructure.**

Run: `cargo test --manifest-path tools/transition-operator/Cargo.toml --test renderer source_`

- [ ] **Step 4: Implement typed command construction.** FFmpeg decodes/resamples
  only to canonical headerless PCM and encodes supplied PCM24 to metadata-free
  FLAC. Arguments are separate `Command::arg` values; plan text is never command
  syntax. Freeze executable/version/flags/locale/thread counts in environment ID.

- [ ] **Step 5: Run focused tests with fake backend and installed FFmpeg.**

Run: `cargo test --manifest-path tools/transition-operator/Cargo.toml --test renderer source_`

- [ ] **Step 6: Commit.**

```powershell
git add tools/transition-operator
git commit -m "feat(transition-operator): add offline render boundary"
```

### Task 15: Implement gain, duck, gate, cut, and delay DSP

**Files:**
- Create: `tools/transition-operator/src/dsp/envelope.rs`
- Create: `tools/transition-operator/src/dsp/dynamics.rs`
- Create: `tools/transition-operator/src/dsp/delay.rs`
- Create: `tools/transition-operator/tests/dsp_envelope.rs`
- Create: `tools/transition-operator/tests/fixtures/pcm/envelope-vectors.json`

**Interfaces:**
- Produces: `eval_envelope`, `apply_gain`, `apply_duck`, `apply_gate`, and
  `render_feedforward_tail` over `PcmBuffer`.
- Depends on: Task 14; may run parallel with Tasks 16-17.

- [ ] **Step 1: Write failing sample-exact vectors.** Cover half-open endpoints,
  holds, hard-step right-continuity, linear/smoothstep/quarter-sine/quarter-cosine,
  equal-power squared sum tolerance, non-boosting multiplication, gate clicks,
  delay capture exclusion, tap sum, tail end, and zero-initialized delay state.

- [ ] **Step 2: Run and observe missing DSP functions.**

Run: `cargo test --manifest-path tools/transition-operator/Cargo.toml --test dsp_envelope`

- [ ] **Step 3: Implement direct binary64 semantics from sections 5.1-5.9.**
  Do not route these operators through FFmpeg.

- [ ] **Step 4: Run focused tests.**

Run: `cargo test --manifest-path tools/transition-operator/Cargo.toml --test dsp_envelope`

- [ ] **Step 5: Commit.**

```powershell
git add tools/transition-operator
git commit -m "feat(transition-operator): render gain dynamics and tails"
```

### Task 16: Implement RBJ filters and LR4 crossover

**Files:**
- Create: `tools/transition-operator/src/dsp/filter.rs`
- Create: `tools/transition-operator/src/dsp/crossover.rs`
- Create: `tools/transition-operator/tests/dsp_filter.rs`
- Create: `tools/transition-operator/tests/fixtures/pcm/filter-vectors.json`

**Interfaces:**
- Produces: `BiquadDf2t`, `apply_filter_envelope`, and
  `apply_crossover_band_gain` with continuous per-channel state.
- Depends on: Task 14; may run parallel with Tasks 15 and 17.

- [ ] **Step 1: Write failing coefficient, impulse, and response vectors.** Cover
  LP/HP at min/max cutoff and Q, signed `64*k` control grid, per-frame coefficient
  interpolation, wet/dry equation, warm-up, fixed two-band LR4, specified
  three-band topology, and recombination tolerances.

- [ ] **Step 2: Run and observe missing filters.**

Run: `cargo test --manifest-path tools/transition-operator/Cargo.toml --test dsp_filter`

- [ ] **Step 3: Implement the exact RBJ DF-II-transposed and cascaded LR4 math.**
  Keep topology closed; reject unsupported profiles in capability validation.

- [ ] **Step 4: Run focused tests.**

Run: `cargo test --manifest-path tools/transition-operator/Cargo.toml --test dsp_filter`

- [ ] **Step 5: Commit.**

```powershell
git add tools/transition-operator
git commit -m "feat(transition-operator): render filters and crossovers"
```

### Task 17: Implement the incoming time-stretch backend adapter

**Files:**
- Create: `tools/transition-operator/src/dsp/time_stretch.rs`
- Create: `tools/transition-operator/tests/dsp_time_stretch.rs`
- Create: `tools/transition-operator/tests/fixtures/pcm/time-map-fixtures.json`
- Modify: `tools/transition-operator/src/render/ffmpeg.rs`

**Interfaces:**
- Produces: `TimeStretchBackend` trait and
  `RubberBandTimeStretch::process(input, rate, output_frames, cue) -> Result<PcmBuffer>`.
- Depends on: Task 14; may run parallel with Tasks 15-16.

- [ ] **Step 1: Write failing conformance tests.** Generate impulses and tones in
  temporary files; assert exact output length, cue within one frame, pitch within
  one cent, transient within 221 frames, integrated level within 100 mdb, and
  continuous-window processing for rates 920,000, 1,000,000, and 1,080,000 ppm.

- [ ] **Step 2: Run and observe unsupported time-map errors.**

Run: `cargo test --manifest-path tools/transition-operator/Cargo.toml --test dsp_time_stretch`

- [ ] **Step 3: Implement a typed FFmpeg `rubberband` lowering.** Request enough
  source frames, remove only measured algorithmic padding/latency, and reject a
  short or misaligned result rather than zero-padding audible source; reject a
  backend/version that cannot meet fixtures. Record command-program
  hash privately, never in `OperatorPlan`.

- [ ] **Step 4: Run the pinned-backend conformance test twice.**

Run: `cargo test --manifest-path tools/transition-operator/Cargo.toml --test dsp_time_stretch -- --test-threads=1`

- [ ] **Step 5: Commit.**

```powershell
git add tools/transition-operator
git commit -m "feat(transition-operator): add bounded time stretch"
```

### Task 18: Implement output safety, true peak, and measurements

**Files:**
- Create: `tools/transition-operator/src/dsp/limiter.rs`
- Create: `tools/transition-operator/src/dsp/meter.rs`
- Create: `tools/transition-operator/tests/dsp_safety.rs`
- Create: `tools/transition-operator/tests/fixtures/pcm/bs1770-4x-vectors.json`

**Interfaces:**
- Produces: `apply_pair_gain`, `LookaheadPeakLimiter`,
  `measure_true_peak_bs1770_4x`, `measure_loudness`, and `SafetyMeasurements`.
- Depends on: Tasks 14-15; filter/time stretch outputs are inputs but not build
  dependencies.

- [ ] **Step 1: Write failing limiter state vectors.** Assert inclusive
  221-frame lookahead, immediate attack, exact release multiplier, linked stereo,
  zero handling, end zero-extension, 100 mdb activity threshold, transition-only
  activity fraction, and whole-render maximum reduction.

- [ ] **Step 2: Write failing BS.1770 vectors.** Include impulses at each of four
  phases, near-Nyquist tones, stereo maxima, boundary handling, PCM24 round-away
  quantization, and rejection before a required clamp.

- [ ] **Step 3: Run and observe missing meter/limiter failures.**

Run: `cargo test --manifest-path tools/transition-operator/Cargo.toml --test dsp_safety`

- [ ] **Step 4: Implement the exact safety profile.** Return measurements at the
  five named stages and stable rejection codes for nonfinite samples, >-1,000
  mdbTP, >3,000 mdb reduction, or >50,000 ppm activity. Never perform a second
  normalization pass.

- [ ] **Step 5: Run focused tests.**

Run: `cargo test --manifest-path tools/transition-operator/Cargo.toml --test dsp_safety`

- [ ] **Step 6: Commit.**

```powershell
git add tools/transition-operator
git commit -m "feat(transition-operator): enforce output safety"
```

### Task 19: Assemble the deterministic reference renderer and QC artifacts

**Files:**
- Create: `tools/transition-operator/src/render/artifact.rs`
- Create: `tools/transition-operator/src/render/qc.rs`
- Modify: `tools/transition-operator/src/render/mod.rs`
- Modify: `tools/transition-operator/tests/renderer.rs`
- Create: `tools/transition-operator/tests/performance.rs`
- Create: `tools/transition-operator/tests/fixtures/plans/render-suite.json`
- Modify: `docs/CODEX_STATE.md`

**Interfaces:**
- Produces: `ReferenceRenderer::render(&ValidatedPlan, &ValidatedRenderRequest,
  &SourceLocator) -> Result<RenderRecord>` and canonical PCM24/FLAC artifacts.
- Depends on: Tasks 14-18.

- [ ] **Step 1: Write failing stage-order and isolation tests.** Use spies and
  synthetic audio to assert time-map, spectral, dynamics, tail capture, primary
  gains, sum, pair gain, limiter, measurement, and quantization order for a valid
  canonical plan. A fixture with a permuted operation array must be rejected
  before any spy/backend call rather than rendered in input order. Assert exact
  output length and 4,096-frame warm-up.

- [ ] **Step 2: Write failing QC/fallback tests.** Cover source mismatch, decode,
  bounds, nonfinite, cue, channel/rate, peak, limiter, encode/decode hash, rich
  rejection isolation, all-rich failure, and terminal fallback render failure.

- [ ] **Step 3: Run and observe missing renderer behavior.**

Run: `cargo test --manifest-path tools/transition-operator/Cargo.toml --test renderer`

- [ ] **Step 4: Implement the fixed pipeline and artifact record.** Render PCM24
  first, hash interleaved three-byte samples, encode metadata-free FLAC, decode it
  back, and require identical PCM hash. Store backend program text only in a
  renderer-private provenance object.

- [ ] **Step 5: Run deterministic repetition and the ignored performance
  characterization.** Render gain, filter/crossover, tail, gate, limiter, and
  time-stretch plans three times each with one thread and assert identical PCM
  hashes. The ignored performance test renders 64 synthetic 36-second candidates
  and writes non-normative elapsed, render/audio ratio, peak RSS, and temporary
  storage observations. On U-01 it rejects total time above 300 seconds
  (render/audio ratio above 0.131), peak RSS above 512 MiB, temporary storage
  above 512 MiB, or more than one uncommitted candidate artifact at once. These
  loose architecture bounds catch runaway work without turning the preparation
  report's much faster observations into a microbenchmark requirement.

```powershell
cargo test --manifest-path tools/transition-operator/Cargo.toml --test renderer -- --test-threads=1
cargo test --manifest-path tools/transition-operator/Cargo.toml --test performance -- --ignored --test-threads=1
```

- [ ] **Step 6: Run the M3 regression gate and commit.**

```powershell
cargo test --manifest-path tools/transition-operator/Cargo.toml
node --test tools/transition-operator/tests/node/canonical-golden.test.js
git add tools/transition-operator docs/CODEX_STATE.md
git commit -m "feat(transition-operator): render and verify offline transitions"
```

**M3 gate:** All approved operators render from the same validated IR; repeated
PCM hashes match in the pinned environment; QC cannot repair a bad candidate;
fallback failure is terminal. No RPI or playback code exists.

## M4 — Feature, scorer, and complete offline pipeline

### Task 20: Audit Pilot V1 feature evidence on S-01

**Files:**
- Create: `docs/TRANSITION_FEATURE_SCHEMA_AUDIT.md`
- Create: `tools/transition-operator/tests/fixtures/features/pilot-v1-audit.json`
- Create: `tools/transition-operator/tests/feature_audit.rs`

**Interfaces:**
- Produces: a sanitized `transition-feature-audit/1` mapping every Pilot V1 field
  to `reuse_exact`, `migrate_with_named_transform`, `recompute_v2`, or `omit_v2`,
  with source code/artifact revision and units.
- Depends on: M2. This is the first task allowed to wake S-01.

- [ ] **Step 1: Write the failing audit-completeness test.** It requires every
  formal feature category, exact old/new units, source-window semantics,
  extractor version, missing-value semantics, and an explicit disposition.

- [ ] **Step 2: Run and observe failure because the audit fixture is absent.**

Run: `cargo test --manifest-path tools/transition-operator/Cargo.toml --test feature_audit`

- [ ] **Step 3: Retrieve evidence read-only.** Run `host-status S-01`; if offline,
  record ownership and run `wake-host S-01`. After SSH identity is reachable,
  inspect `/home/profdrhuso/Projects/spotify-transition-ml-pilot-v1` at commits
  `227c7d0` and `2afa90d` using `git show`, `rg`, and test/schema files. Do not
  retrieve audio, credentials, private manifests, or ratings. Record exact
  field provenance and semantic differences in the two committed audit files.

- [ ] **Step 4: Apply the contradiction gate.** If an old field has incompatible
  semantics, classify it for recomputation/omission. If the approved v2 feature
  contract itself cannot represent required evidence, stop and request a formal
  spec revision; do not reinterpret a field.

- [ ] **Step 5: Run the audit test and preserve remote power ownership rules.**

Run: `cargo test --manifest-path tools/transition-operator/Cargo.toml --test feature_audit`

- [ ] **Step 6: Commit.**

```powershell
git add docs/TRANSITION_FEATURE_SCHEMA_AUDIT.md tools/transition-operator/tests
git commit -m "docs: audit transition feature schema"
```

**Acceptance:** Feature implementation has a reviewed evidence map; S-01 is not
a runtime dependency and was not modified.

### Task 21: Implement `transition-feature-snapshot/2` and explicit migration

**Files:**
- Create: `tools/transition-operator/src/features/mod.rs`
- Create: `tools/transition-operator/src/features/snapshot.rs`
- Create: `tools/transition-operator/src/features/import.rs`
- Create: `tools/transition-operator/schemas/feature-snapshot-v2.schema.json`
- Create: `tools/transition-operator/tests/features.rs`
- Create: `tools/transition-operator/tests/fixtures/features/snapshot-v2.json`

**Interfaces:**
- Produces: `FeatureSnapshotV2`, `validate_feature_snapshot`,
  `snapshot_sha256`, `TemplateFeatureView for FeatureSnapshotV2`, and named
  migration/import functions justified by Task 20.
- Depends on: Task 20; feature-dependent code must not precede it.

- [ ] **Step 1: Write failing strict-schema tests.** Cover source/analysis IDs,
  cue/beat/downbeat/meter/tempo, every window ID/range, vocal/transient/bass/
  spectral/energy fields, whole-source peaks, missing-as-absent, ppm/mdb bounds,
  closed versions, and hash mismatch.

- [ ] **Step 2: Write failing migration tests from sanitized Pilot V1 fixtures.**
  Assert only `reuse_exact` and named-transform fields migrate; recompute/omit
  fields remain absent and suppress dependent templates.

- [ ] **Step 3: Run and observe missing schema/import APIs.**

Run: `cargo test --manifest-path tools/transition-operator/Cargo.toml --test features snapshot_`

- [ ] **Step 4: Implement strict v2 types and explicit pure import.** No generic
  passthrough map, implicit zero, or old model vector is accepted as v2.

- [ ] **Step 5: Run focused tests and commit.**

```powershell
cargo test --manifest-path tools/transition-operator/Cargo.toml --test features snapshot_
git add tools/transition-operator
git commit -m "feat(transition-operator): version transition feature snapshots"
```

### Task 22: Implement deterministic local feature extraction and geometry proposal

**Files:**
- Create: `tools/transition-operator/src/features/extract.rs`
- Modify: `tools/transition-operator/src/geometry.rs`
- Modify: `tools/transition-operator/tests/features.rs`
- Create: `tools/transition-operator/tests/fixtures/pcm/feature-signals.json`

**Interfaces:**
- Produces: `extract_signal_features(&CanonicalSource) -> SignalFeatures` and
  `propose_geometries(&FeatureSnapshotV2) -> Result<GeometrySet>`.
- Consumes: audited cue/beat/downbeat/vocal imports plus locally recomputed whole
  peak, loudness/energy, variability, bass/spectral occupancy, transient/click
  safety, overlap/collision, and source-bound data.
- Depends on: Tasks 18 and 21.

- [ ] **Step 1: Write failing synthetic signal tests.** Use silence, tones,
  impulses, bass-only, treble-only, clipped, vocal-activity fixture values, and
  shifted beat grids to assert exact integer units/window IDs and deterministic
  geometry/fallback selection.

- [ ] **Step 2: Run and observe missing extraction/proposal APIs.**

Run: `cargo test --manifest-path tools/transition-operator/Cargo.toml --test features extract_`

- [ ] **Step 3: Implement only Phase 1-required cheap measurements and the
  audited import boundary.** Algorithms, windows, versions, and capability flags
  are hashed. Missing imported features remain absent; do not introduce source
  separation or a new learned extractor.

- [ ] **Step 4: Run feature and generator integration tests.**

Run: `cargo test --manifest-path tools/transition-operator/Cargo.toml --test features --test generator`

- [ ] **Step 5: Commit.**

```powershell
git add tools/transition-operator
git commit -m "feat(transition-operator): extract offline transition features"
```

### Task 23: Freeze candidate feature vector and scorer boundary

**Files:**
- Create: `tools/transition-operator/src/features/vector.rs`
- Create: `tools/transition-operator/src/features/scorer.rs`
- Create: `tools/transition-operator/schemas/candidate-features-v2.schema.json`
- Create: `tools/transition-operator/tests/candidate_features.rs`
- Modify: `tools/transition-operator/src/candidate.rs`

**Interfaces:**
- Produces: `CandidateFeatureVectorV2 { values: Vec<i64>, present: BitSet }`,
  `CandidateFeatureSchemaV2`, `build_candidate_features`, `CandidateScorer`,
  `DeterministicScorer`, and `score_valid_set`.
- Depends on: Tasks 19 and 22.

- [ ] **Step 1: Write failing index golden tests.** Freeze every index, scale,
  clipping bound, presence bit, closed template/recipe vocabulary, and technical
  render feature. Assert absent stores integer zero with presence false.

- [ ] **Step 2: Write failing scorer-boundary tests.** Reject unknown/new IDs,
  missing/extra/nonfinite/out-of-range scores, condition labels, filenames,
  artist/title inputs, and score ties not resolved by candidate ID. Assert invalid
  critic output falls back to deterministic scorer, then safe fallback.

- [ ] **Step 3: Run and observe missing vector/scorer APIs.**

Run: `cargo test --manifest-path tools/transition-operator/Cargo.toml --test candidate_features`

- [ ] **Step 4: Implement the frozen pure vector builder and ID-only scorer
  protocol.** Serialize scores as signed integer millionths; normalization is a
  separate hash-addressed development-only artifact.

- [ ] **Step 5: Run focused tests and commit.**

```powershell
cargo test --manifest-path tools/transition-operator/Cargo.toml --test candidate_features
git add tools/transition-operator
git commit -m "feat(transition-operator): define candidate scoring boundary"
```

### Task 24: Build the hash-linked pair pipeline and CLI

**Files:**
- Create: `tools/transition-operator/src/pilot/mod.rs`
- Create: `tools/transition-operator/src/pilot/pipeline.rs`
- Create: `tools/transition-operator/schemas/candidate-record-v2.schema.json`
- Create: `tools/transition-operator/tests/pipeline.rs`
- Modify: `tools/transition-operator/src/bin/transition-operator.rs`
- Modify: `tools/transition-operator/README.md`
- Modify: `docs/CODEX_STATE.md`

**Interfaces:**
- Produces CLI commands `ingest`, `features`, `generate`, `render`,
  `score`, `verify-pair`, and `verify-environment`, each taking explicit input and
  output roots; produces canonical `transition-candidate-record/2` values with
  plan/hashes, input-manifest reference, feature vector, template explanation,
  applicability/validation audit, derived capabilities, renderer/artifact
  identity, measurements/QC, scorer provenance, and current-runtime compatibility,
  plus bounded rejection/render/QC manifests.
- Depends on: Tasks 13, 19, 22, and 23.

- [ ] **Step 1: Write a failing temporary-directory end-to-end test.** Ingest two
  synthetic sources, extract/import features, generate candidates, render, QC,
  build feature vectors, score, and verify every source→feature→geometry→plan→
  renderer→PCM hash edge. Run the same command twice and compare canonical bytes.

- [ ] **Step 2: Add failure tests.** Cover invalid plan before renderer, stale
  source/feature/environment hash, failed rich candidate, failed fallback,
  unwritable output, private path in public diagnostic, and output root inside a
  Git-tracked directory.

- [ ] **Step 3: Run and observe missing CLI commands.**

Run: `cargo test --manifest-path tools/transition-operator/Cargo.toml --test pipeline`

- [ ] **Step 4: Wire existing modules without duplicating validation.** Each
  command writes atomically through a temporary sibling and rename, records tool
  revision/config/environment, and refuses overwrite unless existing bytes have
  the expected identity.

- [ ] **Step 5: Run the complete M4 gate and commit.**

```powershell
cargo test --manifest-path tools/transition-operator/Cargo.toml
node --test tools/transition-operator/tests/node/canonical-golden.test.js
git diff --check
git add tools/transition-operator docs/CODEX_STATE.md
git commit -m "feat(transition-operator): assemble offline transition pipeline"
```

**M4 gate:** A synthetic pair produces a reproducible, valid fallback plus rich
candidates, rendered PCM hashes, measurements, candidate features, scores, and
records with no runtime dependency. Review environment and throughput evidence
before touching private Pilot V2 manifests.

## M5 — Private Pilot V2 corpus freeze

### Task 25: Validate and annotate the private 66-track inventory

**Files:**
- Create: `tools/transition-operator/src/pilot/inventory.rs`
- Create: `tools/transition-operator/tests/corpus.rs`
- Modify: `tools/transition-operator/src/bin/transition-operator.rs`
- Modify: `tools/transition-operator/README.md`

**Interfaces:**
- Produces CLI `corpus verify`, `corpus annotation-template`, and
  `corpus validate-annotations`; consumes the known private inventory and writes
  a private `transition-private-annotations/1` manifest outside Git.
- Depends on: M4.

- [ ] **Step 1: Write failing synthetic inventory tests.** Assert SHA-256 and
  canonical PCM identity validation, 66 unique sources, no mutation, required
  artist/album/style fields, normalized group IDs, technical suitability, and
  rejection of paths outside the authorized source root.

- [ ] **Step 2: Run and observe missing corpus commands.**

Run: `cargo test --manifest-path tools/transition-operator/Cargo.toml --test corpus inventory_`

- [ ] **Step 3: Implement read-only validation and a private annotation
  worksheet.** Never print titles/paths in normal logs. The owner reviews/fills
  artist, album, broad style, and pairing-exclusion notes in
  `C:\Users\janni\Desktop\spotify-transition-private-v2`; only hashes/counts may
  enter repository state.

- [ ] **Step 4: Rehash the real 66 sources and validate completed annotations.**
  Fail if any file is missing/changed or any of the 66 rows is unreviewed.

- [ ] **Step 5: Run tests and commit tool code only.**

```powershell
cargo test --manifest-path tools/transition-operator/Cargo.toml --test corpus inventory_
git status --short
git add tools/transition-operator
git commit -m "feat(transition-operator): validate private pilot inventory"
```

**Acceptance:** Originals match the prior inventory; annotation data and names
remain outside Git.

### Task 26: Construct and freeze track-disjoint pairs and splits

**Files:**
- Create: `tools/transition-operator/src/pilot/pairing.rs`
- Modify: `tools/transition-operator/tests/corpus.rs`
- Modify: `tools/transition-operator/src/bin/transition-operator.rs`
- Modify: `docs/CODEX_STATE.md`

**Interfaces:**
- Produces CLI `corpus propose-pairs` and `corpus freeze-pairs`; outputs private
  `transition-pilot-v2-pairs/1` with 60 selected tracks, six reserves, 30 pairs,
  strata, splits, seed, rationale features, and source/annotation hashes.
- Depends on: Task 25.

- [ ] **Step 1: Write failing synthetic pairing tests.** Assert exactly ten
  compatible, ten moderate, ten awkward-but-legal pairs; 6/2/2 of each stratum in
  18/6/6 splits; every selected track appears once; reserves appear zero times;
  artist/album groups are split-disjoint where feasible; input order does not
  affect output; and no pair is chosen by template/render score.

- [ ] **Step 2: Run and observe missing pairing APIs.**

Run: `cargo test --manifest-path tools/transition-operator/Cargo.toml --test corpus pairing_`

- [ ] **Step 3: Implement seeded constrained matching.** Compatibility strata
  use only frozen pre-transition track features and private annotations. Record
  every rejected constraint and deterministic tie-break; do not hand-edit the
  output to favor new operators.

- [ ] **Step 4: Generate a proposal, review only diversity/leakage/legality, and
  freeze exact private bytes.** If constraints cannot yield the exact allocation,
  fail with counts rather than relaxing silently.

- [ ] **Step 5: Verify and commit code only.**

```powershell
cargo test --manifest-path tools/transition-operator/Cargo.toml --test corpus
git status --short
git add tools/transition-operator docs/CODEX_STATE.md
git commit -m "feat(transition-operator): freeze track-disjoint pilot pairs"
```

**M5 gate:** Record only private-manifest hashes and aggregate counts in
`docs/CODEX_STATE.md`; verify all 66 source hashes again and verify Git has no
audio, private paths, titles, artist names, album names, or private manifests.

## M6 — Pilot V2 rendering, blind freeze, and browser evaluator

### Task 27: Define blind/pilot schemas and condition selection

**Files:**
- Create: `tools/transition-pilot-v2/package.json`
- Create: `tools/transition-pilot-v2/README.md`
- Create: `tools/transition-pilot-v2/lib/stable-json.js`
- Create: `tools/transition-pilot-v2/lib/schema.js`
- Create: `tools/transition-pilot-v2/lib/selection.js`
- Create: `tools/transition-pilot-v2/lib/repeats.js`
- Create: `tools/transition-pilot-v2/test/schema.test.js`
- Create: `tools/transition-pilot-v2/test/selection.test.js`
- Create: `tools/transition-pilot-v2/test/fixtures/candidate-set.json`

**Interfaces:**
- Produces strict validators for private selection, sanitized blind manifest,
  ratings template, and completed ratings; `selectConditions(candidateSet,
  deterministicScores, criticScores)`; `collapseIdenticalPcm`; and
  `assignRepeats`.
- Depends on: M4 schemas. May precede completion of private annotation.

- [ ] **Step 1: Write failing schema and selection tests.** Assert fallback,
  deterministic, and critic select only IDs from the same frozen set; critic may
  select fallback; score ties use candidate ID; missing/incompatible critic
  artifact fails freeze; same PCM collapses with all condition mappings retained.

- [ ] **Step 2: Write failing repeat tests.** Assert exactly 6/3/3 controls,
  2/2/2 and 1/1/1 condition balance, at most one per pair, deterministic seed
  order, collapsed-condition assignment priority, distinct blind ID/same PCM,
  and terminal failure when quotas cannot be filled.

- [ ] **Step 3: Run and observe missing modules.**

Run: `node --test tools/transition-pilot-v2/test/schema.test.js tools/transition-pilot-v2/test/selection.test.js`

- [ ] **Step 4: Implement canonical built-in-only Node modules.** Unknown fields,
  unsafe paths, private metadata, duplicate ordinary PCM, invalid split counts,
  and post-freeze mutations are rejected.

- [ ] **Step 5: Run tests and commit.**

```powershell
node --test tools/transition-pilot-v2/test/schema.test.js tools/transition-pilot-v2/test/selection.test.js
git add tools/transition-pilot-v2
git commit -m "feat(pilot-v2): validate blind condition selection"
```

### Task 28: Build atomic blind-set freezing

**Files:**
- Create: `tools/transition-pilot-v2/lib/freeze.js`
- Create: `tools/transition-pilot-v2/bin/pilot-v2.js`
- Create: `tools/transition-pilot-v2/test/freeze.test.js`
- Modify: `tools/transition-pilot-v2/README.md`

**Interfaces:**
- Produces CLI `verify-inputs`, `build-private`, `build-blind --dry-run`,
  `freeze-blind`, and `verify-frozen`; creates a hash-linked private answer map
  and sanitized evaluator tree outside Git.
- Depends on: Task 27 and M4 render records.

- [ ] **Step 1: Write a failing temporary-tree freeze test.** Verify source,
  feature, pair, candidate-set, plan, environment, render, PCM, selector/model,
  and presentation hashes before copy; assert maximum 90 ordinary presentations,
  exactly 12 controls, deterministic order, metadata-free FLAC, and immutable
  manifest/template bytes.

- [ ] **Step 2: Add adversarial tests.** Reject a stale hash, condition leak,
  split leak, accidental duplicate presentation, path traversal, symlink escape,
  private name, changed frozen file, and attempt to freeze inside Git.

- [ ] **Step 3: Run and observe missing freeze behavior.**

Run: `node --test tools/transition-pilot-v2/test/freeze.test.js`

- [ ] **Step 4: Implement staging-directory assembly followed by verification and
  one atomic rename.** Never expose the private condition map through the blind
  server root. Read-only flags are defense in depth; hashes are authoritative.

- [ ] **Step 5: Run focused tests and commit.**

```powershell
node --test tools/transition-pilot-v2/test/freeze.test.js
git add tools/transition-pilot-v2
git commit -m "feat(pilot-v2): freeze blinded transition sets"
```

### Task 29: Implement explicit ordered tie groups and ratings state

**Files:**
- Create: `tools/transition-pilot-v2/evaluator/core.js`
- Create: `tools/transition-pilot-v2/evaluator/controller.js`
- Create: `tools/transition-pilot-v2/test/evaluator-core.test.js`
- Create: `tools/transition-pilot-v2/test/evaluator-controller.test.js`

**Interfaces:**
- Produces: `createRatingsState`, `moveSampleToGroup`, `mergeTieGroups`,
  `splitTieGroup`, `validateDraft`, `validateCompleted`, `completeExport`, and
  `createController`.
- Depends on: Task 27; may run parallel with Tasks 30-31 and private annotation.

- [ ] **Step 1: Write failing tie-model tests.** Manipulate explicit arrays of
  sample IDs ordered best-to-worst; assert every sample appears exactly once,
  no empty group, direct merge/split/reorder, and exact round-trip without numeric
  rank inference.

- [ ] **Step 2: Write failing rating-state tests.** Require `good/meh/bad`,
  smoothness 1-5, intent 1-5, confidence 1-5, bounded flags, optional comment,
  deliberate-control identity binding, manifest/template hashes, autosave merge,
  and first-incomplete resume.

- [ ] **Step 3: Run and observe missing evaluator state.**

Run: `node --test tools/transition-pilot-v2/test/evaluator-core.test.js tools/transition-pilot-v2/test/evaluator-controller.test.js`

- [ ] **Step 4: Implement pure state/validation modules.** Condition/template,
  split, source identity, and duplicate-control status are never included in the
  browser-visible model.

- [ ] **Step 5: Run tests and commit.**

```powershell
node --test tools/transition-pilot-v2/test/evaluator-core.test.js tools/transition-pilot-v2/test/evaluator-controller.test.js
git add tools/transition-pilot-v2
git commit -m "feat(pilot-v2): add explicit tie-group ratings"
```

### Task 30: Implement local server, hash verification, and fast audio switching

**Files:**
- Create: `tools/transition-pilot-v2/evaluator/server.js`
- Create: `tools/transition-pilot-v2/evaluator/browser.js`
- Create: `tools/transition-pilot-v2/test/evaluator-server.test.js`
- Create: `tools/transition-pilot-v2/test/evaluator-ui.test.js`

**Interfaces:**
- Produces localhost-only allowlisted server, bounded byte-range audio responses,
  browser manifest/hash bootstrap, and an `AudioSwitcher` that preloads current
  pair samples and preserves matched play position on condition switch.
- Depends on: Tasks 27-29; server work may begin in parallel with UI state work
  if the browser-visible manifest schema is already frozen.

- [ ] **Step 1: Write failing server security tests.** Assert bind address
  `127.0.0.1`, explicit asset allowlist, no directory listing, path traversal/
  private-map denial, security headers, bounded byte ranges, and manifest/audio
  SHA-256 verification before evaluation starts.

- [ ] **Step 2: Write failing switching tests with fake audio elements.** Assert
  A/B/C switch latency does not reset to zero, prior audio pauses, only one sample
  plays, all pair samples preload, restart is explicit, and playback errors remain
  visible without corrupting ratings.

- [ ] **Step 3: Run and observe missing server/switcher APIs.**

Run: `node --test tools/transition-pilot-v2/test/evaluator-server.test.js tools/transition-pilot-v2/test/evaluator-ui.test.js`

- [ ] **Step 4: Implement built-in HTTP/crypto modules and browser Web Crypto
  verification.** The server serves only evaluator assets, sanitized manifests,
  ratings template, and exact frozen audio.

- [ ] **Step 5: Run tests and commit.**

```powershell
node --experimental-global-webcrypto --test tools/transition-pilot-v2/test/evaluator-server.test.js tools/transition-pilot-v2/test/evaluator-ui.test.js
git add tools/transition-pilot-v2
git commit -m "feat(pilot-v2): serve verified local transition audio"
```

### Task 31: Build the browser UI, durable autosave, and completion export

**Files:**
- Create: `tools/transition-pilot-v2/evaluator/index.html`
- Create: `tools/transition-pilot-v2/evaluator/evaluator.css`
- Create: `tools/transition-pilot-v2/evaluator/ui.js`
- Create: `tools/transition-pilot-v2/test/evaluator-page.test.js`
- Modify: `tools/transition-pilot-v2/test/evaluator-ui.test.js`
- Modify: `tools/transition-pilot-v2/README.md`

**Interfaces:**
- Produces accessible local evaluation UI and download of a validated
  `transition-pilot-v2-ratings/1` document.
- Depends on: Tasks 29-30; CSS/page work may run parallel after IDs are frozen.

- [ ] **Step 1: Write failing DOM/page assertions.** Require one anonymous switch
  control per manifest presentation (one to three collapsed core conditions plus
  at most one deliberate repeat), explicit draggable/button-operated ordered tie groups, visible
  smoothness/intent/category/confidence controls, optional flags/comments,
  progress, saved/error status, resume, keyboard switching, and no track,
  template, condition, split, filename, duration, or control identity.

- [ ] **Step 2: Write failing durability tests.** Assert autosave after every
  edit, competing-tab merge, storage failure recovery, restart/resume identity
  key, stale pilot rejection, incomplete-export block, and completed schema/hash
  validation before Blob download.

- [ ] **Step 3: Run and observe missing page/UI behavior.**

Run: `node --experimental-global-webcrypto --test tools/transition-pilot-v2/test/evaluator-page.test.js tools/transition-pilot-v2/test/evaluator-ui.test.js`

- [ ] **Step 4: Implement the smallest UI satisfying the state contracts.** Reuse
  V1 interaction lessons but copy no private V1 manifest/audio. Tie grouping is a
  direct interaction, not three numeric rank radios.

- [ ] **Step 5: Run the complete evaluator suite and commit.**

```powershell
$pilotTests = Get-ChildItem tools/transition-pilot-v2/test -Filter *.test.js
node --experimental-global-webcrypto --test $pilotTests.FullName
git add tools/transition-pilot-v2
git commit -m "feat(pilot-v2): add resilient blind evaluator"
```

### Task 32: Render, select, and freeze the final blind Pilot V2 dataset

**Files:**
- Modify: `tools/transition-operator/src/pilot/pipeline.rs`
- Modify: `tools/transition-operator/tests/pipeline.rs`
- Modify: `tools/transition-pilot-v2/test/freeze.test.js`
- Update: `docs/CODEX_STATE.md`
- Private outputs only: `C:\Users\janni\Desktop\spotify-transition-private-v2\pilot-v2-*`

**Interfaces:**
- Produces the final private candidate/render records, private condition map,
  sanitized frozen blind evaluator tree, and their SHA-256 roots.
- Depends on: M4, M5, Tasks 27-31, and a separately frozen schema-compatible
  offline critic artifact. This is the first task allowed to build the final set.

- [ ] **Step 1: Add a failing frozen-realization dry-run test.** It must require
  30 frozen pair IDs, exact split/stratum counts, one candidate set per pair,
  valid fallback, deterministic and critic selections from the same set, all
  renders passing QC, environment consistency, collapse before repeats, exact 12
  repeats, and no public identity leak.

- [ ] **Step 2: Run the dry-run against manifests only.**

Run: `node tools/transition-pilot-v2/bin/pilot-v2.js build-blind --dry-run --root C:\Users\janni\Desktop\spotify-transition-private-v2`

Expected before implementation/final inputs: nonzero exit with a stable list of
missing or invalid prerequisites and no created blind tree.

- [ ] **Step 3: Generate and render every frozen pair with the M4 CLI.** Preserve
  the complete candidate set and rejections; render fallback first; render rich
  candidates; remove only QC-rejected rich candidates; rebuild valid scoring
  inputs without changing plans or pair gain; fail the pair if fallback fails.

- [ ] **Step 4: Select three conditions and freeze.** Verify critic model/input
  schema/artifact hash, select from the exact retained set, collapse equal PCM,
  assign repeats, sanitize, freeze hashes, set read-only, and keep answer map
  outside the served tree. No ratings are collected in this task.

- [ ] **Step 5: Verify the complete frozen artifact twice.**

```powershell
node tools/transition-pilot-v2/bin/pilot-v2.js verify-frozen --root C:\Users\janni\Desktop\spotify-transition-private-v2
cargo run --manifest-path tools/transition-operator/Cargo.toml -- verify-environment --root C:\Users\janni\Desktop\spotify-transition-private-v2
git status --short
```

- [ ] **Step 6: Commit only code/tests/state, never private artifacts.**

```powershell
git add tools/transition-operator tools/transition-pilot-v2 docs/CODEX_STATE.md
git diff --cached --check
git commit -m "test(pilot-v2): verify frozen blind transition set"
```

**M6 gate:** The browser evaluator passes all tests against the frozen sanitized
manifest, every served audio hash verifies, condition identity remains sealed,
and the private answer map is inaccessible to the server. Record only aggregate
counts and root hashes in Git.

## M7 — Completed Pilot V2 analysis

### Task 33: Freeze completed ratings before unblinding

**Files:**
- Create: `tools/transition-pilot-v2/lib/unblind.js`
- Create: `tools/transition-pilot-v2/test/unblind.test.js`
- Modify: `tools/transition-pilot-v2/bin/pilot-v2.js`
- Private outputs only: completed/frozen ratings and analysis workspace.

**Interfaces:**
- Produces CLI `freeze-ratings` and `unblind`; `unblind` accepts only the exact
  immutable ratings hash plus private answer-map hash.
- Depends on: M6 and human completion of every rating.

- [ ] **Step 1: Write failing freeze/unblind tests.** Reject incomplete ratings,
  stale manifest/template hashes, changed presentation order, malformed tie
  groups, missing controls, duplicate sample IDs, timestamps outside the rating
  session, writable/mismatched frozen copies, and unblinding before freeze.

- [ ] **Step 2: Run and observe missing commands.**

Run: `node --test tools/transition-pilot-v2/test/unblind.test.js`

- [ ] **Step 3: Implement validate→hash→copy→rehash→read-only freeze and a separate
  unblind command.** The original download remains untouched; unblinded output is
  a new private hash-linked artifact.

- [ ] **Step 4: Run tests, then freeze actual ratings only after the human has
  completed the blind evaluation.**

Run: `node --test tools/transition-pilot-v2/test/unblind.test.js`

- [ ] **Step 5: Commit tooling only.**

```powershell
git add tools/transition-pilot-v2
git commit -m "feat(pilot-v2): freeze ratings before unblinding"
```

### Task 34: Implement repeat-aware, tie-aware Pilot V2 analysis

**Files:**
- Create: `tools/transition-pilot-v2/lib/analysis.js`
- Create: `tools/transition-pilot-v2/test/analysis.test.js`
- Create: `docs/PILOT_V2_ANALYSIS_PROTOCOL.md`
- Modify: `tools/transition-pilot-v2/bin/pilot-v2.js`

**Interfaces:**
- Produces CLI `analyze` and a private machine-readable report plus a sanitized
  Markdown result; computes repeat noise, paired condition results, and failure
  aggregation without changing frozen data.
- Depends on: Task 33.

- [ ] **Step 1: Write failing statistical fixture tests.** Cover exact repeat
  tie-group agreement, pairwise order agreement, smoothness/intent/category
  deltas, condition wins/losses/ties, confidence filtering reported as sensitivity
  only, an exact two-sided sign test over non-tied pairwise preferences, and an
  exact sign-flip permutation distribution for mean paired smoothness, intent,
  and ordinal category deltas. Require split-stratified summaries and issue/comment
  aggregation; exhaustive enumeration is bounded at `2^18` development pairs.

- [ ] **Step 2: Add leakage tests.** Development summaries may tune one
  predeclared decision; validation is evaluated once; test remains sealed until
  that decision is frozen. Analysis must reject a report whose generator,
  candidate, renderer, selector, blind, ratings, or answer-map hashes mismatch.

- [ ] **Step 3: Run and observe missing analysis functions.**

Run: `node --test tools/transition-pilot-v2/test/analysis.test.js`

- [ ] **Step 4: Implement deterministic exact calculations.** Report smoothness
  and intent separately, always place held-out effects beside the measured repeat
  noise floor, retain ties, label one-rater limitations, and make no runtime claim.

- [ ] **Step 5: Run analysis tests and commit.**

```powershell
node --test tools/transition-pilot-v2/test/analysis.test.js
git add tools/transition-pilot-v2 docs/PILOT_V2_ANALYSIS_PROTOCOL.md
git commit -m "feat(pilot-v2): analyze repeat-aware preferences"
```

### Task 35: Produce the final evidence report and runtime go/no-go gate

**Files:**
- Create after ratings: `docs/PRIVATE_PILOT_V2_BLIND_ANALYSIS.md`
- Update: `docs/CODEX_STATE.md`

**Interfaces:**
- Produces a sanitized decision record referencing private hashes and aggregate
  results, with no audio, filenames, titles, artists, albums, or answer-key rows.
- Depends on: Task 34 and one predeclared validation decision followed by sealed
  test evaluation.

- [ ] **Step 1: Write a report-completeness assertion in
  `tools/transition-pilot-v2/test/analysis.test.js`.** Require cohort/splits,
  operator exposure, QC attrition, duplicate-control noise, condition comparisons,
  separate smoothness/intent, held-out results, qualitative failures, provenance,
  limitations, and exactly one explicit gate decision.

- [ ] **Step 2: Run and observe failure while the report is absent.**

Run: `node --test --test-name-pattern=report_ tools/transition-pilot-v2/test/analysis.test.js`

- [ ] **Step 3: Generate the private analysis, independently recompute key
  aggregates, and write the sanitized report.** The allowed decisions are:
  proceed to a separate RPI renderer design, revise offline vocabulary and repeat
  Pilot V2, or stop learned/operator integration. No decision changes playback.

- [ ] **Step 4: Run full Phase 1 verification.**

```powershell
cargo test --manifest-path tools/transition-operator/Cargo.toml
$goldenTests = Get-ChildItem tools/transition-operator/tests/node -Filter *.test.js
$pilotTests = Get-ChildItem tools/transition-pilot-v2/test -Filter *.test.js
node --experimental-global-webcrypto --test $goldenTests.FullName
node --experimental-global-webcrypto --test $pilotTests.FullName
git diff --check
git status --short
```

Also rehash all 66 original MP3 files against the private inventory, verify every
frozen artifact/hash chain, verify no audio/private manifest is tracked, and
confirm `git diff --name-only 25da884a0c2e648d9cf86fc938d3efade827f389..HEAD -- playback connect` is empty.

- [ ] **Step 5: Commit the sanitized report and state only.**

```powershell
git add docs/PRIVATE_PILOT_V2_BLIND_ANALYSIS.md docs/CODEX_STATE.md
git diff --cached --check
git commit -m "docs: record private transition pilot v2 findings"
```

**M7 gate:** Human ratings are immutable before unblinding; analysis accounts for
repeat noise and ties; validation/test discipline is auditable; the report makes
an evidence-based decision without implementing runtime behavior.

## Milestone review checklist

At every milestone:

1. Run the milestone's complete commands from a clean worktree.
2. Inspect `git status --short`, `git diff --stat`, `git diff --check`, and the
   complete staged file set before each commit.
3. Confirm schemas/fixtures changed only with an explicit version or reviewed
   golden update.
4. Confirm tests first failed for the intended missing behavior, then passed.
5. Confirm no private path/name/audio or generated render entered Git.
6. Update `docs/CODEX_STATE.md` with verified hashes, commands, failures, remote
   power ownership, and exactly one next action.
7. Stop at any specified human/evidence gate instead of filling missing facts
   with defaults.

## Final acceptance

Phase 1 is complete only when M1-M7 pass independently, the 30-pair private pilot
has completed blinded ratings and repeat-aware held-out analysis, and the final
report records a go/no-go decision. Completion does not authorize RPI lowering,
ML runtime integration, Pilot-driven playback changes, or any modification to
current playback ownership.

This plan itself requires explicit human approval before Task 1. Approval of the
plan authorizes implementation under one of the Superpowers execution workflows;
it does not waive any milestone or later human-rating gate.
