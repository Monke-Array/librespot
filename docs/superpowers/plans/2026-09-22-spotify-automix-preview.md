# Spotify Automix Preview Playback Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the evidenced Spotify Mixer `automix-preview` request audibly render A pre-roll, the existing deterministic Spotify transition, and B post-roll without changing ordinary Connect queue ownership.

**Architecture:** A typed dealer signal is decoded into a strict preview-only domain request, then lowered through the existing Spotify recipe materializer into the existing `TransitionPlan`. SPIRC owns request deduplication, pending dealer replies, Connect authority, and preview generations; the existing `PlayerInternal` temporarily retains normal playback and reuses its source loaders, secondary PCM worker, transition engine, and single sink under a complete preview token.

**Tech Stack:** Rust 2024 workspace, serde/serde_json dealer decoding, protobuf 3.7 proto2 generation, base64 0.22, SHA-1 semantic fingerprints, Tokio unbounded/oneshot channels, existing librespot playback decoder/secondary/transition machinery.

**Spec:** `docs/superpowers/specs/2026-09-21-spotify-automix-preview.md`

## Global Constraints

- Work from current branch `codex/m3a-live-auto-metadata` and current HEAD; never reset to an older checkpoint.
- Never run `cargo update`.
- Preview A/B ownership is temporary and separate from the authoritative Connect context, current row, next row, queue, play request, and live edge generation.
- Do not call `resolve_transition_plan_sources`, `evaluate_recipe_candidate`, backend Auto, local Auto, or normal-crossfade fallback from preview resolution.
- Reuse the existing Spotify recipe/style materializer, `TransitionPlan`, secondary PCM worker, `TransitionEngine`, player thread, and audio sink.
- Support only `start_position_ms = 3000`, `relative_start_position = true`, and absent `stop_position_ms`; reject every other timing profile.
- Known preset `NONE` is an intentional successful no-preview result. Unknown, custom, or unsupported DSP is an explicit preview failure.
- Every asynchronous preview loader, decoder block, renderer completion, player event, cancellation, and dealer waiter is keyed by the complete `PreviewToken`.
- Preview never emits ordinary `EndOfTrack`, `TrackChanged`, `PlayRequestIdChanged`, queue-visible `Playing`, or live promotion events.
- Every pending dealer reply resolves exactly once on completion, `NONE`, rejection, replacement, load/render failure, normal-command preemption, session replacement, or shutdown.
- Normal playback commands preempt preview deterministically. Restoration is allowed only while Connect session and normal ownership generation still match.
- Preserve existing live transition tests and behavior, including one queue advance after ordinary live promotion.
- Use focused red/green tests before implementation in every code task and commit each green task separately.

## File and responsibility map

- `protocol/proto/automix_preview.proto`: production proto2 envelope with observed field numbers and presence.
- `protocol/build.rs`: includes the production preview schema in generated protocol modules.
- `core/src/dealer/protocol/request.rs`: typed `SignalCommand` transport ingress; it does not interpret preview payloads.
- `core/src/mix_debug.rs`: sanitized signal diagnostics; removes the raw base64 payload trace.
- `connect/src/spotify_mix_preview.rs`: bounded decoding, presence-aware validation, semantic fingerprinting, preview recipe resolution, and the pure SPIRC-side session coordinator.
- `connect/src/spotify_mix.rs`: exposes one shared recipe materialization function used by both live and preview paths; live candidate selection remains unchanged.
- `connect/src/lib.rs`: registers the focused preview module.
- `connect/src/spirc.rs`: owns Connect authority epochs, dealer waiters, duplicate/replacement policy, player preview commands, terminal events, and command preemption.
- `playback/src/preview.rs`: public preview token/request/event vocabulary and exact wall-frame budget helper.
- `playback/src/lib.rs`: exports preview types without exposing player internals.
- `playback/src/secondary.rs`: tags secondary decoder messages/PCM with full preview ownership in addition to the existing generation.
- `playback/src/player.rs`: temporarily retains normal `PlayerState`, loads preview A/B, renders, performs preview-only B handoff/post-roll, restores or yields, and suppresses normal lifecycle events.
- `docs/CODEX_STATE.md`: records verified local and later runtime state with exactly one next action.
- `docs/SPOTIFY_LIVE_RECIPE_PROTOCOL.md`: updated only after RPI-01 proves new preview semantics.

## Specification coverage map

| Specification concern | Owning task |
| --- | --- |
| Typed signal and production protobuf | Task 1 |
| Bounds, proto2 presence, identities, exact timing, fingerprint | Task 2 |
| Shared style/materializer with `NONE` and unsupported gates | Task 3 |
| Complete playback token and frame budget | Task 4 |
| Duplicate attachment, replacement, generations, dealer waiters | Task 5 |
| Normal-state retention and token-owned A/B loading | Task 6 |
| Token-owned secondary decoder/PCM | Task 7 |
| Existing renderer, preview-only handoff, exact B post-roll | Task 8 |
| Cancellation, command preemption, authority-safe restoration | Task 9 |
| SPIRC ingress/events, terminal replies, zero queue advancement | Task 10 |
| Sanitized diagnostics, complete local gate, durable state | Task 11 |
| Audible behavior and lifecycle regression evidence | RPI-01 validation |

## Review Focus

- Oversized or syntactically valid payloads with missing proto2 fields must fail before player work; Task 2 tests both bounds and presence.
- A duplicate whose optional-field presence differs while values appear equivalent must replace rather than attach; Task 5 tests fingerprint presence bits.
- An old secondary worker can deliver after replacement; Task 7 tests that its full token is discarded without cancelling the new preview.
- A normal ownership/session change can race preview completion; Tasks 9 and 10 test that stale completion cannot restore, reply for, promote, or advance anything.
- Preview B can end before the 3000 ms post-roll; Task 8 tests deterministic failure/restoration rather than accidental ordinary EOF handling.

---

### Task 1: Type dealer signal ingress and generate the production preview envelope

**Files:**
- Create: `protocol/proto/automix_preview.proto`
- Modify: `protocol/build.rs`
- Modify: `core/src/dealer/protocol/request.rs`

**Interfaces:**
- Consumes: dealer JSON currently deserialized into `core::dealer::protocol::request::Command`.
- Produces: `Command::Signal(SignalCommand)` and generated `librespot_protocol::automix_preview::AutomixPreview`.

- [ ] **Step 1: Write failing transport tests**

Add a `#[cfg(test)] mod tests` in `core/src/dealer/protocol/request.rs` that parses representative command JSON and pins presence:

```rust
#[test]
fn automix_signal_is_typed_without_decoding_parameters() {
    let request: Request = serde_json::from_value(serde_json::json!({
        "message_id": 17,
        "sent_by_device_id": "controller",
        "command": {
            "endpoint": "signal",
            "signal_id": "automix-preview",
            "parameters": "AQID",
            "logging_params": { "command_id": "preview-1" }
        }
    })).unwrap();

    let Command::Signal(signal) = request.command else { panic!("signal must be typed") };
    assert_eq!(signal.signal_id, "automix-preview");
    assert_eq!(signal.parameters.as_deref(), Some("AQID"));
}

#[test]
fn signal_preserves_absent_parameters() {
    let request: Request = serde_json::from_value(signal_json_without_parameters()).unwrap();
    let Command::Signal(signal) = request.command else { panic!("signal must be typed") };
    assert_eq!(signal.parameters, None);
}

#[test]
fn unrelated_endpoint_remains_unknown() {
    let request: Request = serde_json::from_value(command_json("future-command")).unwrap();
    assert!(matches!(request.command, Command::Unknown(_)));
}
```

- [ ] **Step 2: Run the focused test and verify RED**

Run: `cargo test -p librespot-core dealer::protocol::request::tests::automix_signal_is_typed_without_decoding_parameters -- --exact`

Expected: compile failure because `Command::Signal` and `SignalCommand` do not exist.

- [ ] **Step 3: Add the exact proto2 schema and typed transport**

Create the schema with all 15 observed fields and no defaults:

