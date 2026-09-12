# Live runtime investigation

Goal: explain recorded xruns and queue mismatches with command/worker evidence,
and inventory automix-preview without speculative playback changes.

The user's nine-phase session specification is the execution contract. Work
inline on codex/m3a-live-auto-metadata; U-01 owns edits and Git. Never cargo
update, push, change generator behavior, or discard rollback binaries.

- [x] Verify clean 87f2300 source and deployed hash; inspect service/resources.
- [x] Probe diagnostic tools. bpftrace scheduler tracepoints work; PSI absent;
  perf absent from PATH; BCC runqlat reports missing BTF.
- [ ] Add tools/runtime-diagnostics bounded recorder and synthetic retention test.
  Use systemd, timestamped rotating telemetry, dumpcap ring, event snapshots.
- [ ] Trace dealer command -> SPIRC edge -> player generation -> loader/worker
  -> promotion. Add diagnostics only where existing logs cannot prove ownership.
- [ ] Decode multiple preview captures against protocol/proto definitions and
  client codecs when available; document evidence and unknowns separately.
- [ ] Reproduce any confirmed defect deterministically; demonstrate failing
  regression before minimal fix and passing related tests. Separate commits.
- [ ] Run fmt, workspace check, playback/connect suites, touched-crate clippy
  with documented existing exceptions, and diff check.
- [ ] Build exact ARM candidate using existing target; preserve rollback,
  deploy, check hash, and collect repeated live transition evidence.
- [ ] Write report and update CODEX_STATE with one next action; leave bounded
  recorder enabled if rare failure remains unresolved.

Recorder design: dedicated root system service reads the amogus spotifyd journal,
subprocess telemetry and /proc. Store only under /var/lib/spotifyd-diagnostics,
mode 0700, with size and age bounded rings and a bounded incident count. Trigger
on EPIPE/underrun; preserve pre-event data immediately and finalize after 45 s.
Recorder never commands playback. Failures of optional sources are recorded.
Stop/disable its service to remove overhead; no playback rollback is required.
