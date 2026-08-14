# Spotify Automix oracle fixtures

These fixtures were captured from official Spotify Desktop 1.2.96.518 on
2026-08-14. They contain no credentials, cookies, authorization headers, or
tokens.

get_computed_transitions_2026-08-14.json is the native output oracle.
manifest-2026-08-14.json maps its eight pairs to the nine files under
tracks/. Each track file contains:

- canonical and playable identity;
- exact base64 protobuf payloads for extension kinds 6, 28, 217, 218, 219,
  and 222, including type URLs and safe response status/cache metadata;
- decoded descriptors, cuepoints, beats, vocal probabilities, mixability,
  and Audio Attributes v2;
- the reconstructed normalized downbeat vector consumed by native Automix;
- the Automix-relevant audio-analysis track, bar, and segment scalar fields.

The audio-analysis capture intentionally omits fingerprint strings and the
top-level beats/sections/tatums arrays, plus segment pitch/timbre vectors.
Native Automix's recovered path consumes track.duration, fade bounds, bars,
and segment loudness fields from this response. Omitted field names and
fingerprint-string lengths are recorded in every fixture.

For substituted tracks, native input identity is split:

- descriptors, cuepoints, and mixability use the canonical requested URI;
- beats, vocal activity, Audio Attributes v2, and audio analysis use the
  playable URI.

## Repeat the capture

Start Spotify with a DevTools port, then install the narrow capture helper:

~~~powershell
& .\tools\spotify-automix-oracle\cdp-eval.ps1 `
  -ExpressionPath .\tools\spotify-automix-oracle\capture_automix_inputs.js
~~~

Invoke only an explicitly selected track:

~~~javascript
spotifyAutomixInputCapture.captureTrack({
  canonicalTrackUri: "spotify:track:...",
  playableTrackUri: "spotify:track:...",
  expectedBeatsHash: "...",
})
~~~

The helper returns data to DevTools; it never writes files and never exports
authentication state. cdp-eval.ps1 is a generic localhost-only evaluator.

Run the offline consistency checks with:

~~~powershell
node tools\spotify-automix-oracle\validate_input_fixtures.js
~~~

The durable result for this capture is
input-validation-2026-08-14.json.
