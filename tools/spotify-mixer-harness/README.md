# Spotify Mixer Harness

Development-only harness for collecting official Spotify Desktop Mixer evidence
and comparing it with local spotifyd/librespot behavior.

The harness uses only Node.js built-ins and the existing
`tools/spotify-automix-oracle/cdp-eval.ps1` CDP bridge. It stores sanitized
captures under `tools/spotify-mixer-harness/corpus`.

## Safety

- Does not write credentials, cookies, authorization headers, or token-shaped
  fields to corpus output.
- Does not like, unlike, delete, or modify playlists or library items.
- Keeps in-page media elements muted during metadata snapshots.
- Does not change production DSP behavior.

## Typical Run

Start or attach official Spotify Desktop with a DevTools port:

```powershell
node tools\spotify-mixer-harness\bin\mixer-harness.js install-oracle --start-spotify --port 9222
```

Use Spotify Desktop to play a Mixer-enabled context. The harness discovers the
mounted official XPUI `PlayerAPI` through CDP and uses it for direct control
when available:

```powershell
node tools\spotify-mixer-harness\bin\mixer-harness.js control-status --port 9222
node tools\spotify-mixer-harness\bin\mixer-harness.js official-control --action mute --port 9222
node tools\spotify-mixer-harness\bin\mixer-harness.js snapshot-official --port 9222
node tools\spotify-mixer-harness\bin\mixer-harness.js sample-official --port 9222 --drive-current-context --duration-ms 180000 --interval-ms 5000
```

Useful playback commands:

```powershell
node tools\spotify-mixer-harness\bin\mixer-harness.js play-context --port 9222 --context-uri spotify:playlist:...
node tools\spotify-mixer-harness\bin\mixer-harness.js play-pair --port 9222
node tools\spotify-mixer-harness\bin\mixer-harness.js next --port 9222
node tools\spotify-mixer-harness\bin\mixer-harness.js seek --port 9222 --position-ms 180000
node tools\spotify-mixer-harness\bin\mixer-harness.js transfer --port 9222 --device-name "spotifyd-transition DEV"
```

`play-pair` defaults to the known development A/B fixture only inside the
harness. It is not used by production librespot code. For broad Mixer evidence,
prefer `sample-official --drive-current-context` after an official Mixer-enabled
context is active; it snapshots current/next materialized rows, skips forward
through that context, and repeats.

Build and run spotifyd from its checkout:

```powershell
node tools\spotify-mixer-harness\bin\mixer-harness.js build-spotifyd --spotifyd-root C:\Users\janni\Desktop\Projects\spotifyd
node tools\spotify-mixer-harness\bin\mixer-harness.js run-spotifyd --spotifyd-root C:\Users\janni\Desktop\Projects\spotifyd --duration-ms 60000 --device-name "spotifyd-transition DEV"
```

`run-spotifyd` sets `LIBRESPOT_DEV_BYPASS_MATERIALIZED_EQ=1` and
`LIBRESPOT_DEV_BYPASS_MATERIALIZED_FILTER=1` for that process.
Use Spotify Desktop to transfer playback to `spotifyd-transition DEV` when XPUI
device transfer is not available.

Classify and compare:

```powershell
node tools\spotify-mixer-harness\bin\mixer-harness.js classify-corpus --dev-eq-bypass --dev-filter-bypass
node tools\spotify-mixer-harness\bin\mixer-harness.js compare --spotifyd-run tools\spotify-mixer-harness\corpus\runs\spotifyd-...json --dev-eq-bypass --dev-filter-bypass
```

## Coverage Model

Transitions are deduplicated by a stable DSP signature built from materialized
timing, volume, speed, EQ, filter, FX, preset, and unknown `audio.*` fields.
Song pair URIs and `automix.transition_uri` do not affect the signature hash.

The report classifies each transition as:

- `SUPPORTED`
- `DEV-EQ-BYPASS`
- `DEV-FILTER-BYPASS`
- `DEV-EQ-FILTER-BYPASS`
- `UNSUPPORTED-EQ`
- `UNSUPPORTED-FILTER`
- `UNSUPPORTED-FX`
- `UNSUPPORTED-TIMING`
- `UNSUPPORTED-SPEED`
- `UNKNOWN`
- `RUNTIME-FAILURE`