```protobuf
syntax = "proto2";
package librespot.spotify.automix_preview;
import "automix_transition.proto";
import "spotify_auto_mix_metadata.proto";

message AutomixPreview {
  optional string track_uri_1 = 1;
  optional string track_uri_2 = 2;
  optional string automix_mode = 3;
  optional string transition_uri = 4;
  optional int64 start_position_ms = 5;
  optional bool relative_start_position = 6;
  optional spotify.automix.live.Cuepoints cuepoints = 7;
  optional int64 stop_position_ms = 8;
  optional spotify.automix.proto.Transition transition_recipe = 9;
  optional string playable_track_uri_1 = 10;
  optional string playable_track_uri_2 = 11;
  optional string context_uri = 12;
  optional string arm_id = 13;
  optional double item_speed_a = 14;
  optional double item_speed_b = 15;
}
```

Add `proto_dir.join("automix_preview.proto")` next to the other automix inputs in `protocol/build.rs`. Add this transport type and variant in `request.rs` and include `Signal` in `Display`:

```rust
#[derive(Clone, Debug, Deserialize)]
pub struct SignalCommand {
    pub signal_id: String,
    #[serde(default)]
    pub parameters: Option<String>,
    pub logging_params: LoggingParams,
}

// Insert immediately before the existing untagged Unknown fallback.
Signal(SignalCommand),
```

The core layer must leave base64/protobuf interpretation to Connect. Define `signal_json_without_parameters()` and `command_json(endpoint)` in the test module as complete `Request` JSON objects with all required `logging_params` fields.

- [ ] **Step 4: Run the complete focused transport tests and verify GREEN**

Run: `cargo test -p librespot-core dealer::protocol::request::tests`

Expected: all three new tests pass, and the protocol crate generates `automix_preview` successfully.

- [ ] **Step 5: Commit the green transport slice**

```powershell
git add protocol/proto/automix_preview.proto protocol/build.rs core/src/dealer/protocol/request.rs
git diff --cached --check
git commit -m "feat(core): type Spotify preview signal ingress"
```

### Task 2: Decode and strictly validate preview requests

**Files:**
- Create: `connect/src/spotify_mix_preview.rs`
- Modify: `connect/src/lib.rs`
- Modify: `connect/src/spotify_mix.rs`

**Interfaces:**
- Consumes: `SignalCommand`, generated `AutomixPreview`, `SpotifyTransitionRecipe`, and `SpotifyUri::from_uri`.
- Produces: `decode_automix_preview(&SignalCommand) -> Result<AutomixPreviewRequest, AutomixPreviewError>`.

- [ ] **Step 1: Write failing decoder and validation tests**

Use a test fixture builder that constructs the generated protobuf, encodes it with `base64::engine::general_purpose::STANDARD`, and uses the captured geometry `start_a=184812`, `start_b=944`, `duration=7385`, preset 10, window 3000. Pin these cases:

```rust
#[test]
fn valid_relative_preview_decodes_with_exact_positions_and_presence() {
    let request = decode_automix_preview(&signal(valid_preview_proto())).unwrap();
    assert_eq!(request.window_ms, 3_000);
    assert_eq!(request.outgoing_load_position_ms, 181_812);
    assert_eq!(request.incoming_load_position_ms, 944);
    assert!(request.fields.transition_uri);
    assert!(!request.fields.stop_position_ms);
}

#[test]
fn absent_required_field_is_not_defaulted() {
    let mut wire = valid_preview_proto();
    wire.start_position_ms = None;
    assert_eq!(decode_wire(wire).unwrap_err(), AutomixPreviewError::MissingStartPosition);
}

#[test]
fn non_evidenced_window_absolute_start_and_explicit_stop_are_rejected() {
    let mut wrong_window = valid_preview_proto();
    wrong_window.start_position_ms = Some(2_999);
    assert_eq!(decode_wire(wrong_window).unwrap_err(), AutomixPreviewError::UnsupportedWindow(2_999));

    let mut absolute = valid_preview_proto();
    absolute.relative_start_position = Some(false);
    assert_eq!(decode_wire(absolute).unwrap_err(), AutomixPreviewError::AbsoluteStartUnsupported);

    let mut stopped = valid_preview_proto();
    stopped.stop_position_ms = Some(195_000);
    assert_eq!(decode_wire(stopped).unwrap_err(), AutomixPreviewError::ExplicitStopUnsupported);
}

#[test]
fn oversized_invalid_base64_and_invalid_protobuf_are_bounded_failures() {
    assert_eq!(decode_parameters(&"A".repeat(16 * 1024 + 1)).unwrap_err(), AutomixPreviewError::OversizedParameters);
    assert_eq!(decode_parameters("not base64!").unwrap_err(), AutomixPreviewError::InvalidBase64);
    assert_eq!(decode_parameters("/w==").unwrap_err(), AutomixPreviewError::InvalidProtobuf);
}

#[test]
fn canonical_playable_and_item_speed_mismatches_are_rejected() {
    assert_eq!(decode_wire(with_recipe_canonical_a(valid_preview_proto(), TRACK_C)).unwrap_err(), AutomixPreviewError::CanonicalAMismatch);
    assert_eq!(decode_wire(with_recipe_playable_b(valid_preview_proto(), TRACK_C)).unwrap_err(), AutomixPreviewError::PlayableBMismatch);
    assert_eq!(decode_wire(with_recipe_item_speed_b(valid_preview_proto(), 1.25)).unwrap_err(), AutomixPreviewError::ItemSpeedBMismatch);
}
```

Also test missing/non-positive/non-finite item speeds, missing recipe/overlap/timing, `start_a < 3000`, malformed URI, and mode other than exact `auto`.

- [ ] **Step 2: Run the focused decoder test and verify RED**

Run: `cargo test -p librespot-connect spotify_mix_preview::tests::valid_relative_preview_decodes_with_exact_positions_and_presence -- --exact`

Expected: compile failure because the module and decoder do not exist.

- [ ] **Step 3: Implement bounded, presence-aware decoding**

Define the preview-only types; do not implement a conversion to `SpotifyTransitionCandidate`:

```rust
pub(crate) const PREVIEW_WINDOW_MS: u32 = 3_000;
const MAX_PREVIEW_PARAMETERS_LEN: usize = 16 * 1024;

#[derive(Debug, Eq, PartialEq)]
pub(crate) enum AutomixPreviewError {
    WrongSignalId,
    MissingParameters,
    EmptyParameters,
    OversizedParameters,
    InvalidBase64,
    InvalidProtobuf,
    MissingCanonicalA,
    MissingCanonicalB,
    MissingPlayableA,
    MissingPlayableB,
    InvalidTrackUri(&'static str),
    MissingMode,
    UnsupportedMode(String),
    MissingStartPosition,
    MissingRelativeFlag,
    AbsoluteStartUnsupported,
    UnsupportedWindow(i64),
    ExplicitStopUnsupported,
    MissingItemSpeed(&'static str),
    InvalidItemSpeed(&'static str),
    MissingRecipe,
    MissingRecipeOverlap,
    MissingRecipeTiming,
    InvalidRecipeDuration,
    InvalidRecipeAnalysis,
    CanonicalAMismatch,
    CanonicalBMismatch,
    PlayableAMismatch,
    PlayableBMismatch,
    ItemSpeedAMismatch,
    ItemSpeedBMismatch,
    OutgoingPrerollUnderflow,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PreviewFieldPresence {
    pub transition_uri: bool,
    pub cuepoints: bool,
    pub stop_position_ms: bool,
    pub context_uri: bool,
    pub arm_id: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PreviewFingerprint([u8; 20]);

#[derive(Clone, Debug)]
pub(crate) struct AutomixPreviewRequest {
    pub automix_mode: String,
    pub canonical_a: SpotifyUri,
    pub canonical_b: SpotifyUri,
    pub playable_a: SpotifyUri,
    pub playable_b: SpotifyUri,
    pub context_uri: Option<String>,
    pub transition_uri: Option<String>,
    pub arm_id: Option<String>,
    pub cuepoints: Option<Cuepoints>,
    pub fields: PreviewFieldPresence,
    pub relative_start_position: bool,
    pub window_ms: u32,
    pub outgoing_load_position_ms: u32,
    pub incoming_load_position_ms: u32,
    pub item_speed_a_bits: u64,
    pub item_speed_b_bits: u64,
    pub recipe: SpotifyTransitionRecipe,
    pub fingerprint: PreviewFingerprint,
}

pub(crate) fn decode_automix_preview(
    command: &SignalCommand,
) -> Result<AutomixPreviewRequest, AutomixPreviewError>;
```

