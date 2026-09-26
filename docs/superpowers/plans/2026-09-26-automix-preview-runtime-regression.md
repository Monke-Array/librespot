# Automix Preview Runtime Regression Stabilization Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Restore ordinary Spotify control responsiveness and bound preview/dealer lifecycle work without removing or extending automix-preview playback.

**Architecture:** Keep SPIRC as preview authority owner and the player as token-enforcing playback owner, but make the no-preview command path mutate only cheap SPIRC state. Publish player-side authority cancellation only when an active preview was actually retired. Make dealer responder tasks terminate with their websocket transport, and bound identical duplicate waiters while preserving the existing terminal-reply contract.

**Tech Stack:** Rust 2024, Tokio unbounded channels/tasks, existing SPIRC/player preview coordinator, RPI-01 systemd runtime validation.

**Spec:** `docs/superpowers/specs/2026-09-21-spotify-automix-preview.md`

## Global Constraints

- Preserve the preview subsystem and all token, stale-work, queue-isolation, and deterministic-renderer invariants.
- Do not add preview capabilities or support new Spotify presets/styles.
- Normal commands must not await preview teardown, restoration, rendering, or dealer reply completion.
- Do not change ordinary live-transition selection, promotion, or queue advancement.
- Do not add sleeps or timeout-based race suppression.
- Keep RPI-01 on the exact pre-preview rollback until the local gate is green.

## Runtime evidence to preserve

- Candidate `5891156`, binary SHA-256 `dfe890c8…73d4`: 46 matched commands, dealer-to-SPIRC p50 0.456 ms, p95 463.946 ms, max 954.047 ms.
- Baseline `94b4a69`, binary SHA-256 `b148f34d…60b5`: 28 matched commands, p50 0.405 ms, p95 13.402 ms, max 79.969 ms.
- Candidate normal commands advanced preview authority and enqueued `SetPreviewAuthority` even with no admitted preview.
- Retained runtime history contained preview rejections at recipe resolution (`UnsupportedRenderer`, presets 1 and 17); no preview token or player start was reached.
- Retained disconnect history contained dealer/session drops during intervals with no preview signals, so pending preview replies are not the evidenced cause of those drops.

## Review Focus

- A normal command with no active preview advances SPIRC authority but emits no player preview command.
- A normal command with an active preview retires it, drains all current waiters, and publishes exactly one player cancellation before ordinary action.
- A dealer request whose websocket transport disappears cannot leave its per-request responder task alive.
- Identical duplicate preview signals cannot grow the waiter vector without a fixed bound; overflow fails only the new waiter and does not replace audio.
- Malformed/unsupported preview requests still fail before player ownership, while live transition and ordinary queue tests remain unchanged.

---

### Task 1: Remove preview work from the ordinary no-preview command path

**Files:**
- Modify: `connect/src/spirc.rs`
- Test: `connect/src/spirc.rs`

**Interfaces:**
- Consumes: `PreviewCoordinator::cancel`, `PreviewCoordinator::invalidate_authority`, current Connect session ID, and normal ownership generation.
- Produces: a focused authority-advance outcome containing the new `PreviewAuthority` and an optional retired `PreviewToken`; player publication occurs only when the retired token is present.

- [ ] **Step 1: Write failing ownership tests**

Add `normal_command_without_preview_advances_authority_without_player_update` and `normal_command_with_preview_retires_once_and_requires_player_update`. Assert literal generation changes, optional retired token, and waiter replies. The first test must fail because current code unconditionally publishes player authority.

- [ ] **Step 2: Run the focused tests and verify RED**

Run: `cargo test -p librespot-connect spirc::tests::normal_command_without_preview_advances_authority_without_player_update -- --exact`

Expected: FAIL because no no-op player-update outcome exists.

- [ ] **Step 3: Implement the minimal authority outcome**

Move generation/coordinator mutation into a small `PreviewSpircOwnership` method. Return the new authority plus `retired: Option<PreviewToken>`. In `advance_normal_ownership`, call `Player::set_preview_authority` only for `Some(retired)`; keep generation advancement and waiter drainage synchronous and allocation-bounded.

- [ ] **Step 4: Verify focused and existing SPIRC preview tests GREEN**

Run: `cargo test -p librespot-connect spirc::tests::normal_command_`

Run: `cargo test -p librespot-connect spirc::tests::preview_`

### Task 2: End dealer responder tasks when their websocket transport closes

**Files:**
- Modify: `core/src/dealer/manager.rs`
- Test: `core/src/dealer/manager.rs`

**Interfaces:**
- Consumes: the responder websocket sender and the per-request reply receiver.
- Produces: a request responder task that selects between the terminal reply and transport closure.

