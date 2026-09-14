# Current objective

Live-validate the exact `b63949c` runtime candidate while the bounded recorder
collects evidence. Keep M5 open until the storage/XRUN and playback lifecycle
fixes have meaningful active-playback evidence.

# Branch and deployment

- Branch: `codex/m3a-live-auto-metadata`; current/pushed HEAD:
  `b63949c10895735f0acd78252aef7d406e14dbf4`.
- ARM snapshot: `/home/amogus/.cache/spotifyd-runtime-build-b63949c`; all eight
  librespot manifest pins and all eight lockfile sources use exact `b63949c`.
- Build unit `spotifyd-arm-build-b63949c.service` succeeded with
  `--release --locked -j 2`, Nice 15, and idle I/O priority. Cargo reported
  63m24s wall time and systemd reported 40m56s CPU time.
- Built/deployed artifact:
  `/home/amogus/.cache/spotifyd-runtime-build-b63949c/target/release/spotifyd`.
  SHA256 of both artifact and `/usr/local/bin/spotifyd`:
  `32e6d39c07aeb2a55bfa5fb247f99e5d155a95aeb7e5143541248bc8d00d4af3`.
- Rollback `/usr/local/bin/spotifyd.rollback-90e1628-pre-b63949c` SHA256:
  `54a8c36d487c5cbc39239fac1dd9f19fa9a50a5fc82695259b2e315d3edd2e9c`.
  Older rollback `/usr/local/bin/spotifyd.rollback-09369c86-pre-90e1628`
  remains available with SHA256
  `c2f208c471ced9e2675f5f48fe32b06c0d6d41c0b42cd718b49025ca92edac5d`.
- The intended reused target directory still contains the old artifact; it was
  rejected by SHA verification. The successful unit wrote the recorded artifact
  to the snapshot-local target. No stale artifact was deployed.
- Post-deploy startup self-identifies as `librespot-b63949c1`, authenticated,
  connected to the AP and dealer, and left spotifyd plus the system diagnostic
  recorder active with zero restarts. Automatic incident count remains three.

# Architecture and invariants

- Spotify/Connect chooses what plays. Transition logic may choose only how the
  authoritative outgoing/incoming pair is handed off.
- A playing-source seek closes a Running sink before decoder seek/read-ahead,
  cancels secondary/transition ownership, and leaves restart to the normal poll
  path after PCM exists.
- A changed-position same-track Load uses a fresh loader instead of synchronously
  seeking a current or ready decoder. With no current PCM producer, the sink is
  temporarily closed until the loader succeeds.
- A changed-position plan for a dormant direct secondary retires its generation
  and reloads asynchronously. The current producer and sink continue running.
  A same-position update preserves decoder identity and generation without churn.
- Ready/loading secondary state belongs to its session, target, and generation.
  Cancellation retires runnable worker/transition DSP state; stale generations
  cannot provide PCM or promote.
- Resolved context replacement compares the active `(current, next)` edge. A
  changed authoritative edge invalidates the old transition and preloads the new
  next; an unchanged edge does not churn ownership.
- Promotion is single-owner, preserves the incoming source clock, and retires
  transition DSP before ordinary playback owns the source.

# Confirmed fixes

- `f51a26b`: stop the sink before blocking playing-source seek/read-ahead.
- `09369c8`: raise `slow_operation=sink_write` from 100 ms to 250 ms; ordinary
  ALSA writes around 118-129 ms are no longer false slow events.
- `eab442a` and `477c2c9`: recorder telemetry timeout handling and persistent
  journal cursors prevent crashes and replayed XRUN incidents.
- `b78e849`: refresh preload/transition ownership when a resolved context changes
  the authoritative active edge.
- `40b713c`: publish a completed same-filesystem audio download into cache by
  hard link, with copy fallback, eliminating a second full-file SD-card write.
- `064037d`: reject promotion of a ready direct source at the wrong requested
  position; the normal fresh loader owns the request instead.
- `b63949c`: eliminate synchronous changed-position seeks from same-track Load
  reuse and ready-secondary plan retargeting.

# Regression and local verification

- Cache regressions prove same-filesystem inode reuse and copy fallback.
- `different_position_load_retires_ready_fallback_and_starts_fresh_loader`
  proves a mismatched ready source cannot be synchronously sought/promoted.