Add `SpotifyTransitionRecipe::from_transition(Transition)` and read-only overlap/preset accessors needed by this module. Decode only after checking the encoded string is non-empty and no longer than 16 KiB. Validate every proto2 `Option` before calling generated getters. Require exact envelope/recipe canonical URI, playable URI, and item-speed agreement using the existing item-speed tolerance helper.

Build `PreviewFingerprint` by feeding SHA-1 a tagged, length-delimited canonical sequence containing all validated strings, numeric `to_bits()` values, cuepoint/recipe bytes, and each optional presence bit. Exclude dealer message ID and logging fields. Do not hash raw unbounded input. Add a test proving that a different dealer `message_id`/`command_id` produces the same fingerprint while changing only `context_uri` presence produces a different fingerprint.

Define the fixture helpers used above in the new module: `signal(AutomixPreview) -> SignalCommand`, `decode_wire(AutomixPreview)`, `decode_parameters(&str)`, `valid_preview_proto()`, and the three `with_recipe_*` mutators. Each mutator changes exactly one generated proto2 field so failures identify the owning check.

- [ ] **Step 4: Run all decoder tests and verify GREEN**

Run: `cargo test -p librespot-connect spotify_mix_preview::tests`

Expected: valid decode and every malformed/presence/bounds/identity test pass.

- [ ] **Step 5: Commit the green decoder slice**

```powershell
git add connect/src/lib.rs connect/src/spotify_mix.rs connect/src/spotify_mix_preview.rs
git diff --cached --check
git commit -m "feat(connect): decode Spotify automix previews"
```

### Task 3: Share recipe materialization without entering live candidate resolution

**Files:**
- Modify: `connect/src/spotify_mix.rs`
- Modify: `connect/src/spotify_mix_preview.rs`

**Interfaces:**
- Consumes: a validated `SpotifyTransitionRecipe`.
- Produces: `materialize_spotify_recipe(&SpotifyTransitionRecipe) -> Result<SpotifyRecipeMaterialization, SpotifyRecipePlanError>` and `resolve_automix_preview(AutomixPreviewRequest) -> Result<PreviewResolution, AutomixPreviewResolutionError>`.

- [ ] **Step 1: Write failing shared-materializer and preview-resolution tests**

```rust
#[test]
fn supported_preview_uses_shared_style_and_plan() {
    let request = decode_wire(valid_preview_proto()).unwrap();
    let resolved = resolve_automix_preview(request).unwrap();
    let PreviewResolution::Playable(preview) = resolved else { panic!("must render") };
    assert_eq!(preview.preset_id, 10);
    assert_eq!(preview.plan.current_start(), Duration::from_millis(184_812));
    assert_eq!(preview.plan.next_start(), Duration::from_millis(944));
}

#[test]
fn none_is_successful_no_preview_but_unknown_and_custom_are_rejected() {
    assert!(matches!(resolve_wire(with_preset(0)).unwrap(), PreviewResolution::NoTransition { preset_id: 0 }));
    assert_eq!(resolve_wire(with_preset(999)).unwrap_err(), AutomixPreviewResolutionError::UnknownPreset(999));
    assert_eq!(resolve_wire(with_custom_curve()).unwrap_err(), AutomixPreviewResolutionError::UnsupportedRenderer { preset_id: 10 });
}

#[test]
fn preview_provenance_still_cannot_authorize_live_resolution() {
    let result = resolve_transition_plan_sources(&active_edge(), Some(preview_candidate()), None, true);
    let SpotifyTransitionPlanResolution::LocalAuto { rejections } = result else { panic!("preview must not select live playback") };
    assert_eq!(rejections[0].reason, SpotifyTransitionRejection::PreviewOnly);
}
```

Add a regression assertion that a supported saved live candidate produces the same plan before and after the refactor.

- [ ] **Step 2: Run the focused tests and verify RED**

Run: `cargo test -p librespot-connect spotify_mix_preview::tests::supported_preview_uses_shared_style_and_plan -- --exact`

Expected: compile failure because the shared result and preview resolver do not exist.

- [ ] **Step 3: Extract one recipe materializer and call it from both routes**

Use one result type in `spotify_mix.rs`:

```rust
pub(crate) enum SpotifyRecipeMaterialization {
    None { preset_id: i32 },
    Playable {
        preset_id: i32,
        style: Box<ResolvedSpotifyStyle>,
        plan: TransitionPlan,
    },
}

pub(crate) fn materialize_spotify_recipe(
    recipe: &SpotifyTransitionRecipe,
) -> Result<SpotifyRecipeMaterialization, SpotifyRecipePlanError>;
```

Move the existing preset-0, style lookup, unsupported-capability gate, volume-plan construction, and speed automation into this function. Change live candidate evaluation to validate ownership/identity and return a resolved recipe even for preset 0; make `resolve_transition_plan_sources` translate shared `None` back into its existing `TerminalNone` result. Make preview call the materializer directly after preview validation:

```rust
pub(crate) enum PreviewResolution {
    NoTransition { preset_id: i32 },
    Playable(ResolvedAutomixPreview),
}

pub(crate) struct ResolvedAutomixPreview {
    pub request: AutomixPreviewRequest,
    pub preset_id: i32,
    pub style: Box<ResolvedSpotifyStyle>,
    pub plan: TransitionPlan,
}
```

There must be no call from `spotify_mix_preview.rs` to `resolve_transition_plan_sources` or `evaluate_recipe_candidate`, and no local/backend fallback branch.

Define `with_preset`, `with_custom_curve`, `resolve_wire`, `active_edge`, and `preview_candidate` as test-only builders in the owning modules. The supported-live regression must compare `TransitionPlan` timing, gain curves, and speed automation, not only its enum variant.

- [ ] **Step 4: Run preview and live resolver tests and verify GREEN**

Run: `cargo test -p librespot-connect spotify_mix_preview::tests`

Run: `cargo test -p librespot-connect spotify_mix::tests`

Expected: preview resolution tests pass and all existing live resolver/materializer tests remain green.

- [ ] **Step 5: Commit the green materializer slice**

```powershell
git add connect/src/spotify_mix.rs connect/src/spotify_mix_preview.rs
git diff --cached --check
git commit -m "refactor(connect): share Spotify recipe materialization"
```

### Task 4: Define playback preview ownership and exact frame accounting

**Files:**
- Create: `playback/src/preview.rs`
- Modify: `playback/src/lib.rs`

**Interfaces:**
- Consumes: `SpotifyUri`, `TransitionPlan`, and the evidenced 3000 ms request geometry.
- Produces: public `PreviewGeneration`, `PreviewAuthority`, `PreviewToken`, `PreviewTrack`, `PreviewPlaybackRequest`, `PreviewCancelReason`, `RetainedPlaybackDisposition`, `PreviewRestoreOutcome`, `PreviewFailure`, and `PreviewFrameBudget`.

- [ ] **Step 1: Write failing ownership and frame-budget tests**

```rust
#[test]
fn token_equality_requires_session_generation_and_normal_authority() {
    let token = token("session-a", 4, 9);
    assert_ne!(token, token("session-b", 4, 9));
    assert_ne!(token, token("session-a", 5, 9));
    assert_ne!(token, token("session-a", 4, 10));
}

#[test]
fn three_second_post_roll_is_exact_at_44100_hz() {
    let mut budget = PreviewFrameBudget::new(Duration::from_millis(3_000));
    assert_eq!(budget.remaining(), 132_300);
    assert_eq!(budget.take(100_000), 100_000);
    assert_eq!(budget.take(40_000), 32_300);
    assert!(budget.is_complete());
}
```

- [ ] **Step 2: Run the focused tests and verify RED**

Run: `cargo test -p librespot-playback preview::tests -- --nocapture`

Expected: compile failure because the preview module does not exist.

- [ ] **Step 3: Implement the typed ownership vocabulary**

```rust
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct PreviewGeneration(pub u64);

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct PreviewAuthority {
    pub connect_session_id: String,
    pub normal_ownership_generation: u64,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct PreviewToken {
    pub authority: PreviewAuthority,
    pub generation: PreviewGeneration,
}

#[derive(Clone, Debug)]
pub struct PreviewTrack {
    pub canonical: SpotifyUri,
    pub expected_playable: SpotifyUri,
}

#[derive(Clone, Debug)]
pub struct PreviewPlaybackRequest {
    pub token: PreviewToken,
    pub outgoing: PreviewTrack,
    pub incoming: PreviewTrack,
    pub outgoing_load_position_ms: u32,
    pub incoming_load_position_ms: u32,
    pub post_roll: Duration,
    pub plan: TransitionPlan,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RetainedPlaybackDisposition {
    Restore,
    Discard,
}
```

