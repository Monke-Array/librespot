# librespot / Spotify Transition Project

Before substantial work, read `docs/CODEX_STATE.md` for verified current state.
Use `.codex/WORKING_MEMORY.md` only for temporary unresolved work; it is ignored
and must not be committed. Global context, machine, and power-ownership rules
come from `C:\Users\janni\.codex\AGENTS.md` and are not duplicated here.

## Repository rules

Primary development happens on U-01.

Current feature branch:

    codex/m3a-live-auto-metadata

Before edits:

    git status --short
    git branch --show-current

Do not reset this branch to upstream/master or an earlier snapshot.

Do not run `cargo update`.

Do not include unrelated research/debug artifacts in production commits.

Before committing, inspect:

    git status --short
    git diff --stat
    git diff --check

## Current architecture

The project contains custom transition functionality including:

- Spotify Mixer context recognition;
- local Auto transition generation;
- scheduled transition plans;
- secondary decoding/preloading;
- PCM readiness;
- transition rendering;
- secondary-to-current promotion;
- configurable normal crossfade.

Two runtime routes must remain distinct:

Mixer context:
    Mixer/Auto transition path

non-Mixer context:
    normal configurable crossfade path

A deterministic playback path must always remain available.

## Current priority

Playback correctness comes before transition-quality ML.

The important state path is:

Connect queue/context
    -> SPIRC edge selection
    -> transition route selection
    -> preload
    -> secondary decoder
    -> PCM readiness
    -> transition arm/start
    -> rendering
    -> promotion
    -> queue/player ownership after promotion

Queue advancement, EOF handling, transition promotion, and Spotify session
replacement must remain coherent.

Do not hide playback-state bugs with sleeps, arbitrary buffering, forced
hard cuts, or by disabling transition functionality.

## ML boundary

Experimental ML development does NOT belong directly in the playback state
machine.

Training, dataset generation, rendering, and research belong on S-01.

RPI-01 must never require S-01 for playback.

Initial production-oriented ML direction:

track A/B metadata
    -> deterministic safe candidate generator
    -> candidate transition plans
    -> learned critic/ranker
    -> best valid candidate
    -> deterministic librespot renderer

Hard runtime constraints remain deterministic.

The neural model may rank safe transition candidates but must not control:
- queue ownership;
- decoder lifetime;
- buffer safety;
- legal playback positions;
- clipping constraints;
- transition state-machine correctness.

## Runtime validation

Unit tests alone are not sufficient evidence that audible transition behavior
is fixed.

When runtime validation is needed:

1. finish candidate changes and tests locally;
2. deploy the exact candidate to RPI-01;
3. prepare logging autonomously over SSH;
4. ask the user for one concise Spotify reproduction;
5. collect logs autonomously afterward;
6. correlate audible behavior with runtime state.

Do not ask the user to manually relay journalctl/systemd output when SSH access
is available.

## Testing

For relevant changes, use the smallest useful targeted tests first.

Before claiming completion, normally verify as applicable:

    cargo fmt --check
    cargo check --workspace
    cargo test -p librespot-playback
    cargo test -p librespot-connect
    git diff --check

Report actual test results rather than merely stating that tests were run.
