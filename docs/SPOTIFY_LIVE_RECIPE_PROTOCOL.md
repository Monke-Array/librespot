# Spotify live transition protocol investigation

Investigation date: 2026-09-15. Status: IN PROGRESS; live upstream recipe
origin and first official/librespot protocol divergence are NOT yet proven.

## Scope and authority

Connect owns the outgoing/incoming queue edge. Recipe identities validate that
edge; they cannot select tracks. No production playback, DSP, queue ownership,
or local Auto generation changes are part of this investigation.

Baseline: branch `codex/m3a-live-auto-metadata`, `b63949c`; official Windows
Spotify 1.3.0.277. RPI-01 spotifyd and diagnostics were active at
2026-09-15T16:41:39+02:00. Existing runtime validation remains open as recorded
in CODEX_STATE.md.

## Existing evidence (do not mistake for new live-path proof)

- `tools/runtime-diagnostics/automix_preview.proto`: September 12 desktop codec
  reconstruction; envelope SHA256
  `b26a59943bf8da2e1331931a84ca1bba3ef90a09de86a685bb6d4f0cad4abf2c`.
  Private original: `.codex/runtime-20260912/preview/preview-000.bin`.
- `protocol/proto/automix_transition.proto`: existing Transition/Overlap/Preset
  schema reconstructed from the August desktop generated codec.
- August 25 private stage trace
  `tools/spotify-mixer-harness/corpus/reports/stage-trace-2026-08-25T10-13-14-893Z.json`
  already shows materialized audio metadata in native ContextPlayer output,
  before XPUI decoding. This is an application IPC observation, not a backend
  packet capture. Zero intercepted XPUI Automix calls does not exclude native
  computation or native network requests.
- `tools/spotify-automix-oracle/README.md` documents canonical/playable splitting
  in native Auto input fixtures. This does not by itself establish live routing.

## Experiment matrix

| ID | UTC time | Endpoint/action | Fixed context/pair | Mixer | Result |
|---|---|---|---|---|---|
| Setup | 14:42–14:44 | Launch desktop; attach existing stage tracer | Restored paused Bounce → Maldito Alcohol | Context says auto | Materialized native queue state present; not a new audible transition |
| E1 | From 14:44:11 | Phone controls official desktop; natural progression requested | Playlist `5tz0Ioe9JTJyPB4Sd2ewT8`; HIGHEST IN THE ROOM → Praise The Lord | Context auto; user confirmation pending | Collecting; startup/transfer events must be excluded |
| E2 | Pending | Same phone, same context/pair on RPI | Must match E1 identities and configuration | Same | Pending |
| E3 | Pending | Official replay with native logging | Same as E1 | Same | Needed to distinguish native computation from received recipe |
| E4 | Pending | One controlled Mixer setting variation | Same pair/context | One variable | Pending |

Private captures: `.codex/protocol-20260915/` (ignored). Never commit native
logs, credentials, authorization headers, account data, or audio. Durable samples
must be selected, decoded, and sanitized separately.

## Observed paths and uncertainty

```text
Phone action → Spotify/Connect → [native desktop ingress NOT YET OBSERVED]
    → native ContextPlayer
    → Esperanto ContextPlayer.GetQueue / GetState
    → native bridge response → generated XPUI decoder → PlayerAPI
```

The stage tracer observes `executeEsperantoCall` responses and decoded queue
streams. These are native-to-UI messages. They do not prove a server supplied
the materialized `audio.*` fields.

```text
Phone action → Spotify/Connect → dealer player command
    → JSON/protobuf context decoding → SPIRC authoritative edge
    → inline recipe / saved-transition hydration / local Auto fallback
```

Concrete existing librespot paths:

| Path | Existing handling | Evidence/limitation |
|---|---|---|
| Dealer `endpoint=signal`, `signal_id=automix-preview` | Diagnostic base64 extraction; command falls through Unknown | `core/src/mix_debug.rs`, `core/src/dealer/protocol/request.rs`; preview reception is proven, ordinary playback use is not |
| Track `automix.auto_transition_recipe` | Base64 → Transition → validated recipe | `connect/src/spotify_mix.rs`; availability in matched live command pending |
| Track `automix.transition_uri=spotify:transition:<id>:<revision>` | POST `/extended-metadata/v0/extended-metadata`, extension 244 → TransitionData | `connect/src/spotify_mix_hydration.rs`, `core/src/spclient.rs`; already implemented, do not propose as wholly missing |
| `spotify:core-auto-transition` | Not accepted by saved-transition URI parser | Observed in official native output; not evidence that it is a remotely fetchable resource |
| `automix.backend_auto_transition` | Recognized but not enabled by current client | `connect/src/spirc.rs`; meaning and negotiated availability require direct observation |
| Unknown context JSON fields | Protobuf JSON parsing allows unknown-field omission | `core/src/dealer/protocol.rs`; potential loss boundary, not a proven cause |

First protocol divergence: **unresolved**. Exact missing live request/decoder:
**unresolved**. A downstream difference is not sufficient to identify either.

## Proposed semantic contract (provisional)

The repository already has a `SpotifyTransitionRecipe` wrapper around the
Transition protobuf. Extend its provenance/identity contract only after ingress
is established; do not create a second unrelated recipe type.

```text
SpotifyTransitionRecipeEvidence {
  authoritative_edge_reference: context + row UIDs + canonical A/B + generation
  outgoing_identity, incoming_identity
  outgoing_playable_identity?, incoming_playable_identity?
  recipe: Transition?                 // retain protobuf presence/unknown fields
  materialized_audio_attributes?      // a distinct representation, if observed
  source: transport + service/method + timestamp + payload hash + client version
  availability: preview | live | both | unverified
  confidence_by_field
}
```

Recipe optional fields include start A/B in ms, duration ms, speed A/B, BPM A/B,
bars, beatmatched, preset, overlap override, and item speeds. Materialized
incoming/outgoing fade durations can differ; do not silently collapse them to
one duration. Missing values are not zeros. Preset IDs are not PresetType enums.

## Risks and acceptance gate

- UI state can briefly combine a new track with an old context/queue during
  transfer. Correlate settled native queue and state messages, not one snapshot.
- A restored/remote queue can contain materialized fields without this desktop
  having computed or played them. Verify active endpoint and natural transition.
- A sentinel URI or binary diagnostic string proves existence, not execution.
- Canonical identity can match while playable audio differs; require explicit
  relink evidence before applying source timing to decoded audio.
- Preview recipe arrival cannot advance the queue or authorize a live plan.
- A recipe containing only preset ID requires style/curve resolution; data
  receipt and exact rendering support are separate gates.

## Recommended Sol High task (not implementation-ready)

Do not implement a speculative live request yet. After the matched experiment
identifies ingress, implement only its bounded receive/request/decode adapter,
preserve field presence and provenance, validate the current authoritative edge
and playable identities, and expose a diagnostic recipe without playback changes.
Acceptance requires reproducing the official recipe from a matched real A/B
capture; negative tests must reject stale, preview-only, and mismatched edges.
Replace this section with the exact proven transport/schema/task before handoff.