Use bounded enums rather than arbitrary strings: `PreviewCancelReason::{Replaced, NormalCommand, SessionChanged, Inactive, Shutdown}`, `PreviewRestoreOutcome::{Restored, DiscardedByAuthority, DiscardedByCommand, NotOwned}`, and `PreviewFailure::{OutgoingLoad, IncomingLoad, PlayableMismatch, SecondaryUnavailable, Render, PrematureOutgoingEof, PrematureIncomingEof}`. `PreviewFrameBudget::new` converts wall duration to frames using the same ceil-safe sample-rate calculation as the player. Its `take(available)` returns no more than the remaining frames and subtracts exactly that amount. Define the test `token(session, preview_generation, normal_generation)` helper alongside these types.

- [ ] **Step 4: Run the focused tests and verify GREEN**

Run: `cargo test -p librespot-playback preview::tests`

Expected: token and exact 132,300-frame budget tests pass.

- [ ] **Step 5: Commit the green ownership slice**

```powershell
git add playback/src/lib.rs playback/src/preview.rs
git diff --cached --check
git commit -m "feat(playback): model preview ownership"
```

### Task 5: Implement preview generation, duplicate attachment, replacement, and reply ownership

**Files:**
- Modify: `connect/src/spotify_mix_preview.rs`

**Interfaces:**
- Consumes: `PreviewAuthority`, `PreviewFingerprint`, `PreviewDescriptor`, and `mpsc::UnboundedSender<Reply>`.
- Produces: `PreviewCoordinator::admit`, `finish`, and `cancel` with exactly-once waiter draining.

- [ ] **Step 1: Write failing coordinator tests**

```rust
#[test]
fn identical_duplicate_attaches_without_new_generation() {
    let mut coordinator = PreviewCoordinator::default();
    let (first_tx, _first_rx) = reply_channel();
    let first = coordinator.admit(authority(7), fingerprint(1), descriptor(), first_tx).unwrap();
    let (duplicate_tx, _duplicate_rx) = reply_channel();
    let duplicate = coordinator.admit(authority(7), fingerprint(1), descriptor(), duplicate_tx).unwrap();
    assert_eq!(duplicate, PreviewAdmission::Attached(first.token().clone()));
    assert_eq!(coordinator.active().unwrap().waiter_count(), 2);
}

#[test]
fn materially_different_request_replaces_and_fails_old_waiters() {
    let (old_tx, mut old_rx) = reply_channel();
    let first = coordinator.admit(authority(7), fingerprint(1), descriptor(), old_tx).unwrap();
    let (new_tx, mut new_rx) = reply_channel();
    let second = coordinator.admit(authority(7), fingerprint(2), descriptor(), new_tx).unwrap();
    assert!(second.token().generation.0 > first.token().generation.0);
    assert!(matches!(old_rx.try_recv(), Ok(Reply::Failure)));
    assert!(matches!(new_rx.try_recv(), Err(TryRecvError::Empty)));
}

#[test]
fn optional_presence_difference_is_material_and_stale_finish_is_ignored() {
    let first = admit_with_presence(&mut coordinator, false);
    let second = admit_with_presence(&mut coordinator, true);
    assert_ne!(first, second);
    assert!(!coordinator.finish(&first, Reply::Success));
    assert_eq!(coordinator.active().unwrap().token(), &second);
}
```

Define test helpers `reply_channel() -> (mpsc::UnboundedSender<Reply>, mpsc::UnboundedReceiver<Reply>)`, `authority(generation) -> PreviewAuthority`, deterministic `fingerprint(byte) -> PreviewFingerprint`, and `descriptor() -> PreviewDescriptor`. Pin drain-on-shutdown here; `NONE` and decode rejection are wired and tested in Task 10 because they bypass active coordinator ownership.

- [ ] **Step 2: Run the focused coordinator test and verify RED**

Run: `cargo test -p librespot-connect spotify_mix_preview::tests::identical_duplicate_attaches_without_new_generation -- --exact`

Expected: compile failure because `PreviewCoordinator` does not exist.

- [ ] **Step 3: Implement the single-active-session coordinator**

```rust
pub(crate) struct PreviewCoordinator {
    next_generation: u64,
    active: Option<ActivePreviewSession>,
}

pub(crate) struct ActivePreviewSession {
    token: PreviewToken,
    fingerprint: PreviewFingerprint,
    descriptor: PreviewDescriptor,
    waiters: Vec<mpsc::UnboundedSender<Reply>>,
}

#[derive(Clone, Debug)]
pub(crate) struct PreviewDescriptor {
    canonical_a: String,
    playable_a: String,
    canonical_b: String,
    playable_b: String,
    context_uri: Option<String>,
    transition_uri: Option<String>,
    arm_id_present: bool,
    provenance: SpotifyTransitionProvenance,
    preset_id: i32,
    style_ids: SpotifyStyleIds,
    start_a_ms: u32,
    start_b_ms: u32,
    duration_ms: u32,
    outgoing_load_ms: u32,
    incoming_load_ms: u32,
    post_roll_ms: u32,
    item_speed_a_bits: u64,
    item_speed_b_bits: u64,
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) enum PreviewAdmission {
    Start(PreviewToken),
    Attached(PreviewToken),
    Replaced { retired: PreviewToken, started: PreviewToken },
}

```

The exact admission signature is `admit(&mut self, PreviewAuthority, PreviewFingerprint, PreviewDescriptor, mpsc::UnboundedSender<Reply>) -> Result<PreviewAdmission, PreviewCoordinatorError>`. `PreviewDescriptor` retains sanitized canonical/playable A/B, context/transition/arm presence, recipe provenance, preset/style IDs, and resolved timing/speed values for lifecycle logs; it contains no raw payload or account data. All admission variants expose `token(&self) -> &PreviewToken`; `ActivePreviewSession` exposes read-only `token()`, `descriptor()`, and `waiter_count()` helpers. `admit` attaches only when both authority and fingerprint match. Every other validated request drains old waiters with `Reply::Failure`, retires the old token, increments with `checked_add`, and starts a new session; exhausted generation returns `PreviewCoordinatorError::GenerationExhausted` rather than wrapping. `finish(&mut self, token: &PreviewToken, reply: Reply) -> bool` drains only when the complete token equals the active token. `cancel(&mut self, reply: Reply) -> Option<PreviewToken>` always drains and returns the retired token for player cancellation. `invalidate_authority(&mut self, reply: Reply) -> Result<(), PreviewCoordinatorError>` retires any active preview and consumes a generation even if none is active, so a later session cannot reuse a token number. Sender errors are logged and never retried.

- [ ] **Step 4: Run all coordinator tests and verify GREEN**

Run: `cargo test -p librespot-connect spotify_mix_preview::tests`

Expected: duplicate, replacement, stale completion, terminal reply, and shutdown-drain tests pass.

- [ ] **Step 5: Commit the green coordinator slice**

```powershell
git add connect/src/spotify_mix_preview.rs
git diff --cached --check
git commit -m "feat(connect): own preview generations and replies"
```

### Task 6: Retain normal player state and load preview A/B under the complete token

**Files:**
- Modify: `playback/src/player.rs`

**Interfaces:**
- Consumes: `PreviewAuthority`, `PreviewPlaybackRequest`, and `PreviewToken` from Task 4.
- Produces: `Player::set_preview_authority`, `Player::start_preview`, `Player::cancel_preview`, preview commands/events, and token-tagged A/B loading state.

- [ ] **Step 1: Write failing suspension/load tests**

Extend player test helpers with scripted pending/success/failure loaders, then add:

```rust
#[test]
fn preview_start_retains_normal_source_and_cancels_normal_transition_state() {
    let mut player = playing_test_player_at(42_123);
    install_ready_normal_preload(&mut player);
    player.handle_command(PlayerCommand::SetPreviewAuthority {
        authority: authority(9),
        retained: RetainedPlaybackDisposition::Discard,
        reason: PreviewCancelReason::SessionChanged,
    }).unwrap();
    player.handle_command(PlayerCommand::StartPreview(preview_request(token(1, 9)))).unwrap();
    assert!(player.preview.as_ref().unwrap().retained.state.is_playing());
    assert_eq!(player.retained_position_for_test(), Some(42_123));
    assert!(matches!(player.preload, PlayerPreload::Loading { .. }));
    assert_eq!(player.preview_load_positions_for_test(), Some((181_812, 944)));
}

#[test]
fn loaded_preview_sources_must_match_expected_playable_identities() {
    let (mut player, mut events) = preview_player_with_loaded_uri(OUTGOING, UNEXPECTED_PLAYABLE);
    player.poll_preview_loads_for_test();
    assert!(matches!(events.try_recv(), Ok(PlayerEvent::PreviewFailed { token, reason: PreviewFailure::PlayableMismatch, .. }) if token == preview_token(1, 9)));
}

#[test]
fn replacing_preview_transfers_retained_normal_state_without_restoring_between_requests() {
    let mut player = playing_test_player_at(42_123);
    player.start_preview_for_test(preview_request(token(1, 9)));
    player.start_preview_for_test(preview_request(token(2, 9)));
    assert_eq!(player.retained_position_for_test(), Some(42_123));
    assert_eq!(player.active_preview_token(), Some(token(2, 9)));
    assert_eq!(player.sink_start_count_for_test(), 1);
}

#[test]
fn stale_outgoing_or_incoming_load_completion_cannot_enter_replacement() {
    let mut player = playing_test_player_at(42_123);
    let old = player.start_preview_for_test(preview_request(token(1, 9)));
    player.start_preview_for_test(preview_request(token(2, 9)));
    player.inject_owned_load_result_for_test(old.outgoing_owner(), successful_source(OUTGOING));
    player.inject_owned_load_result_for_test(old.incoming_owner(), successful_source(INCOMING));
    assert_eq!(player.active_preview_token(), Some(token(2, 9)));
    assert_eq!(player.preview_sources_accepted_for_test(), 0);
}

#[test]
fn preview_reuses_the_existing_player_thread_and_sink() {
    let mut player = playing_test_player_at(42_123);
    let sink_identity = player.sink_instance_id_for_test();
    let player_id = player.player_id;
    player.start_preview_for_test(preview_request(token(1, 9)));
    assert_eq!(player.sink_instance_id_for_test(), sink_identity);
    assert_eq!(player.player_id, player_id);
}
```

Add load-failure tests for outgoing and incoming, and assert no ordinary `Loading`, `Preloading`, `TrackChanged`, `Unavailable`, or `LoadFailed` event is emitted for preview sources.

- [ ] **Step 2: Run the first player lifecycle test and verify RED**

Run: `cargo test -p librespot-playback player::tests::preview_start_retains_normal_source_and_cancels_normal_transition_state -- --exact`

Expected: compile failure because preview player commands and owner state do not exist.

- [ ] **Step 3: Implement suspension and dual source loading**

Add command/event vocabulary:

```rust
enum PlayerCommand {
    Load { track_id: SpotifyUri, play: bool, position_ms: u32 },
    Preload { track_id: SpotifyUri, transition: PreloadTransition },
    Play,
    Pause,
    Stop,
    Seek(u32),
    SetSession(Session),
    SetPreviewAuthority {
        authority: PreviewAuthority,
        retained: RetainedPlaybackDisposition,
        reason: PreviewCancelReason,
    },
    StartPreview(PreviewPlaybackRequest),
    CancelPreview { token: PreviewToken, reason: PreviewCancelReason },
}

pub enum PlayerEvent {
    PreviewStarted { token: PreviewToken },
    PreviewCompleted { token: PreviewToken, restore: PreviewRestoreOutcome },
    PreviewCancelled { token: PreviewToken, reason: PreviewCancelReason, restore: PreviewRestoreOutcome },
    PreviewFailed { token: PreviewToken, reason: PreviewFailure, restore: PreviewRestoreOutcome },
}
```

Add internal ownership:

```rust
struct PreviewOwner {
    request: PreviewPlaybackRequest,
    retained: RetainedPlayback,
    phase: PreviewPhase,
}

struct RetainedPlayback {
    state: PlayerState,
    recovery: Option<PlayerRecovery>,
    sink_was_running: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum SourceOwner {
    Normal,
    Preview(PreviewToken),
}
```

Wrap every loader so the completion itself returns its owner:

```rust
struct OwnedTrackLoader {
    owner: SourceOwner,
    inner: TrackLoaderFuture,
}

struct OwnedLoadResult {
    owner: SourceOwner,
    result: Result<PlaybackSource, PlayerLoadError>,
}
```

Use `OwnedTrackLoader` in `PlayerState::Loading` and `PlayerPreload::Loading`, add `owner: SourceOwner` to `PlayerPreload::Ready`, and add the owner to every source-bearing `PlayerState` variant (`Loading`, `Paused`, `Playing`, and `EndOfTrack`). Add `transition_owner: Option<SourceOwner>` beside `TransitionEngine`. Normal constructors use `SourceOwner::Normal`; preview A/B use `SourceOwner::Preview(request.token.clone())`. Before preview, call existing transition/secondary cancellation, clear crossfade ack, stop the sink, move `state` and `recovery` into `RetainedPlayback`, and create both loaders at the request's exact A/B positions. Poll results only if the returned full token equals the active owner. Validate each loaded `AudioItem.uri` against `expected_playable` before playback. Emit `PreviewStarted` once when A becomes the audible source; readiness of B must not emit a second start event.

Add test-only helpers with these signatures in the existing player test module so every assertion above is executable: `playing_test_player_at(u32) -> PlayerInternal`, `preview_request(PreviewToken) -> PreviewPlaybackRequest`, `install_ready_normal_preload(&mut PlayerInternal)`, `preview_player_with_loaded_uri(&str, &str) -> (PlayerInternal, PlayerEventChannel)`, and `sink_instance_id_for_test(&PlayerInternal) -> usize` backed by the existing counting test sink.

- [ ] **Step 4: Run focused suspension/load and existing loading tests and verify GREEN**

Run: `cargo test -p librespot-playback player::tests::preview_`

Run: `cargo test -p librespot-playback player::tests::load_`

Expected: preview suspension/load tests pass and existing normal load/preload tests remain green.

- [ ] **Step 5: Commit the green player loading slice**

```powershell
git add playback/src/player.rs
git diff --cached --check
git commit -m "feat(playback): load token-owned preview sources"
```

### Task 7: Propagate full preview ownership through secondary decoding

**Files:**
- Modify: `playback/src/secondary.rs`
- Modify: `playback/src/player.rs`

**Interfaces:**
- Consumes: `SourceOwner::Preview(PreviewToken)` when the player starts the incoming worker.
- Produces: `SecondaryDecodeOwner { generation, preview_token }` on every worker message and PCM block.

- [ ] **Step 1: Write failing stale-worker tests**

```rust
#[test]
fn secondary_pcm_carries_the_complete_preview_token() {
    let owner = SecondaryDecodeOwner::preview(12, token("session-a", 4, 9));
    let block = decode_one_test_block(owner.clone());
    assert_eq!(block.owner, owner);
}

#[test]
fn stale_preview_worker_cannot_cancel_or_feed_replacement() {
    let mut player = preview_test_player(token("session-a", 2, 9));
    inject_secondary_block(&mut player, token("session-a", 1, 9));
    assert_eq!(player.active_preview_token(), Some(token("session-a", 2, 9)));
    assert_eq!(player.preview_pcm_consumed_for_test(), 0);
}
```

Preserve the existing stale normal secondary-generation test.

- [ ] **Step 2: Run the focused stale-token test and verify RED**

Run: `cargo test -p librespot-playback player::tests::stale_preview_worker_cannot_cancel_or_feed_replacement -- --exact`

Expected: compile failure because secondary messages carry only `u64` generation.

- [ ] **Step 3: Tag every secondary artifact and compare complete ownership**

```rust
#[derive(Clone, Debug, Eq, PartialEq)]
struct SecondaryDecodeOwner {
    generation: u64,
    preview_token: Option<PreviewToken>,
}

struct SecondaryDecodeMessage {
    owner: SecondaryDecodeOwner,
    event: SecondaryDecodeEvent,
}

pub(crate) struct SecondaryPcmBlock {
    owner: SecondaryDecodeOwner,
    position: AudioPacketPosition,
    packet: AudioPacket,
    source_end_position_ms: Option<u32>,
}
```

Pass this owner into `SourceDecoder::start_secondary`, `SecondaryDecodeWorker`, `run_decode_worker`, terminal events, buffered PCM, stretched PCM, and blocks. Normal workers use `preview_token: None` and retain current numeric generation behavior. Preview workers carry `Some(token.clone())`. `try_take_secondary_frames` discards stale ownership without cancelling the currently active replacement preview.

