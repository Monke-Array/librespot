# Live runtime diagnostics

`recorder.py` runs as the dedicated `spotifyd-diagnostics.service`, independently
of SSH and spotifyd. It reads the UID 1000 spotifyd user journal and Linux
telemetry. It never issues playback commands. Optional unavailable PSI is
reported explicitly. A timed-out `vcgencmd` sample is recorded and skipped
without restarting the recorder. `pidstat` follows spotifyd by process name
across restarts.

Storage: `/var/lib/spotifyd-diagnostics` (root, 0700). Each telemetry and packet
ring has 12 segments, rotated every minute or 4 MiB. At an ALSA EPIPE/underrun,
the recorder immediately copies the rings and copies again 45 seconds later.
Duplicate ALSA messages are coalesced. Only the newest three incidents remain;
total ring + incident storage is approximately 700 MiB maximum. A live pcap's
last packet may be incomplete. Traffic above the size cap shortens retained
history. A 256 MiB disk reserve stops collection; systemd limits restart retries.
The packet filter captures TCP headers on this host, excluding SSH, with a
128-byte snap length; payload is encrypted transport, not decoded Spotify data.
Treat all artifacts as private listening/network history.

Install the Python file at `/usr/local/lib/spotifyd-diagnostics/recorder.py` and
the unit at `/etc/systemd/system/spotifyd-diagnostics.service`, then run
`sudo systemctl daemon-reload` and
`sudo systemctl enable --now spotifyd-diagnostics`.
Disable overhead with `sudo systemctl disable --now spotifyd-diagnostics`.
This leaves evidence on disk; no spotifyd binary rollback is necessary.

The spotifyd user drop-in `spotifyd-runtime-debug.conf` enables existing DEBUG
logs plus `LIBRESPOT_RUNTIME_TRACE=1`. Runtime instrumentation is inert unless
that variable is exactly `1`. It records shared loader IDs/roles, session,
generation, command and queue identity, worker readiness and promotion. Slow
decoder/command/cancellation/promotion operations log above 10 ms; sink writes
above 250 ms. These thresholds are observational, never playback decisions.
Monotonic trace time is relative to first runtime event; journal JSON supplies
boot-relative monotonic time and boot ID for scheduler correlation.

`scheduler.bt` uses working sched tracepoints without BTF. Run a bounded window:
`sudo timeout 120 bpftrace scheduler.bt <spotifyd PID>`.
It reports runnable delay above 2 ms and off-CPU periods above 50 ms with kernel
stacks. Expected idle/network waits must not be labeled audio starvation.
Thread IDs are Linux TIDs; Rust trace thread IDs require journal/TID correlation.

`inspect_preview.py <journal.jsonl> <private-output-dir>` extracts baseline
multiline unknown-signal errors and new diagnostic preview records. It saves
binary payloads and a wire-type inventory. Decode embedded field 9 with
`protoc --decode=spotify.automix.proto.Transition automix_transition.proto`.
Do not infer absent fields or wire-number semantics without the client codec.

`automix_preview.proto` reconstructs the preview envelope from the installed
Spotify desktop client's generated `r_.encode`/`r_.decode` codec captured on
2026-09-12. Its field numbers and types match that codec. The first live
envelope, kept outside Git under `.codex/runtime-20260912/preview`, has SHA256
`b26a59943bf8da2e1331931a84ca1bba3ef90a09de86a685bb6d4f0cad4abf2c` and
matches the schema's wire types; structural consistency across more envelopes
is still unverified.

Verification: `python -m unittest discover -s tools/runtime-diagnostics -v`.
