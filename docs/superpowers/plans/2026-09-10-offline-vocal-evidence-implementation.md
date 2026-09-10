# Offline Vocal Evidence Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a validated, CPU-only offline vocal-evidence extractor that supplies honest occupancy and pair-collision measurements to a new FeatureSnapshot version, then rerun the controlled corpus audit and render the second private listening set if coverage permits.

**Architecture:** Canonical PCM is analyzed by a frozen separator outside playback. A versioned 10 ms trace stores calibrated vocal evidence and positive/negative decisions. Rust aggregates that trace into exact window occupancy, collision occupancy, collision spans, and collision-local source strengths; M2 alone applies the existing thresholds and chooses templates and duck targets.

**Tech Stack:** Rust 2021, serde/canonical JSON/SHA-256 in `transition-operator`; isolated Python 3 CPU environment with pinned Spleeter/TensorFlow for feasibility; pinned FFmpeg/M3 for private rendering.

**Spec:** `docs/superpowers/specs/2026-09-08-transition-operator-design.md`, as clarified by the approved 2026-09-10 vocal-evidence decisions and Task 1 below.

## Global Constraints

- Do not change the M2 thresholds `200_000`, `300_000`, or `700_000` ppm.
- `vocal_activity_ppm` is occupancy of positive-evidence 441-frame hops in the exact FeatureWindow.
- Missing vocal evidence remains absent and cannot establish vocal freedom or exclude a competing duck modality.
- `outgoing_vocal_sustained` remains `vocal_activity_ppm > 700_000` and means high occupancy, not continuity.
- Pair vocal collision is vocal-vocal evidence; pair transient collision is transient-transient evidence.
- M2 owns classification, applicability, duck target choice, and candidate generation.
- OperatorPlan operations, M3 DSP, energy-ramp semantics, scoring, playback/runtime, RPI, critic, Pilot V2, and M5/M6 remain untouched.
- All separator work is CPU-only. Private audio, names, annotations, stems, traces, and manifests stay outside Git.

---

### Task 1: Freeze collision semantics and the FeatureSnapshot v3 contract

**Files:**
- Modify: `docs/superpowers/specs/2026-09-08-transition-operator-design.md`
- Create: `tools/transition-operator/schemas/feature-snapshot-v3.schema.json`
- Modify: `tools/transition-operator/schemas/operator-plan-v1.schema.json`
- Test: `tools/transition-operator/tests/features.rs`
- Test: `tools/transition-operator/tests/templates.rs`

**Interfaces:**
- Produces: normative two-second pair interval `[-110_250, -22_050)`; vocal collision occupancy; longest simultaneous-positive island; collision-local evidence strength; strict known-modality requirement.
- Produces v3 pair fields: modality-specific `*_collision_start_frame`, `*_collision_end_frame`, `outgoing_*_collision_strength_ppm`, and `incoming_*_collision_strength_ppm`.

- [ ] **Step 1: Record the adversarial metric table in the specification**

Use a two-second interval and record literal outcomes for IoU, intersection/minimum, and intersection/interval occupancy. The cases must include equal 1.2 s phrases with 0.6 s overlap (`333_333`, `500_000`, `300_000`), a 0.4 s phrase inside 1.6 s (`250_000`, `1_000_000`, `200_000`), 0.2 s overlap between two 0.9 s phrases (`125_000`, `222_222`, `100_000`), no vocals, and one-source-only activity. Conclude that interval occupancy alone makes 300k/700k mean 0.6 s/1.4 s of simultaneous evidence in the frozen interval.

- [ ] **Step 2: Document the target-strength clarification**

State that FeatureWindow activity remains binary-hop occupancy, while duck target comparison uses mean calibrated modality evidence strength inside the longest simultaneous-positive collision island. This avoids the tautology that both sources have 100% occupancy inside their intersection.

- [ ] **Step 3: Write failing v3 schema tests**

Add tests constructing a snapshot with separate vocal/transient intervals and strengths. Assert v2 rejects the new fields, v3 accepts complete modality groups, and v3 rejects partial interval/strength groups or out-of-range ppm.

- [ ] **Step 4: Run the focused tests and verify RED**

Run: `cargo test --manifest-path tools/transition-operator/Cargo.toml --test features snapshot_v3`

Expected: failure because the v3 schema/types are absent.

- [ ] **Step 5: Add the strict v3 JSON schema and plan reference**

Copy the settled v2 structure, change the schema identity to `transition-feature-snapshot/3`, replace shared collision boundaries with modality-specific boundaries, add the four modality-specific strength fields, and allow an OperatorPlan/v1 feature reference to name v3 without changing operations or DSP.

- [ ] **Step 6: Re-run focused tests and verify GREEN**