Define `decode_one_test_block(SecondaryDecodeOwner)`, `preview_test_player(PreviewToken)`, and `inject_secondary_block(&mut PlayerInternal, PreviewToken)` with the existing scripted decoder/channel helpers. `preview_pcm_consumed_for_test()` reads the preview owner's frame counter and must remain zero for stale input.

- [ ] **Step 4: Run secondary and player ownership tests and verify GREEN**

Run: `cargo test -p librespot-playback secondary`

Run: `cargo test -p librespot-playback stale_preview_worker`

Run: `cargo test -p librespot-playback stale_secondary_generation_cannot_deliver_pcm`

Expected: new token tests and the pre-existing normal-generation regression pass.

- [ ] **Step 5: Commit the green secondary ownership slice**

```powershell
git add playback/src/secondary.rs playback/src/player.rs
git diff --cached --check
git commit -m "feat(playback): bind preview tokens to secondary PCM"
```

### Task 8: Render preview transition, hand off internally to B, and stop after post-roll

**Files:**
- Modify: `playback/src/player.rs`

**Interfaces:**
- Consumes: loaded preview A/B, scheduled `TransitionPlan`, token-tagged PCM, and `PreviewFrameBudget`.
- Produces: preview-only transition completion and exact B post-roll with no ordinary promotion/event path.

- [ ] **Step 1: Write failing renderer/handoff tests**

```rust
#[test]
fn preview_transition_uses_existing_engine_then_hands_off_without_promotion_events() {
    let (mut player, mut events) = armed_preview_test_player();
    render_through_transition(&mut player);
    let events = drain_events(&mut events);
    assert!(matches!(player.preview_phase(), PreviewPhase::PostRoll { .. }));
    assert!(!events.iter().any(is_ordinary_ownership_event));
    assert!(player.crossfade_load_ack.is_none());
}

#[test]
fn preview_b_plays_exactly_three_seconds_then_completes() {
    let (mut player, mut events) = post_roll_test_player(132_300 + 512);
    player.poll_preview_audio_for_test();
    let events = drain_events(&mut events);
    assert_eq!(player.preview_post_roll_frames_written_for_test(), 132_300);
    assert!(events.iter().any(|event| matches!(event, PlayerEvent::PreviewCompleted { .. })));
}

#[test]
fn preview_b_early_eof_is_failure_not_end_of_track() {
    let (mut player, mut events) = post_roll_test_player(132_299);
    player.poll_preview_audio_for_test();
    let events = drain_events(&mut events);
    assert!(events.iter().any(|event| matches!(event, PlayerEvent::PreviewFailed { reason: PreviewFailure::PrematureIncomingEof, .. })));
    assert!(!events.iter().any(|event| matches!(event, PlayerEvent::EndOfTrack { .. })));
}
```

Also assert outgoing begins at `start_a-3000`, transition begins at `plan.current_start()`, incoming begins at `plan.next_start()`, supported speed automation is retired once at handoff, and renderer failure is preview failure. Define `armed_preview_test_player`, `render_through_transition`, `post_roll_test_player`, `drain_events`, and `is_ordinary_ownership_event` in the existing player test module using its scripted PCM decoders and recording sink.

- [ ] **Step 2: Run the preview renderer test and verify RED**

Run: `cargo test -p librespot-playback player::tests::preview_transition_uses_existing_engine_then_hands_off_without_promotion_events -- --exact`

Expected: test failure because `complete_crossfade` still enters normal promotion.

- [ ] **Step 3: Add the preview branch around existing transition mechanics**

Keep `arm_transition_if_selected`, PCM readiness, `transition_mix_offset_frames`, and `TransitionEngine::render` shared. Branch only at ownership-sensitive completion:

```rust
fn complete_crossfade(&mut self) -> PlayerResult {
    if self.preview.is_some() {
        return self.complete_preview_transition();
    }
    self.complete_normal_crossfade()
}
```

`complete_preview_transition` first verifies `transition_owner == Some(SourceOwner::Preview(active_token))`, calls `transition.complete()`, drops outgoing preview A, moves incoming B from `PlayerPreload::Ready` into `PlayerState::Playing`, retires incoming speed DSP, sets `PreviewPhase::PostRoll(PreviewFrameBudget::new(3000ms))`, and emits none of the normal promotion events. During post-roll, cap each B packet to `budget.take(packet_frames)`, write only that slice, and call preview terminal completion exactly when the budget reaches zero. EOF/error before zero is `PreviewFailed`, never `natural_end_of_track_event`.

- [ ] **Step 4: Run preview renderer and live promotion regressions and verify GREEN**

Run: `cargo test -p librespot-playback player::tests::preview_`

Run: `cargo test -p librespot-playback player::tests::complete_crossfade`

Run: `cargo test -p librespot-playback player::tests::scheduled_`

Expected: preview timing/no-event tests pass and existing scheduled live promotion still emits its established event sequence.

- [ ] **Step 5: Commit the green renderer slice**

```powershell
git add playback/src/player.rs
git diff --cached --check
git commit -m "feat(playback): render isolated automix previews"
```

### Task 9: Restore retained playback safely and preempt preview on normal commands

**Files:**
- Modify: `playback/src/player.rs`

**Interfaces:**
- Consumes: current `PreviewAuthority`, retained `PlayerState`, session identity, and normal `PlayerCommand`.
- Produces: deterministic `restore_preview`, stale-authority discard, replacement cancellation, and defensive normal-command preemption.

- [ ] **Step 1: Write failing restoration/preemption tests**

```rust
#[test]
fn completed_preview_restores_exact_retained_source_and_play_intent() {
    let mut player = preview_from_playing_source_at(42_123);
    finish_preview(&mut player);
    assert_eq!(player.current_position_for_test(), Some(42_123));
    assert!(player.state.is_playing());
}

#[test]
fn retained_paused_source_remains_paused_after_preview() {
    let mut player = preview_from_paused_source_at(9_876);
    finish_preview(&mut player);
    assert_eq!(player.current_position_for_test(), Some(9_876));
    assert!(matches!(player.state, PlayerState::Paused { .. }));
}

#[test]
fn retained_loading_and_stopped_states_resume_without_preview_events() {
    for mut player in [preview_from_loading_state(), preview_from_stopped_state()] {
        finish_preview(&mut player);
        assert!(player.preview.is_none());
        assert!(player.retained_state_resumed_for_test());
        assert_eq!(player.ordinary_preview_source_event_count_for_test(), 0);
    }
}

#[test]
fn newer_authority_or_session_prevents_stale_restore() {
    let mut player = preview_from_playing_source_at(42_123);
    player.handle_command(PlayerCommand::SetPreviewAuthority {
        authority: authority(10),
        retained: RetainedPlaybackDisposition::Discard,
        reason: PreviewCancelReason::SessionChanged,
    }).unwrap();
    inject_preview_completion(&mut player, token(1, 9));
    assert_eq!(player.last_restore_outcome_for_test(), PreviewRestoreOutcome::DiscardedByAuthority);
    assert_ne!(player.current_position_for_test(), Some(42_123));
}

#[test]
fn every_normal_playback_command_preempts_before_execution() {
    for command in normal_ownership_commands() {
        let (mut player, events) = active_preview_test_player();
        player.handle_command(command).unwrap();
        assert_eq!(count_preview_cancelled(&events), 1);
        assert!(player.preview.is_none());
        assert!(normal_command_effect_observed(&player));
    }
}

#[test]
fn stale_renderer_completion_after_preemption_cannot_restore_or_promote() {
    let mut player = active_preview_test_player().0;
    player.handle_command(normal_load_command(TRACK_C)).unwrap();
    let after_load = player.ownership_snapshot_for_test();
    inject_preview_completion(&mut player, token(1, 9));
    assert_eq!(player.ownership_snapshot_for_test(), after_load);
}
```

- [ ] **Step 2: Run the authority-race test and verify RED**

Run: `cargo test -p librespot-playback player::tests::newer_authority_or_session_prevents_stale_restore -- --exact`

Expected: failure because restoration/preemption checks are not implemented.

- [ ] **Step 3: Implement terminal cleanup, restoration, and the defensive command gate**

