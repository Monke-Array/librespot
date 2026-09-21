# Spotify Automix Preview Playback Design

Date: 2026-09-21

Status: approved 2026-09-22; normative implementation contract

## 1. Purpose and scope

This specification defines playable handling for Spotify Mixer
`automix-preview` requests in librespot. It extends the current transition work
without reopening the live queue-edge resolver or creating another DSP system.

The normative flow is:

```text
Connect dealer signal
  -> typed preview envelope
  -> preview identity/timing validation
  -> existing Spotify recipe/style materializer
  -> existing TransitionPlan and transition renderer
  -> temporary preview-only playback ownership
  -> preview completion or cancellation
  -> retained normal playback restoration, when still authoritative
```

The words **MUST**, **MUST NOT**, **SHOULD**, and **MAY** are normative.

Preview is not ordinary Connect playback. Preview track identities select only
the two temporary preview sources. They never select, replace, reorder, or
advance the authoritative Connect queue.

## 2. Evidence and compatibility boundary

The current repository already proves the live path from Spotify semantic
recipe through deterministic style resolution, `TransitionPlan`, secondary PCM,
rendering, promotion, and one authoritative queue advance. Preview reuses that
semantic and rendering path, but not its live candidate or queue ownership.

Existing captures prove the following preview contract for the supported path:

- the request is a dealer `hm://connect-state/v1/player/command` command with
  endpoint `signal`, signal ID `automix-preview`, and a base64 protobuf string in
  `parameters`;
- the envelope carries canonical and playable A/B identities, an embedded
  `spotify.automix.proto.Transition`, a context URI, per-item speeds, and
  presence-bearing optional editor fields;
- the captured request uses `start_position_ms = 3000` with
  `relative_start_position = true`;
- the official player begins A at `start_a - 3000 ms`, renders the transition,
  and retains B for a matching 3000 ms post-roll;
- normal playback is paused before the request and resumed after the preview
  request completes;
- duplicate identical signals can arrive within one operation.

The initial implementation supports exactly that relative-window behavior.
Absolute start positioning and an explicitly present `stop_position_ms` are
rejected with stable reasons until a capture establishes their semantics.

## 3. Architectural invariants

The implementation MUST preserve all of these invariants:

1. Connect owns the normal context, current row, next row, queue, play request,
   and queue advancement.
2. A preview owns only temporary A/B sources, its renderer instance state, its
   reply waiters, and its preview generation.
3. Preview resolution MUST NOT call the ordinary live transition candidate
   resolver. In particular, preview cannot become a
   `SpotifyTransitionCandidate` accepted for a live edge.
4. Preview rendering MUST use the existing Spotify semantic recipe, style
   resolution, `TransitionPlan`, decoder, secondary PCM, and `TransitionEngine`
   implementations.
5. Preview MUST NOT create a second player thread or audio sink and MUST NOT be
   represented as a fake Connect context or queue.
6. Preview MUST NOT emit an ordinary preview `TrackChanged`, `EndOfTrack`, or
   live transition promotion event.
7. Preview completion MUST NOT call SPIRC next-track handling or alter the
   authoritative current/next rows.
8. Every asynchronous preview load, decoder, render, completion, cancellation,
   and dealer reply MUST be associated with the complete preview token.
9. A stale token MUST be observational only: it may be logged and discarded but
   cannot alter audio, restoration, replies, queue state, or a newer preview.
10. Normal authoritative playback commands always preempt preview.

## 4. Typed protocol and request model

### 4.1 Dealer command

`core::dealer::protocol::request::Command` gains a typed signal variant. Its
transport shape preserves `signal_id`, `parameters`, and logging parameters.
Unknown signal IDs remain unsupported commands; typing `signal` does not make
arbitrary signals successful.

### 4.2 Preview protobuf

The reconstructed preview schema moves from diagnostics into the production
protocol crate with its observed field numbers and proto2 presence:

| Field | Meaning |
| --- | --- |
| 1/2 | canonical track A/B URI |
| 3 | automix mode |
| 4 | optional transition URI |
| 5 | preview start/window value in milliseconds |
| 6 | relative-start flag |
| 7 | optional cuepoints |
| 8 | optional explicit stop position |
| 9 | embedded semantic `Transition` |
| 10/11 | playable track A/B URI |
| 12 | optional context URI |
| 13 | optional arm ID |
| 14/15 | item speed A/B |

The package name is a librespot reconstruction label, not an upstream package
claim.

### 4.3 Domain request

Wire data decodes into a domain type separate from live transition candidates.
The type retains field presence and contains:

- canonical and playable A/B;
- automix mode;
- optional transition URI, context URI, arm ID, cuepoints, and stop position;
- relative start/window value and presence;
- item speeds and presence;
- `SpotifyTransitionRecipe`;
- a deterministic fingerprint of the validated semantic request.

The domain type has no conversion into a live edge or live candidate. The only
lowering it exposes is preview recipe resolution and preview playback request
construction.

## 5. Decode and validation

The decoder applies bounded base64 and protobuf parsing before allocating
playback work. It rejects:

- absent, empty, oversized, invalid-base64, or invalid-protobuf parameters;
- absent or malformed canonical/playable A/B URIs;
- missing embedded transition;
- missing start value or missing relative flag;
- `relative_start_position != true`;
- a relative window other than the evidenced 3000 ms;
- an explicitly present stop position;
- missing, non-finite, or non-positive item speeds;
- non-`auto` captured mode in the initial capability profile;
- recipe canonical, playable, or item-speed mismatch against the envelope;
- negative recipe geometry, non-positive duration, `start_a < window`, or other
  existing recipe sanity failures;
- unsupported curve overrides or renderer capabilities.

No absent Spotify field receives a guessed default. Optional informational
fields remain optional and are logged only in sanitized form.

The preview start positions are:

```text
outgoing load position = recipe.start_a_ms - relative_window_ms
incoming load position = recipe.start_b_ms
post-roll wall duration = relative_window_ms
```

The transition plan retains the recipe's absolute source start positions. Thus
the existing scheduled transition policy begins mixing only after A has played
the evidenced pre-roll.

## 6. Recipe and style resolution

A preview-specific pure resolver accepts the validated preview request and
calls the same existing recipe-to-style and style-to-plan functions used after
live candidate selection. It does not call live source selection, precedence,
hydration, backend Auto, local Auto, or deterministic queue fallback.

Its outcomes are:

- `NoTransition`: known preset `NONE`; successful intentional no-preview result;
- `Playable`: a supported resolved style and `TransitionPlan`;
- `Rejected`: malformed identity/geometry, unknown preset, custom override, or
  unsupported renderer capability, with an exact reason.

Unknown or unrenderable is never converted to `NONE`. Rejection never
substitutes local Auto, normal crossfade, or another Spotify preset.

## 7. Preview identity, generation, and duplicate handling

### 7.1 Complete token

Every accepted playable preview receives a token containing:

```text
PreviewToken {
    connect_session_id,
    generation: PreviewGeneration(u64),
    normal_ownership_generation,
}
```

The preview generation monotonically advances for each materially new preview
and for session invalidation. The normal ownership generation binds retained
playback to the authoritative Connect/player state that existed when preview
started; the existing transition-edge generation may be reused if it covers all
queue/context invalidations needed by restoration. Token equality requires all
fields.

### 7.2 Active SPIRC session

SPIRC stores one `PreviewSession` containing the token, request fingerprint,
sanitized A/B and recipe provenance, lifecycle state, and every pending dealer
reply sender for that request.

When a signal arrives:

- no active preview: allocate a new generation and start it;
- same session and identical validated fingerprint: attach its reply sender to
  the existing session without restarting audio;
- different fingerprint: fail all old pending replies, cancel the old token,
  allocate a newer generation, and start the replacement;
- stale session: reject without playback work.

The fingerprint is over a canonical representation of every validated preview
domain field and every optional-field presence bit. Only dealer message IDs and
logging parameters are excluded. Thus only semantically identical envelopes
attach as duplicates; a changed context, provenance field, identity, timing,
recipe, or optional presence replaces the generation.

## 8. Dealer reply ownership