- `different_position_load_reopens_playing_source_without_blocking_seek` proves
  changed-position current Load cancels transition ownership, closes the sink,
  and enters the fresh loader without touching a panic-on-seek decoder.
- `same_track_plan_position_change_reloads_secondary_without_blocking_current`
  failed before `b63949c` at a panic-on-seek decoder and now proves replacement
  loader/generation ownership while the current sink stays Running.
- The paired same-position regression proves a changed plan can retain the same
  ready decoder and generation when its incoming start position is unchanged.
- Final gate at `b63949c`: fmt passed; workspace check passed; playback 99/99;
  Connect 116/116; Spotify Auto oracle 5/5; doctest 1/1; playback/connect
  all-target Clippy passed with the six established pre-existing allowances;
  diff check passed.

# Runtime evidence

- Preserved XRUN `1789384890613886092`, ALSA trigger
  2026-09-14 13:21:30.117011+02, is at
  `/var/lib/spotifyd-diagnostics/manual/1789384890613886092-xrun`.
- First divergence was `current_decoder_next_packet` blocking 3.336338s while
  ALSA stayed Running, followed by EPIPE. Queue, context, preload, promotion, and
  CPU/memory state were coherent; no nearby dealer/context command existed.
- A completed 12,002,028-byte preload had been copied to a second cache inode.
  Delayed writeback was about 24.1 MiB, SD waits reached 3.7s, and the current
  cached-source read waited 3.466s. A clean 9,302,932-byte comparison download
  later produced about 18.9 MiB delayed writes, confirming approximately 2x
  cache write amplification. `40b713c` removes that duplicated same-filesystem
  data write; live post-fix writeback measurement is pending.
- The older audible stop incident `1789301128310568290` occurred while an ARM
  build saturated SD I/O: current decode stalled 4.486s and 1.447s, with 60-66%
  I/O wait and healthy memory. It is build-induced storage starvation, not
  memory exhaustion. Duplicate `1789301558842656438` was recorder cursor replay.
- User-reported Hypa Hypa -> brief Y.K.P -> Nash Gimn skip is preserved at
  `/var/lib/spotifyd-diagnostics/manual/1789308023769543909-skip`. It had no
  XRUN/network/load error; resolved context changed A -> B to A -> C after B was
  ready, and stale B promoted before C loaded. `b78e849` fixes that ownership bug.
- Pre-fix 100 ms sink-write tracing produced 67 events in eight active seconds
  (about 502/min). A later 2,181-second active window at 250 ms produced zero
  sink-write events and about 23.2 KiB/min total journal traffic. Genuine writes
  above 250 ms remain observable.
- Runtime traces before `b63949c` showed same-target plan retarget commands block
  for 157.380 ms and 95.209 ms in the synchronous secondary seek. This reachable
  hazard is fixed, but is not attributed to an older XRUN without matching
  preserved evidence.

# Targeted audit and recovery findings

- Transition generation, cancellation, EOF/promotion exclusivity, queue/context
  replacement, session replacement, and DSP retirement are coherent under the
  current regressions. No further demonstrated transition defect is open.
- Preload TransientService cancels its secondary/transition state, emits a
  preload LoadFailed event, and preserves the Connect queue. It does not advance
  or poison the authoritative next track.
- Current-source transient failure latches recovery to the same URI, position,
  request, and play intent; starved recovery closes the sink. Success resumes
  once, repeated transient failure stays in bounded/slow latched retry, permanent
  failure becomes Unavailable, and SessionInvalid waits for replacement.
- New Load/seek/stop commands invalidate recovery generations. Session replacement
  restarts current recovery and in-flight/ready preload against the new session;
  stale completions cannot regain ownership.

# Unresolved issues

- The deployed cache publication, Load/preload lifecycle changes, and existing
  seek fix need active live validation. No manual/natural seek has yet been
  observed in this validation window.
- No natural A -> B to A -> C resolved-context edge replacement has yet occurred
  after the edge fix; startup and ordinary edges do not prove that live boundary.
- Earlier context-adjacent XRUNs lack preserved timelines and remain individually
  unclassified. Context activity is not assumed causal.
- M5 remains open until active playback shows no unexplained XRUN and the new
  cache path demonstrates reduced writeback without destabilizing transitions.

# NEXT ACTION

Continue passive cursor-bounded monitoring during normal playback; on the next
completed uncached preload, verify hard-link publication/writeback volume and
preserve any audible incident or XRUN before changing priorities.