```rust
fn restore_preview(&mut self, token: &PreviewToken) -> PreviewRestoreOutcome {
    let Some(owner) = self.preview.take() else { return PreviewRestoreOutcome::NotOwned };
    if owner.request.token != *token
        || self.preview_authority.as_ref() != Some(&token.authority)
        || self.session.session_id() != token.authority.connect_session_id
    {
        return PreviewRestoreOutcome::DiscardedByAuthority;
    }
    self.state = owner.retained.state;
    self.recovery = owner.retained.recovery;
    self.resume_restored_state()
}
```

All terminal paths first cancel transition/secondary state and drop both preview sources, then call this check. `resume_restored_state(&mut self) -> PreviewRestoreOutcome` returns `Restored`; for retained playing state it resets `reported_nominal_start_time`, restarts the existing sink, and emits one `PositionCorrection` for the retained canonical track/play-request ID at its exact retained source position. Retained paused state remains paused and emits no synthetic play event.

`SetPreviewAuthority` applies its disposition atomically: `Restore` terminates the old preview while the old authority is still current, restores retained playback, then installs the newer authority; `Discard` drops retained playback and installs the newer authority without restarting it. The defensive gate uses restore-before-execute for play/resume/pause/seek/preload and discard-before-execute for load/stop/session replacement. A new `StartPreview` with a newer token under the same authority replaces preview audio while transferring the one retained normal state; it does not briefly restore or restart the sink. Event-emission commands and volume/configuration commands do not preempt.

Define test helpers `preview_from_playing_source_at`, `preview_from_paused_source_at`, `preview_from_loading_state`, `preview_from_stopped_state`, `finish_preview`, `inject_preview_completion`, `normal_ownership_commands`, and `ownership_snapshot_for_test` in the player test module. The normal command table must contain concrete commands for load, preload, play, pause, stop, seek, and replacement session.

- [ ] **Step 4: Run restoration, preemption, and normal player lifecycle tests and verify GREEN**

Run: `cargo test -p librespot-playback player::tests::completed_preview_`

Run: `cargo test -p librespot-playback player::tests::retained_`

Run: `cargo test -p librespot-playback player::tests::newer_authority_`

Run: `cargo test -p librespot-playback player::tests::every_normal_playback_command_`

Run: `cargo test -p librespot-playback player::tests::stale_renderer_completion_`

Expected: all preview terminal/preemption cases pass and existing seek/load/recovery/session tests stay green.

- [ ] **Step 5: Commit the green restoration slice**

```powershell
git add playback/src/player.rs
git diff --cached --check
git commit -m "feat(playback): restore playback after previews"
```

### Task 10: Wire preview into SPIRC without queue ownership leakage

**Files:**
- Modify: `connect/src/spirc.rs`
- Modify: `connect/src/spotify_mix_preview.rs`

**Interfaces:**
- Consumes: typed `Command::Signal`, strict decode/resolution, `PreviewCoordinator`, player preview API/events.
- Produces: pending dealer replies, duplicate attachment/replacement, normal ownership epochs, early preview event handling, and zero queue advancement.

- [ ] **Step 1: Write failing SPIRC ownership tests**

Extract small request/event helpers so tests do not require a websocket. Pin:

```rust
#[test]
fn preview_signal_is_pending_until_matching_terminal_event() {
    let (sender, mut receiver) = reply_channel();
    let token = task.handle_preview_signal(valid_signal(), sender).unwrap().token().clone();
    assert!(matches!(receiver.try_recv(), Err(TryRecvError::Empty)));
    task.handle_player_event(PlayerEvent::PreviewCompleted { token, restore: PreviewRestoreOutcome::Restored }).unwrap();
    assert!(matches!(receiver.try_recv(), Ok(Reply::Success)));
}

#[test]
fn preview_terminal_event_bypasses_player_queue_action() {
    let before = queue_ownership_snapshot(&task);
    task.handle_player_event(PlayerEvent::PreviewCompleted {
        token: active_token(&task),
        restore: PreviewRestoreOutcome::Restored,
    }).unwrap();
    assert_eq!(queue_ownership_snapshot(&task), before);
}

#[test]
fn normal_command_preempts_preview_and_resolves_waiters() {
    for command in dealer_preemption_cases() {
        let (mut task, mut receiver, retired) = task_with_active_preview();
        task.handle_test_request(command).unwrap();
        assert!(matches!(receiver.try_recv(), Ok(Reply::Failure)));
        assert!(task.preview.active().is_none());
        assert!(task.player_cancelled_before_normal_action(&retired));
    }
}

#[test]
fn stale_preview_completion_cannot_complete_new_waiter_or_advance_queue() {
    let (mut task, old_token, new_token, mut new_receiver) = task_with_replaced_preview();
    let before = queue_ownership_snapshot(&task);
    task.handle_player_event(completed(old_token)).unwrap();
    assert_eq!(task.preview.active().unwrap().token(), &new_token);
    assert!(matches!(new_receiver.try_recv(), Err(TryRecvError::Empty)));
    assert_eq!(queue_ownership_snapshot(&task), before);
}
```

Also test `NONE` immediate success, malformed/unsupported immediate failure without disturbing an already-active valid preview, unknown signal failure, identical duplicate one player start/two waiters, different preview one cancellation/new start, outgoing/incoming load and renderer failures resolving every waiter with failure, session replacement/shutdown drain, a dealer responder disappearing before terminal completion, canonical/playable request transfer into `PreviewPlaybackRequest`, and unchanged ordinary live `EndOfTrack` advancement.

Build the test harness with `reply_channel`, `task_with_active_preview`, `task_with_replaced_preview`, `queue_ownership_snapshot`, and a recording player-command sender. `QueueOwnershipSnapshot` must include context URI, current URI/row, next URI/row, queue revision/content, `play_request_id`, and `transition_edge_generation`; equality is the zero-advancement proof.

- [ ] **Step 2: Run the pending-reply test and verify RED**

Run: `cargo test -p librespot-connect spirc::tests::preview_signal_is_pending_until_matching_terminal_event -- --exact`

Expected: failure because `handle_connect_state_request` immediately replies and SPIRC has no preview coordinator.

- [ ] **Step 3: Add SPIRC ownership fields and a preview-only dealer path**

Add a focused ownership field to `SpircTask`:

```rust
#[derive(Default)]
struct PreviewSpircOwnership {
    coordinator: PreviewCoordinator,
    normal_ownership_generation: u64,
}
```

Add `preview_ownership: PreviewSpircOwnership` to `SpircTask` and initialize it with `Default::default()` in both production and test constructors.

Implement the authority transition with checked generation and explicit retained-state policy:

```rust
impl SpircTask {
    fn preview_authority(&self) -> PreviewAuthority {
        PreviewAuthority {
            connect_session_id: self.session.session_id(),
            normal_ownership_generation: self.preview_ownership.normal_ownership_generation,
        }
    }

    fn advance_normal_ownership(
        &mut self,
        reason: PreviewCancelReason,
        retained: RetainedPlaybackDisposition,
    ) -> Result<(), PreviewCoordinatorError> {
        self.preview_ownership.normal_ownership_generation = self
            .preview_ownership
            .normal_ownership_generation
            .checked_add(1)
            .ok_or(PreviewCoordinatorError::GenerationExhausted)?;
        if matches!(reason, PreviewCancelReason::SessionChanged | PreviewCancelReason::Inactive | PreviewCancelReason::Shutdown) {
            self.preview_ownership.coordinator.invalidate_authority(Reply::Failure)?;
        } else {
            self.preview_ownership.coordinator.cancel(Reply::Failure);
        }
        self.player.set_preview_authority(self.preview_authority(), retained, reason);
        Ok(())
    }
}
```

In `handle_connect_state_request`, inspect `Command::Signal` before calling `connect_state.set_last_command`. For `automix-preview`, decode and resolve; reply immediately for rejection or `NONE`; otherwise call `PreviewCoordinator::admit`. Attached duplicates retain their sender and send no player command. Replacement sends `CancelPreview` for the retired token and `StartPreview` for the new token. Unknown signal IDs fail immediately.

Build the player request from canonical and expected playable identities, exact A/B positions, 3000 ms post-roll, and the resolved plan. Do not read or alter current/next Connect rows for preview selection.