Run the Task 1 focused tests and `cargo test --manifest-path tools/transition-operator/Cargo.toml --test validation`.

- [ ] **Step 7: Commit the normative contract**

Commit only the specification, schemas, and their contract tests with message `spec: define offline vocal collision evidence`.

### Task 2: Implement the model-independent vocal trace by TDD

**Files:**
- Create: `tools/transition-operator/src/features/vocal.rs`
- Modify: `tools/transition-operator/src/features/mod.rs`
- Test: `tools/transition-operator/tests/vocal_trace.rs`

**Interfaces:**
- Produces: `VocalTraceV1::finalize(body) -> Result<VocalTraceV1>`.
- Produces: `vocal_activity_ppm(trace, start_frame, end_frame) -> Result<i64>`.
- Produces: `measure_vocal_collision(outgoing, incoming, outgoing_cue_frame, incoming_cue_frame, incoming_rate_ppm) -> Result<VocalCollisionMeasurements>`.
- `VocalCollisionMeasurements` contains collision occupancy, optional longest-island span/start/end, and optional outgoing/incoming mean evidence strengths.

- [ ] **Step 1: Write RED tests for trace identity and validation**

Use literal ten-hop traces. Assert schema/version, 441-frame hop, source PCM binding, evidence range, active/threshold consistency, exact hop count, canonical hash stability, and hash sensitivity to every semantic input.

- [ ] **Step 2: Verify RED**

Run: `cargo test --manifest-path tools/transition-operator/Cargo.toml --test vocal_trace trace_identity`

- [ ] **Step 3: Implement the minimal trace representation**

Represent complete-source fixed-grid evidence as a source-bound body containing analysis identity, positive-evidence threshold, source length, 441-frame hop, per-hop evidence ppm, per-hop active flags, and a domain-separated trace SHA-256. Reject partial or malformed bodies.

- [ ] **Step 4: Verify GREEN**

Run the trace-identity tests.

- [ ] **Step 5: Write RED activity aggregation boundary tests**

Assert zero, full, exactly 2/10=`200_000`, 3/10=`300_000`, 7/10=`700_000`, and one-hop either side. Include windows clipped at frame zero/source end and windows whose edges are not hop-aligned; a hop belongs to a window when its center lies in the clipped half-open range.

- [ ] **Step 6: Implement activity aggregation and verify GREEN**

Use integer round-nearest ties-away and reject a clipped interval containing no valid hop centers.

- [ ] **Step 7: Write RED collision tests**

On the 200-hop canonical interval assert empty union, one active source, exact 60-hop=`300_000`, 140-hop=`700_000`, one-hop sides, negative transition frames, unequal incoming-rate mapping, multiple islands, earliest longest-island tie resolution, exactly-two-beat and over-two-beat spans, and evidence-strength means derived from literal values.

- [ ] **Step 8: Implement collision alignment and verify GREEN**

Count simultaneous-positive timeline hops over all 200 canonical hops. Divide by 200, not union size. Map outgoing by cue plus timeline and incoming by cue plus round-nearest-away scaled timeline. Select the earliest longest contiguous collision island and average each source's evidence ppm across that island.

- [ ] **Step 9: Write and pass missing-evidence tests**

At the integration boundary, `Option<&VocalTraceV1>` must return absent measurements when either trace is absent; no valid zero may be synthesized.

- [ ] **Step 10: Commit the trace layer**

Commit the new module and tests with message `feat: add deterministic vocal evidence traces`.

### Task 3: Migrate FeatureSnapshot and M2 to v3 by TDD

**Files:**
- Modify: `tools/transition-operator/src/features/snapshot.rs`
- Modify: `tools/transition-operator/src/features/extract.rs`
- Modify: `tools/transition-operator/src/generator.rs`
- Modify: `tools/transition-operator/src/templates/common.rs`
- Modify: `tools/transition-operator/src/templates/dynamics.rs`
- Modify: `tools/transition-operator/src/validation/structural.rs`
- Modify: `tools/transition-operator/tests/features.rs`
- Modify: `tools/transition-operator/tests/generator.rs`
- Modify: `tools/transition-operator/tests/templates.rs`

**Interfaces:**
- Replaces active generation input with `FeatureSnapshotV3`/`FeatureSnapshotBodyV3`.
- Extends `TemplateInputs` with modality-specific interval and collision-strength fields.
- Adds an optional vocal trace input to pair snapshot construction without giving the extractor any template knowledge.

- [ ] **Step 1: Write RED v3 snapshot projection tests**

Assert exact whole-window occupancy, preserved `>700_000` high-occupancy derivation, pair vocal measurements, modality-specific interval/strength projection, and missing vocal propagation.

- [ ] **Step 2: Implement minimal v3 structs, validation, hashing, parsing, and projection**