Starting a playable preview transfers the signal's reply sender into the active
preview session. The normal dealer-command handler does not immediately reply.
The SPIRC event loop remains non-blocking while replies are pending.

Every sender is resolved exactly once:

- successful preview completion: success;
- intentional `NONE`: immediate success;
- malformed or unsupported request: immediate failure;
- replacement by different preview: failure;
- source/load/decoder/render failure: failure;
- normal-command preemption: failure;
- session replacement or shutdown: failure.

Dropping SPIRC or losing the dealer still closes senders through existing
channel semantics; the implementation additionally drains active waiters during
explicit lifecycle teardown. No request can remain intentionally pending after
its preview session reaches a terminal state.

## 9. Player ownership model

### 9.1 One player and sink

`PlayerInternal` gains an optional preview owner. It reuses its existing source
loader, `PlaybackSource`, secondary decode worker, transition engine, converter,
normalisation, and sink.

The preview owner contains:

- complete preview token;
- retained normal `PlayerState` and its play intent;
- preview phase and expected identities;
- post-roll frame budget;
- cancellation/terminal reason.

Normal preload, transition, and crossfade-ack state are cancelled before the
normal state is retained. They are not restored because their edge may no
longer be authoritative after the preview interval; SPIRC may issue a fresh
preload after restoration.

### 9.2 Suspension

On preview start the player:

1. verifies that the command token is newer than any retired preview token;
2. cancels recovery-incompatible or active normal transition work;
3. stops the sink temporarily when needed;
4. moves the normal state into the preview owner without discarding its current
   decoder/source position;
5. starts asynchronous A and B loads tagged with the preview token;
6. suppresses all ordinary track lifecycle events for those preview sources.

A normal current source is retained at its exact last decoded position. A
normal loading or paused state may also be retained. If a state cannot safely be
retained, preview is rejected before replacing it.

### 9.3 Preview rendering

Once A is ready, preview playback starts without publishing normal player
identity. B enters the existing scheduled preload path. PCM readiness and
transition arming use the existing renderer rules.

At transition completion the player performs a preview-internal handoff to B.
This is not normal promotion: it emits no `EndOfTrack`, `TrackChanged`,
`PlayRequestIdChanged`, or queue-visible `Playing` event. It consumes exactly
the post-roll wall-frame budget, then terminates the preview.

Load failure, missing PCM, underrun, renderer error, or premature EOF terminates
the preview as failure. It does not continue a preview source indefinitely and
does not fall into ordinary queue playback.

### 9.4 Restoration

On completion or cancellation, the player drops both preview sources and all
preview renderer state before considering restoration.

It restores the retained normal state only if:

- the full preview token is still current;
- the player's current session identity still matches the retained session;
- the normal ownership generation still matches the retained authority;
- no newer authoritative normal player command has replaced the retained state.

For a retained playing source, restoration resets its nominal start time at the
retained source position, restarts the sink, and emits only the normal current
source's position update needed to resynchronize Connect. A retained paused
source remains paused. Loading/stopped states resume their ordinary behavior.

If authority changed, the player discards the retained state and obeys the new
normal command/session. It never restores stale normal playback over newer
Connect ownership.

## 10. Preemption and stale work

These commands preempt preview before executing: play, resume, pause, stop,
load, transfer, seek, previous, next, disconnect, session replacement, and
shutdown. Queue/context replacement that produces a new normal load is also
covered by the player's defensive command gate.

Volume-only and non-playback option updates may coexist because they do not
change source ownership.

Preemption occurs at both layers:

- SPIRC retires the active preview token and resolves dealer waiters;
- Player rejects or cancels work whose token is no longer current before
  running the normal command.

Async A/B loader completions, secondary PCM blocks, renderer completion, and
preview player events all compare the complete token. A completion from an old
generation cannot cancel, complete, restore, or promote a newer generation.

## 11. Player events and queue isolation

Preview exposes only typed events such as started, completed, cancelled, and
failed, each carrying the complete token and a bounded reason. SPIRC handles
these before ordinary `PlayerEvent` queue logic.

Preview source events are never translated to ordinary track events. The
existing `player_queue_action` receives no preview completion or preview source
terminal event. Therefore it cannot clear the live play request ID, call
`handle_next`, or advance the queue.

