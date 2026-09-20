# Spotify Mixer Transition Compatibility Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Resolve authoritative Spotify Mixer A-to-B edges through saved, backend, local Auto, NONE, and deterministic fallback sources, materialize every evidence-backed style semantic, and schedule only DSP the renderer can apply safely.

**Architecture:** `spotify_mix` owns a typed source/outcome resolver and edge validation; a focused `spotify_mix_style` module expands preset/style IDs and preserves normalized directional controls without guessing physical EQ/filter/FX units. `spirc` supplies current context/session/edge ownership and performs asynchronous hydration/local-Auto orchestration, while playback receives only validated `TransitionPlan`s and retains decoder/promotion generation ownership.

**Tech Stack:** Rust, protobuf, existing librespot Connect/playback state machines, fixture-backed Spotify Automix evidence, Cargo tests.

**Spec:** `docs/SPOTIFY_LIVE_RECIPE_PROTOCOL.md` and `docs/SPOTIFY_STYLE_CONTRACT.md`

## Global Constraints

- Connect remains authoritative for what plays; transition inputs only control how authoritative A becomes authoritative B.
- Saved recipe precedes enabled backend recipe, then local Auto, then the existing deterministic fallback.
- Known preset 0 (`NONE`) is terminal; unknown or unusable presets continue fallback.
- Preserve absent versus explicit protobuf values and canonical versus playable identity.
- Reject stale session/context/row/edge results without changing queue ownership.
- Clamp effective bars to 2 through 32; use positive supplied BPM, else derive `bars * 240 / outgoing_duration_seconds`, else 120; multiply each side by its item speed once.
- Do not render physical EQ/filter/FX values or jogwheel/looping behavior that the protocol evidence leaves unknown.
- Never run `cargo update`.

## Review Focus

- A syntactically valid recipe for a different row/context must fall through without replacing the active preload.
- Preset 0 with style overrides must remain terminal NONE.
- Unknown preset IDs and unsupported nonzero DSP families must be observable fallback, not silent NONE or partial Spotify rendering.
- A relinked playable URI mismatch must reject when comparable playable identity is present while canonical IDs remain authoritative.
- A preview/dealer payload must never enter the live resolver without an authoritative edge binding.

---

### Task 1: Typed recipe, provenance, and edge resolver

**Files:**
- Modify: `connect/src/spotify_mix.rs`
- Test: `connect/src/spotify_mix.rs`

**Interfaces:**
- Consumes: `ProvidedTrack`, decoded `Transition`, current context/session/edge token, optional saved/backend candidates.
- Produces: `SpotifyTransitionSource`, `SpotifyTransitionProvenance`, `SpotifyTransitionEdge`, `SpotifyTransitionResolution`, and a candidate decision that distinguishes selected, terminal NONE, retry-next-source, and deterministic fallback.

- [ ] **Step 1: Write failing resolver tests**

Add tests that assert saved beats backend/local, valid backend beats local when enabled, backend absence/failure requests local, preset 0 is terminal even with overrides, unknown preset requests fallback, canonical/playable mismatch rejects, stale ownership rejects, and a preview-tagged candidate is ineligible for live selection.

- [ ] **Step 2: Run focused tests and confirm RED**

Run: `cargo test -p librespot-connect spotify_mix::tests::resolver_ -- --nocapture`

Expected: compile failure for the missing resolver types/functions.

- [ ] **Step 3: Implement the typed resolver**

Decode recipes without collapsing optional fields, validate canonical/row/playable/item-speed identity against `SpotifyTransitionEdge`, classify preset 0 separately from unknown IDs, and return typed rejection/fallback reasons. Keep plan adaptation outside source selection.

- [ ] **Step 4: Run focused tests and confirm GREEN**

Run: `cargo test -p librespot-connect spotify_mix::tests::resolver_ -- --nocapture`

Expected: all new resolver tests pass.

### Task 2: Hydration envelope hardening

**Files:**
- Modify: `connect/src/spotify_mix_hydration.rs`
- Test: `connect/src/spotify_mix_hydration.rs`

**Interfaces:**
- Consumes: extension-244 response for an exact `TransitionHydrationKey`.
- Produces: a hydrated saved candidate whose status, Any type, requested revision, playlist, row bytes, canonical pair, and ownership key are validated.

- [ ] **Step 1: Write failing hydration tests**

Add cases for non-success response status, wrong Any type URL, conflicting latest/requested revision, and stale key/generation rejection while retaining the existing valid extension-244 fixture.

- [ ] **Step 2: Run focused tests and confirm RED**

Run: `cargo test -p librespot-connect spotify_mix_hydration::tests -- --nocapture`

Expected: at least the new status/type tests fail.

- [ ] **Step 3: Harden decoding and validation**

Validate response status and `type.googleapis.com/spotify.playlistmixing.mixtransition.TransitionData` before parsing Any bytes; keep latest URI informational and require the returned requested URI to match the hydration key.

- [ ] **Step 4: Run focused tests and confirm GREEN**

Run: `cargo test -p librespot-connect spotify_mix_hydration::tests -- --nocapture`

Expected: all hydration tests pass.

### Task 3: Deterministic preset/style semantic resolver

**Files:**
- Create: `connect/src/spotify_mix_style.rs`
- Modify: `connect/src/lib.rs`
- Modify: `connect/src/spotify_mix.rs`
- Test: `connect/src/spotify_mix_style.rs`

**Interfaces:**
- Consumes: `Preset`, directional curve overrides, overlap bars/BPM/item speeds, and the pinned 0-through-22 preset table.
- Produces: `ResolvedSpotifyStyle` with exact style IDs, effective bars/BPM, directional normalized volume curves, preserved EQ/filter/FX controls, and explicit renderer capability failures.

