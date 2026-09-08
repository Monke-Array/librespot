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