Use a new domain hash `transition-feature-snapshot/3\0`. Require complete interval groups when a collision island exists and forbid interval/strength fields without a positive span.

- [ ] **Step 3: Write RED duck fail-closed and target tests**

Add adversarial cases proving a localized transient collision is inapplicable when vocal evidence is unknown; localized vocal/transient paths select the greater collision-local strength; exact ties select outgoing; whole-window activity cannot change the target.

- [ ] **Step 4: Update M2 and verify GREEN**

Require known vocal and transient collision values before duck applicability. Select modality-specific fields after M2 identifies the sole localized modality. Keep all applicability thresholds, recipes, gains, attacks, releases, and scores unchanged.

- [ ] **Step 5: Extend transient measurements**

Preserve the existing transient IoU value. Add its own longest-island interval and mean outgoing/incoming onset strength. Do not reuse or overwrite vocal interval fields.

- [ ] **Step 6: Update snapshot/generator golden fixtures**

Regenerate only identities affected by the explicit snapshot-version/reference change. Verify rendered operation semantics are unchanged.

- [ ] **Step 7: Run all standalone tests**

Run: `cargo test --manifest-path tools/transition-operator/Cargo.toml`.

- [ ] **Step 8: Commit v3/M2 integration**

Commit with message `feat: integrate collision-local FeatureSnapshot v3 evidence`.

### Task 4: Run the private CPU separator feasibility gate