- [ ] **Step 1: Write failing style tests**

Cover preset 2 expansion `(volume=7, eq=1, filter=0, fx=0)`, preset 11 as volume-only cut, bars clamping at 2/32 including 3-bar behavior, BPM supplied/derived/120 fallback, per-side item-speed multiplication, explicit style ID overrides including zero, curve-override presence, unknown preset rejection, and nonzero jogwheel/looping capability rejection.

- [ ] **Step 2: Run focused tests and confirm RED**

Run: `cargo test -p librespot-connect spotify_mix_style::tests -- --nocapture`

Expected: compile failure for the missing style resolver.

- [ ] **Step 3: Implement pinned style semantics**

Add the verified preset table, deterministic volume-style curves for IDs 0 through 11 with bar-dependent widths, normalized typed controls for EQ/filter/FX, native ID-override precedence, and presence-preserving curve overrides. Mark physical EQ/filter/FX, nonzero jogwheel/looping, and unsupported partial override forms as capability failures.

- [ ] **Step 4: Run focused tests and confirm GREEN**

Run: `cargo test -p librespot-connect spotify_mix_style::tests -- --nocapture`

Expected: all style resolver tests pass.

### Task 4: TransitionPlan adapter and live source orchestration

**Files:**
- Modify: `connect/src/spotify_mix.rs`
- Modify: `connect/src/spirc.rs`
- Modify: `playback/src/transition.rs` only if directional timing cannot be retained without a coherent extension
- Modify: `playback/src/player.rs` only for concise source/ownership/promotion diagnostics
- Test: `connect/src/spotify_mix.rs`
- Test: `connect/src/spirc.rs`
- Test: `playback/src/transition.rs` if changed

**Interfaces:**
- Consumes: typed source resolution, hydrated saved candidates, backend attribute candidates, local Auto results, active edge/session ownership, and resolved styles.
- Produces: a renderable `TransitionPlan`, terminal no-transition, or explicit next fallback action; player preload remains keyed to the authoritative incoming track.

- [ ] **Step 1: Write failing adapter/orchestration tests**

Assert saved/hydrated/backend/local ordering, backend unusable to local fallback, terminal NONE scheduling without a transition, renderable volume-only preset mapping, unsupported EQ/filter/FX whole-candidate fallback, local-Auto ownership, stale edge/session suppression, and retained incoming speed automation.

- [ ] **Step 2: Run focused tests and confirm RED**

Run: `cargo test -p librespot-connect spotify_mix spirc -- --nocapture`

Expected: new source/adapter assertions fail before orchestration changes.

- [ ] **Step 3: Integrate resolver decisions into SPIRC**

Replace implicit `Option<TransitionPlan>` precedence with typed outcomes; try inline/hydrated saved, guarded backend, then local Auto. Bind asynchronous outcomes to context, row UIDs, canonical/playable identities, session ID and an edge generation. Pass only the authoritative incoming `track_id` to `preload_with_transition`; terminal NONE preloads without a transition and suppresses local Auto for that edge.

- [ ] **Step 4: Adapt supported styles to TransitionPlan**

Map exact outgoing/incoming starts, overlap, evidence-backed volume curves and supported incoming speed events. If directional source durations or a required DSP family cannot be represented correctly, return a typed unsupported reason and continue the documented fallback rather than approximating it.

- [ ] **Step 5: Run focused tests and confirm GREEN**

Run: `cargo test -p librespot-connect spotify_mix spirc -- --nocapture`

Expected: all resolver, adapter, ownership, and orchestration tests pass.

### Task 5: Observability, state, and verification

**Files:**
- Modify: `connect/src/spotify_mix.rs`
- Modify: `connect/src/spirc.rs`
- Modify: `playback/src/player.rs`
- Modify: `docs/CODEX_STATE.md`

**Interfaces:**
- Consumes: final typed source/outcome and player transition lifecycle.
- Produces: concise logs for A/B, source/provenance, preset/styles, fallback, timing/speed, omitted DSP, ownership, and promotion; durable verified repository state.

- [ ] **Step 1: Add structured diagnostics**

Log identifiers and decisions without recipe bytes, raw payloads, credentials, or account data. Include player generation at arming/promotion using the existing runtime trace path.

- [ ] **Step 2: Run local verification**

Run: `cargo fmt --check`; `cargo check --workspace`; `cargo test -p librespot-playback`; `cargo test -p librespot-connect`; `cargo clippy -p librespot-playback --all-targets`; `cargo clippy -p librespot-connect --all-targets`; relevant tools under `tools/spotify-automix-oracle`, `tools/spotify-mixer-harness`, and `tools/runtime-diagnostics`; `git diff --check`.

Expected: all commands exit 0; report exact test counts and any documented Clippy allowance.

- [ ] **Step 3: Update durable state**

Replace the protocol-investigation next action in `docs/CODEX_STATE.md` with exact implemented sources, capability gates, tests, commit/deployment state, and one next action for RPI validation.

- [ ] **Step 4: Inspect final Git state**

Run: `git status --short`; `git diff --stat`; `git diff --check`.

Expected: only implementation, focused tests, plan, and state documentation are changed; no private evidence or unrelated artifacts are present.

- [ ] **Step 5: Commit and push the clean implementation**

Commit with a focused message after verification, then push the existing feature branch because the repository workflow and user request explicitly call for it.

- [ ] **Step 6: Deploy exact candidate and prepare bounded RPI diagnostics**

Preserve the known-good binary, build/deploy the exact commit using the established RPI workflow, record hashes/revision, and start bounded autonomous logs. Ask for one Spotify reproduction only after deployment is ready; collect and correlate logs over SSH afterward.