Before every authoritative normal action—dealer transfer/play/pause/seek/skip, queue/context mutation, local API play/pause/previous/next/load/transfer/disconnect, session change, becoming inactive, and shutdown—increment `normal_ownership_generation`, publish `SetPreviewAuthority`, cancel active preview and drain waiters, then run existing behavior. Use `Restore` for commands acting on the retained current source (resume/play, pause, seek, and queue/preload changes) and `Discard` for commands replacing or relinquishing it (new-context play/load, transfer, skip to another source, disconnect, session ownership change, inactive, and shutdown). When starting a preview, publish its current authority before `StartPreview` so the player accepts only the exact token. Volume and sink-independent option updates remain non-preempting.

At the beginning of `handle_player_event`, consume `PreviewStarted`, `PreviewCompleted`, `PreviewCancelled`, and `PreviewFailed` by complete token. Return before `player_queue_action`, live transition logic, or Connect state mutation. Matching completion replies success; cancellation/failure replies failure; stale events log and return.

- [ ] **Step 4: Run SPIRC preview and live queue regressions and verify GREEN**

Run: `cargo test -p librespot-connect spirc::tests`

Run: `cargo test -p librespot-connect spirc::tests::player_queue_action_`

Run: `cargo test -p librespot-connect spotify_mix::tests`

Expected: all preview ownership/reply tests pass, preview snapshots show zero queue mutation, and ordinary live terminal events still advance exactly once.

- [ ] **Step 5: Commit the green SPIRC slice**

```powershell
git add connect/src/spirc.rs connect/src/spotify_mix_preview.rs
git diff --cached --check
git commit -m "feat(connect): coordinate Spotify automix previews"
```

### Task 11: Finalize safe diagnostics, run the complete local gate, and checkpoint state

**Files:**
- Modify: `core/src/mix_debug.rs`
- Modify: `connect/src/spotify_mix_preview.rs`
- Modify: `connect/src/spirc.rs`
- Modify: `playback/src/player.rs`
- Modify: `docs/CODEX_STATE.md`

**Interfaces:**
- Consumes: all preview lifecycle types and terminal outcomes.
- Produces: sanitized structured runtime evidence and a locally verified candidate commit.

- [ ] **Step 1: Write failing diagnostic-safety tests**

Add a formatting helper whose output can be asserted without installing a logger:

```rust
#[test]
fn preview_diagnostic_is_sanitized_and_ownership_explicit() {
    let line = preview_diagnostic(&resolved_preview(), PreviewLogEvent::Completed {
        restore: PreviewRestoreOutcome::Restored,
    });
    assert!(line.contains("generation=4"));
    assert!(line.contains("queue_advanced=false"));
    assert!(line.contains("normal_promotion_emitted=false"));
    assert!(!line.contains("preview_parameters="));
    assert!(!line.contains(&raw_base64_fixture()));
}
```

Add cases for capability rejection, duplicate attachment, replacement, start, cancellation, failure, completion, restore result, and stale-token rejection.

- [ ] **Step 2: Run the diagnostic test and verify RED**

Run: `cargo test -p librespot-connect spotify_mix_preview::tests::preview_diagnostic_is_sanitized_and_ownership_explicit -- --exact`

Expected: compile failure because the formatter does not exist and raw payload tracing still exists in `mix_debug.rs`.

- [ ] **Step 3: Implement concise structured diagnostics and remove raw payload logging**

Log only bounded fields from typed data:

```text
[spotify-preview] event=complete session=<bounded> generation=4 normal_generation=9
canonical_a=<uri> playable_a=<uri> canonical_b=<uri> playable_b=<uri>
fingerprint=<12 hex chars> provenance=preview-signal preset=10 style=<ids>
start_a_ms=184812 start_b_ms=944 duration_ms=7385 window_ms=3000
outgoing_load_ms=181812 incoming_load_ms=944 post_roll_ms=3000
restore=restored queue_advanced=false normal_promotion_emitted=false
```

Delete `runtime_trace!("preview_parameters={parameters}")` from `core/src/mix_debug.rs`. Log typed decode failures by stable enum name without echoing raw JSON/base64/protobuf. Ensure every terminal log includes the complete token and the two false ownership confirmations.

- [ ] **Step 4: Run preview-specific tests and the complete local verification gate**

Run in this order and record actual pass counts/warnings in `docs/CODEX_STATE.md`:

```powershell
cargo test -p librespot-core dealer::protocol::request::tests
cargo test -p librespot-connect spotify_mix_preview::tests
cargo test -p librespot-connect spirc::tests
cargo test -p librespot-playback preview::tests
cargo test -p librespot-playback player::tests::preview_
cargo fmt --check
cargo check --workspace
cargo test -p librespot-playback
cargo test -p librespot-connect
cargo clippy -p librespot-playback --all-targets
cargo clippy -p librespot-connect --all-targets
python -m unittest discover -s tools/runtime-diagnostics -v
node tools/spotify-automix-oracle/validate_input_fixtures.js
node --test tools/spotify-mixer-harness/test/capabilities.test.js tools/spotify-mixer-harness/test/classify.test.js tools/spotify-mixer-harness/test/cli.test.js tools/spotify-mixer-harness/test/report.test.js tools/spotify-mixer-harness/test/sanitize.test.js tools/spotify-mixer-harness/test/signature.test.js tools/spotify-mixer-harness/test/spotifyd.test.js tools/spotify-mixer-harness/test/stage_tracer.test.js
git diff --check
```

Do not change test expectations to hide a preview or live-transition regression.

- [ ] **Step 5: Update durable state and commit the locally verified candidate**

Replace obsolete state in `docs/CODEX_STATE.md` with: candidate commit ancestry, typed ingress, supported/rejected timing/profile behavior, token ownership, dedup/replacement, reply lifecycle, player suspend/render/post-roll/restore behavior, exact local results, and runtime status explicitly marked “not yet RPI-validated.” Set exactly one next action: deploy this exact candidate to RPI-01 and perform one bounded Mix Preview reproduction.

```powershell
git status --short
git diff --stat
git diff --check
git add core/src/mix_debug.rs connect/src/spotify_mix_preview.rs connect/src/spirc.rs playback/src/player.rs docs/CODEX_STATE.md
git diff --cached --check
git commit -m "docs: record automix preview candidate state"
git status --short
git push
```

Expected: clean worktree and pushed candidate whose exact commit is recorded before deployment.

## RPI-01 validation after the local gate

This is an execution gate, not a code-development task. Do it only after Task 11 is completely green.

1. Record `git rev-parse HEAD`, `git status --short`, and the pushed remote ref.
2. Connect with `ssh -o BatchMode=yes RPI-01 '<command>'`; verify identity, free disk, RAM, swap, current service status, deployed binary hash, and current rollback path.
3. Preserve the current known-good binary before replacing it. Never overwrite `/usr/local/bin/spotifyd.rollback-90e1628-pre-b63949c`.
4. Build/deploy the exact pushed candidate using the established RPI snapshot/target-directory workflow; verify deployed binary SHA-256 and revision.
5. Start one bounded journal/runtime-trace capture that filters typed `[spotify-preview]`, player transition, worker generation, queue, promotion, sink, XRUN, underrun, EPIPE, panic, and decoder-lifetime signals. Do not capture auth/account material or raw payloads.
6. Only when service playback and diagnostics are ready, ask the user once: “Please press Spotify’s Mix Preview control once for the selected transition, then tell me whether you heard A pre-roll → transition → B post-roll.”
7. Collect logs autonomously and correlate signal decode, token, A/B identities, exact seek positions, style/plan, preview start, renderer completion, 3000 ms post-roll, reply resolution, restoration, and `queue_advanced=false`.
8. If the first run is healthy, ask for no extra reproduction unless a materially different replacement/preemption or unsupported-style case is necessary to close a specific acceptance item.
9. Confirm no ordinary queue advance, live promotion, preview `TrackChanged`, stale action, decoder-lifetime regression, XRUN, underrun, EPIPE, panic, or sink failure.
10. Update `docs/SPOTIFY_LIVE_RECIPE_PROTOCOL.md` only with semantics proven by this run. Update `docs/CODEX_STATE.md` with exact commit/hash/log artifact and exactly one next action, then commit and push runtime-evidence documentation.

## Completion evidence

The implementation is complete only when the code, tests, and RPI evidence answer all ten success-gate questions from the specification: ingress; A/B and recipe decode; preview owner; stale rejection; renderable styles; source loading/positioning; renderer reuse; completion/cancellation; proof of zero ordinary queue corruption; and audible RPI-01 reproduction without lifecycle regressions.
