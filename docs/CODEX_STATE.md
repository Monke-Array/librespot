# Current objective

Keep Spotify transition playback correct on the
`codex/m3a-live-auto-metadata` branch before doing further transition-quality
work. Mixer/Auto and ordinary-playlist crossfade routes must remain distinct.

# Current branch / commit

- Branch: `codex/m3a-live-auto-metadata`
- Current HEAD is the documentation-only commit titled
  `docs: establish Codex external memory`.
- Production-code baseline at the 2026-09-06 checkpoint: `e46504c`
  (`fix(playback): bound secondary buffering by PCM duration`).
- The worktree was clean apart from the previously untracked `AGENTS.md` before
  this documentation consolidation.

# Architecture / invariants

- Playback state flows from Connect queue/context through SPIRC edge selection,
  route selection, preload, secondary decode, PCM readiness, transition render,
  promotion, and post-promotion queue/player ownership.
- Mixer context selects the local Mixer/Auto transition route.
- Non-Mixer context selects the normal configurable crossfade route.
- Queue advancement, EOF handling, promotion, and session replacement must
  agree on one owner; secondary readiness/promotion is not a second track end.
- Deterministic playback and fallback behavior must remain available.
- Experimental ML belongs outside the playback state machine. A learned ranker
  may score deterministically valid plans but cannot own queues, decoders,
  buffers, positions, clipping constraints, or state transitions.
- RPI-01 is the integration target. S-01 and S-02 are optional offline compute
  and never runtime dependencies.

# Confirmed findings / root causes

- Commit `29afab6` records the selected transition-ownership fix in Connect.
- Commit `e5f944c` introduced configurable normal-playlist crossfade separately
  from the Mixer route.
- Commits `5a5f6f7` and `d07faf2` contain Mixer Auto preload readiness and local
  Auto-plan materialization.
- Commit `e46504c` bounds secondary buffering using decoded PCM duration.
- These are verified repository facts from Git history, not fresh audible proof.

# Latest tests

- No Rust source changed during the 2026-09-06 infrastructure consolidation.
- Code-level Rust tests were not rerun solely for this documentation change.
- The documentation/ignore change must pass `git diff --check` before commit.

# Latest runtime evidence

- No new RPI-01 playback reproduction was performed during this infrastructure
  task.
- Current-HEAD audible transition behavior therefore remains unverified by this
  checkpoint; unit history must not be presented as runtime proof.

# External jobs / artifacts

- S-01 worker inventory on 2026-09-06 reported zero active jobs.
- Transition-quality research and artifacts live in the independent
  `/home/profdrhuso/Projects/spotify-transition-ml` repository on S-01.
- Use worker job IDs and artifact/report paths here in future sessions, not raw
  log copies.

# Known unresolved issues

- The exact current librespot/spotifyd pair still needs production-like RPI-01
  validation for Mixer and non-Mixer routes.
- Audible evidence must cover transition start, promotion, queue advancement,
  EOF, and Spotify session replacement without duplicate ownership events.

# NEXT ACTION

Deploy the exact current librespot and spotifyd candidate revisions to RPI-01,
then run one controlled Mixer and one non-Mixer playback sequence while
capturing transition, promotion, queue-advance, EOF, and reconnect evidence.
