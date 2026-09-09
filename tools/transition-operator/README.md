# Offline transition operator

This standalone Rust workspace implements the offline-only, deterministic
`OperatorPlan/v1` pipeline. It deliberately has no dependency on librespot
playback crates and is not part of the repository's root Cargo workspace.

Run its tests with:

```powershell
cargo test --manifest-path tools/transition-operator/Cargo.toml
```

The `.private/` and `artifacts/` directories are local-only boundaries. Never
place private source audio or private corpus manifests in Git.

## M1 foundation

The library currently provides the approved `OperatorPlan/v1` data model,
integer-only canonical JSON, fixed validation order, semantic and provenance
identities, derived renderer requirements, and descriptive current-runtime
compatibility. Renderer and candidate-generation code are deliberately absent.

Canonical plan fixtures under `tests/fixtures/plans/` contain no trailing
newline. Their canonical bytes, plan/audio hashes, domain-separated candidate
IDs, and capability requirements are frozen in `golden-identities.json` and
checked independently by Rust and Node:

```powershell
cargo test --manifest-path tools/transition-operator/Cargo.toml
node --test tools/transition-operator/tests/node/canonical-golden.test.js
```

`OperatorPlan` contains typed semantic operations only. Backend commands,
Spotify preset IDs, arbitrary graphs, renderer state, and live playback
ownership are outside this crate and cannot be represented by the v1 model.