- [ ] **Step 1: Write the failing transport-loss test**

Add `pending_request_responder_ends_when_transport_closes`. Dispatch one real parsed request through `DealerRequestHandler`, retain the returned SPIRC-side reply sender, drop the websocket receiver, yield once, and assert the SPIRC-side sender is closed. This catches a responder task retaining the receiver after dealer loss.

- [ ] **Step 2: Run the test and verify RED**

Run: `cargo test -p librespot-core dealer::manager::tests::pending_request_responder_ends_when_transport_closes -- --exact`

Expected: FAIL because the reply receiver remains owned by the spawned task.

- [ ] **Step 3: Select reply versus transport closure**

Clone only the websocket sender needed to await `closed()`. On transport closure, drop the request reply receiver and responder without waiting for SPIRC. Preserve the existing success/failure/unanswered mapping when a reply wins.

- [ ] **Step 4: Add the existing-behavior nonblocking regression**

Add `pending_request_does_not_block_later_request_reply`: leave request A pending, reply to B, and assert B reaches the websocket receiver first. This test documents the already-correct independent-task boundary.

- [ ] **Step 5: Run dealer manager tests GREEN**

Run: `cargo test -p librespot-core dealer::manager::tests`

### Task 3: Bound duplicate preview waiters and preserve terminal drainage

**Files:**
- Modify: `connect/src/spotify_mix_preview.rs`
- Modify: `connect/src/spirc.rs`
- Test: `connect/src/spotify_mix_preview.rs`
- Test: `connect/src/spirc.rs`

**Interfaces:**
- Consumes: identical-fingerprint duplicate admission and existing `Reply` senders.
- Produces: bounded duplicate admission; overflow fails only the new waiter and leaves the active preview/token unchanged.

- [ ] **Step 1: Write the failing bounded-waiter test**

Admit one active preview, attach duplicates to the chosen fixed capacity, then submit one more identical waiter. Assert the overflow waiter receives `Reply::Failure`, the active token is unchanged, and stored waiter count does not exceed capacity.

- [ ] **Step 2: Run the test and verify RED**

Run: `cargo test -p librespot-connect spotify_mix_preview::tests::identical_duplicate_waiters_are_bounded -- --exact`

Expected: FAIL because the vector currently grows without a bound.

- [ ] **Step 3: Implement bounded duplicate admission**

Add a private fixed waiter capacity and a `PreviewAdmission` overflow outcome. Do not cancel or replace the active preview on overflow. Handle the outcome in SPIRC as an immediate failure already delivered by the coordinator, with no player command.

- [ ] **Step 4: Verify cancellation/session/shutdown drainage**

Extend existing coordinator/SPIRC tests so cancellation, session invalidation, and shutdown drain every admitted waiter exactly once. Keep stale terminal events observational only.

- [ ] **Step 5: Run focused coordinator and SPIRC tests GREEN**

Run: `cargo test -p librespot-connect spotify_mix_preview::tests`

Run: `cargo test -p librespot-connect spirc::tests`

### Task 4: Verify locally, deploy once, and validate runtime in priority order

**Files:**
- Modify: `docs/CODEX_STATE.md`
- Modify only if required for evidence: `connect/src/spirc.rs` diagnostics

**Interfaces:**
- Consumes: exact fixed commit/binary plus RPI runtime trace.
- Produces: normal-control latency evidence, bounded connection evidence, live-transition evidence, and the exact clean preview failure stage if presets remain unsupported.

- [ ] **Step 1: Run the local gate**

Run: `cargo fmt --check`

Run: `cargo check --workspace`

Run: `cargo test -p librespot-playback`

Run: `cargo test -p librespot-connect`

Run: `cargo test -p librespot-core dealer::manager::tests`

Run: `git diff --check`

- [ ] **Step 2: Update durable state with verified facts only**

Record exact A/B evidence, proven independent causes, focused/full test counts, current RPI rollback hash, and exactly one next action.

- [ ] **Step 3: Build/deploy the exact candidate on RPI-01**

Start one established persistent RPI build. Do not poll it; stop and wait for the user to report completion. Preserve the current rollback and record the fixed binary SHA-256.

- [ ] **Step 4: Validate without Preview first**

Run the same normal-control sequence, measure dealer→SPIRC latency, exercise rapid pause/play and seeks, observe a bounded idle interval, and confirm ordinary live transition behavior.

- [ ] **Step 5: Validate Preview once only after normal stability**

Trace signal→decode→resolution→token→player lifecycle. If the observed recipe remains unsupported, require immediate failure before player ownership and document `resolve_automix_preview: UnsupportedRenderer { preset_id }` as the last successful boundary.