**Files:**
- Private create: `C:\Users\janni\Desktop\spotify-transition-vocal-evidence-20260910\`
- Later create after passing: `tools/vocal-evidence/README.md`
- Later create after passing: `tools/vocal-evidence/requirements-lock.txt`
- Later create after passing: `tools/vocal-evidence/extract_trace.py`
- Later create after passing: `tools/vocal-evidence/model-manifest.json`
- Test after passing: `tools/vocal-evidence/tests/test_extract_trace.py`

**Interfaces:**
- Consumes canonical s16le stereo 44.1 kHz PCM and a pinned local model cache.
- Produces canonical `vocal-evidence-trace/1` JSON accepted by Task 2.

- [ ] **Step 1: Build a private CPU-only feasibility environment**

Create the environment outside Git. Pin Python/runtime/dependencies, disable GPU visibility, set deterministic CPU thread counts, and pin Spleeter 2-stem model/config hashes. Normal extraction must use local files and never resolve `latest` or download automatically.

- [ ] **Step 2: Select three private representative tracks**

Choose one clear singing track, one dense rap/spoken-style track, and one difficult transient-heavy instrumental track. Keep names and paths only in the private manifest.

- [ ] **Step 3: Benchmark the one-track bounded pipeline twice**

Record wall/audio factor, peak RSS, trace hashes, source PCM hashes, versions, thread settings, and qualitative leakage/failure notes. Retain no stems after inspection.

- [ ] **Step 4: Apply the approved rejection gate**

Reject Spleeter if peak RSS materially exceeds 2 GiB, CPU cost is operationally unreasonable, the environment is fragile, traces are not repeatable, or obvious rap/instrumental failures make validation implausible. If rejected, repeat Steps 1-3 with CPU-only HTDemucs before seeking another architecture.

- [ ] **Step 5: Only after passing, write RED CLI tests**

Test a synthetic canonical PCM fixture, manifest/hash checking, no-network operation, CPU-only configuration, trace schema, and repeatable bytes while replacing only the external separator inference call with a deterministic fixture trace.

- [ ] **Step 6: Commit the isolated offline tool after tests pass**

Commit generic code, locks, hashes, and license metadata only with message `feat: add frozen offline vocal evidence extractor`.

### Task 5: Calibrate and validate privately

**Files:**
- Private modify: `C:\Users\janni\Desktop\spotify-transition-vocal-evidence-20260910\`
- Create after pass: `docs/TRANSITION_VOCAL_EVIDENCE_VALIDATION.md`

**Interfaces:**
- Produces frozen positive-evidence and temporal-filter constants plus aggregate, anonymized validation evidence.

- [ ] **Step 1: Create track-disjoint private calibration/holdout annotations**

Annotate audible human-vocal intervals for instrumental, transient-heavy instrumental, sparse, sustained, singing, dense rap, spoken/rap-like, quiet/backing, and near-cue onset/offset material. Separator output may aid display but is not a label.

- [ ] **Step 2: Freeze calibration without M2 coverage**

Choose the absolute/relative stem evidence threshold and deterministic temporal filtering from calibration only. Record the frozen constants and calibration trace hashes before generating holdout results.

- [ ] **Step 3: Evaluate the untouched holdout**

Measure overall and singing/rap/spoken frame recall, instrumental false-active duration, onset/offset errors, window-activity error, none/localized/sustained collision agreement, and repeated CPU trace hashes.

- [ ] **Step 4: Apply the predeclared quality gate**

Require overall recall >=95%, each singing/rap/spoken recall >=90%, instrumental false-active duration <=5%, no annotated sustained window at or below 200k, median boundary error <=100 ms, p95 <=250 ms, activity MAE <=50k ppm, and collision-class agreement >=95% outside annotation uncertainty. Stop under end condition D if it fails.

- [ ] **Step 5: Commit only anonymized aggregate validation**

Document method, frozen hashes/constants, counts, aggregate metrics, determinism, CPU/RSS, and limitations without audio, filenames, annotations, traces, or stems. Commit with message `docs: validate offline vocal evidence extraction`.

### Task 6: Connect validated traces and rerun the controlled audit

**Files:**
- Modify: `tools/transition-operator/src/features/extract.rs`
- Modify: `tools/transition-operator/src/bin/transition-operator.rs`
- Modify: `tools/transition-operator/README.md`
- Test: `tools/transition-operator/tests/features.rs`
- Private modify: `C:\Users\janni\Desktop\spotify-transition-listening-demo-v2-20260910\`

**Interfaces:**
- Adds an offline-only trace import path bound to source PCM/model/algorithm hashes.
- Produces the same audit report shape with v2-before/v3-after counts and rejection tallies.

- [ ] **Step 1: Write RED end-to-end synthetic import tests**

Prove valid trace import populates v3, mismatched PCM/model/trace hashes fail, absent trace stays absent, and M2 receives exact measured values.

- [ ] **Step 2: Implement and verify the import path**

Keep separator execution outside Rust generation; Rust consumes only a validated trace artifact. Run focused feature/generator/template tests.

- [ ] **Step 3: Extract private traces sequentially**

Process the authorized 66 tracks CPU-only with bounded memory. Preserve traces and private mappings only outside Git.

- [ ] **Step 4: Rerun all 4,290 directed pairs without recalibration**

Record old/new family applicable counts, fractions, dominant rejection predicates, distinct emitted candidate counts, failures, wall time, and peak RSS.

- [ ] **Step 5: Commit generic integration and durable anonymized state**

Commit with message `feat: consume validated vocal evidence in M2` after standalone verification.

### Task 7: Render the second listening set when coverage permits

**Files:**
- Private create/modify: `C:\Users\janni\Desktop\spotify-transition-listening-demo-v2-20260910\manifest-private.json`
- Private create: `C:\Users\janni\Desktop\spotify-transition-listening-demo-v2-20260910\unblinding.txt`
- Private create: `C:\Users\janni\Desktop\spotify-transition-listening-demo-v2-20260910\ratings-template.txt`
- Private create: blind `pair-NN-sample-NN.flac` files

**Interfaces:**
- Consumes v3 snapshots and complete real M2 candidate sets.
- Produces 4-8 deterministically selected pairs and at most one deterministic representative per family/pair.

- [ ] **Step 1: Select pairs before listening**

Maximize honest family coverage with deterministic feature/M2 ordering, preferring track-disjointness. Do not fabricate a zero-coverage family.

- [ ] **Step 2: Preserve complete candidate sets and choose representatives**

Use the predeclared lowest semantic candidate ID per applicable family. Include safe control where useful; avoid parameter-neighbor renders.

- [ ] **Step 3: Render through OperatorPlan/M3/QC/FLAC**

Validate every plan, render with the pinned environment, require M3 QC, decode every FLAC, and verify container/PCM hashes. Perform representative repeat renders and assert identical outputs. No candidate-specific loudness repair is allowed.

- [ ] **Step 4: Write private evaluator aids**

Keep family/template identities out of blind filenames and ratings. Include the approved eight rating fields per sample.

### Task 8: Final verification and state

**Files:**
- Modify: `docs/CODEX_STATE.md`

- [ ] **Step 1: Run standalone formatting, lint, and tests**

Run `cargo fmt --manifest-path tools/transition-operator/Cargo.toml --check`, all-target Clippy with `-D warnings`, and the complete standalone test suite.

- [ ] **Step 2: Run repository boundary verification**

Run root `cargo fmt --check`, `cargo check --workspace`, playback/connect tests, `git diff --check`, private-name scans, tracked-audio scans, and runtime/RPI diff inspection.

- [ ] **Step 3: Verify private artifacts**

Recount audit rows, hashes, selected-pair mappings, FLAC decode/QC, deterministic repeats, and peak memory evidence.

- [ ] **Step 4: Update durable state and commit**

Replace obsolete objective/next-action state with verified commits, aggregate results, artifact locations, limitations, and exactly one next action: owner listening/rating.

- [ ] **Step 5: Inspect final repository state**

Require clean `git status --short` and stop before critic training or any later milestone.