Tests take snapshots of current row, next row, queue revision/contents, live
play request ID, and transition edge generation before and after preview
terminal events. All remain unchanged except for ordinary commands explicitly
sent during preemption.

## 12. Observability

Concise structured diagnostics include:

- preview session ID and generation;
- canonical and playable A/B;
- request fingerprint prefix and recipe provenance;
- preset and resolved style IDs;
- recipe start A/B, duration, relative window, outgoing load position,
  incoming load position, and post-roll duration;
- capability result or exact rejection reason;
- duplicate attachment, replacement, start, cancellation, failure,
  completion, and restoration result;
- stale token rejection;
- terminal confirmation `queue_advanced=false` and
  `normal_promotion_emitted=false`.

Diagnostics never contain raw protobuf/base64 payloads, credentials, account
identity, audio, or arbitrary metadata.

## 13. Test contract

Implementation follows red/green tests and must cover at least:

1. valid captured preview decode with field presence;
2. invalid base64/protobuf, missing fields, and bounds rejection;
3. preview domain type cannot enter live candidate resolution;
4. canonical/playable/item-speed mismatch;
5. existing recipe/style resolution produces the expected plan;
6. known `NONE` is intentional no-preview success;
7. unknown/custom/unsupported style is explicit failure;
8. the evidenced 3000 ms relative window produces A=`start_a-3000`,
   B=`start_b`, and a matching 3000 ms post-roll, while other windows reject;
9. absolute-start and explicit-stop requests reject;
10. identical duplicate attaches without a new generation or audio restart;
11. materially different request replaces the generation and resolves old
    waiters;
12. stale loader, renderer, cancellation, and completion tokens have no effect;
13. normal playback commands preempt and resolve all waiters;
14. source load/decoder/PCM/render failure restores or yields to normal state;
15. preview completion emits no normal promotion, preview `TrackChanged`, or
    `EndOfTrack`;
16. preview completion performs no queue advancement;
17. restoration resumes the retained exact source and position when valid;
18. session/authority replacement prevents stale restoration;
19. existing live transition selection, rendering, promotion, and single queue
    advancement tests remain unchanged and green.

## 14. Expected implementation boundaries

Expected production changes are narrowly scoped to:

- `protocol/proto/automix_preview.proto` and protocol build inputs;
- `core/src/dealer/protocol/request.rs` for typed signal ingress;
- a focused Connect preview decode/resolution module;
- `connect/src/spirc.rs` for preview coordination, reply waiters, and event
  handling;
- `playback/src/player.rs` for temporary preview ownership and lifecycle;
- minimal reusable helper changes in `playback/src/secondary.rs` only if required
  by token validation;
- diagnostics and protocol/state documentation.

The existing live hydration/source resolver and preset tables are reused rather
than copied.

## 15. Verification and live acceptance

Local acceptance requires the repository's full formatting, workspace check,
playback/connect test, clippy, diagnostics, Mixer/oracle, and diff checks, with
actual counts recorded.

After a clean commit and push, the exact candidate is deployed to RPI-01 while
preserving the known-good binary. Bounded diagnostics are prepared before one
user reproduction request.

Live acceptance requires evidence that:

- the preview is audible from A pre-roll through supported transition and B
  post-roll;
- supported Spotify timing/style is used;
- identical duplicates do not restart audio;
- materially different preview replaces the active generation;
- normal commands preempt and recover;
- the real queue, current/next ownership, and live transition generation do not
  advance because of preview;
- no stale promotion, decoder-lifetime regression, XRUN, underrun, EPIPE, or
  sink regression occurs.

An unsupported-style preview should be included when available to prove clean
rejection. Unit tests alone do not close audible acceptance.

## 16. Explicit non-goals

This milestone does not define physical EQ/filter/FX mappings, custom curve
semantics, jogwheel/looping, absolute preview starts, explicit preview stops,
preview-driven queue edits, backend recipe fetches, local Auto substitution,
multiple sinks, or broad Spotify protocol archaeology.

Any future support for those behaviors requires new evidence and a separate
capability change rather than an implementation-time guess.
