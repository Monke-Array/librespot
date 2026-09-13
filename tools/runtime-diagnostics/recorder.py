#!/usr/bin/env python3
"""Bounded, playback-independent incident recorder. Linux/Python stdlib only."""
import argparse
import glob
import json
import os
from pathlib import Path
import re
import selectors
import shutil
import signal
import subprocess
import time

MIB = 1024 * 1024
TRIGGER = re.compile(r"snd_pcm_writei.*Broken pipe|underrun occurred", re.I)


class Ring:
    def __init__(self, directory, slots=12, limit=4 * MIB):
        self.directory = Path(directory)
        self.directory.mkdir(parents=True, exist_ok=True)
        self.slots, self.limit = slots, limit
        self.file = None
        self.opened = 0

    def write(self, value):
        data = (json.dumps(value, ensure_ascii=True) + "\n").encode()
        now = time.monotonic()
        if self.file is None or self.file.tell() + len(data) > self.limit or now - self.opened >= 60:
            if self.file:
                self.file.close()
            files = sorted(self.directory.glob("*.jsonl"))
            for old in files[:max(0, len(files) - self.slots + 1)]:
                old.unlink()
            self.file = (self.directory / f"{time.time_ns()}.jsonl").open("ab", buffering=0)
            self.opened = now
        self.file.write(data)


class Recorder:
    def __init__(self, root):
        self.root = Path(root).resolve()
        self.root.mkdir(parents=True, exist_ok=True, mode=0o700)
        self.ring = Ring(self.root / "ring")
        self.pcap = self.root / "pcap"
        self.pcap.mkdir(exist_ok=True)
        self.incidents = self.root / "incidents"
        self.incidents.mkdir(exist_ok=True)
        self.selector = selectors.DefaultSelector()
        self.children = {}
        self.pending = None
        self.stopping = False
        self.last_incident = -1000
        self.journal_cursor_file = self.root / "journal.cursor"
        try:
            self.journal_cursor = self.journal_cursor_file.read_text().strip() or None
        except OSError:
            self.journal_cursor = None

    def record(self, source, data):
        value = dict(wall_ns=time.time_ns(), mono_ns=time.monotonic_ns(), source=source, data=data)
        self.ring.write(value)
        if source == "journal":
            try:
                cursor = json.loads(data).get("__CURSOR")
            except (AttributeError, json.JSONDecodeError):
                cursor = None
            if isinstance(cursor, str) and cursor:
                self.journal_cursor = cursor
            if TRIGGER.search(data):
                self.persist_journal_cursor()
                self.trigger(value)

    def persist_journal_cursor(self):
        if not self.journal_cursor:
            return
        temporary = self.journal_cursor_file.with_name(self.journal_cursor_file.name + ".tmp")
        temporary.write_text(self.journal_cursor + "\n")
        os.replace(temporary, self.journal_cursor_file)

    def snapshot(self, target):
        target.mkdir(exist_ok=True)
        for directory in [self.root / "ring", self.pcap]:
            dest = target / directory.name
            dest.mkdir(exist_ok=True)
            # Files are bounded. A live pcap copy may end in a partial packet;
            # closed segments are intact. Never pause playback or packet capture.
            for src in sorted(directory.iterdir()):
                if src.is_file():
                    try:
                        shutil.copyfile(src, dest / src.name)
                    except FileNotFoundError:
                        pass  # ring rotated during the copy

    def trigger(self, value):
        now = time.monotonic()
        if self.pending or now - self.last_incident < 60:
            return
        self.last_incident = now
        existing = sorted(p for p in self.incidents.iterdir() if p.is_dir())
        for old in existing[:-2]:
            shutil.rmtree(old)  # only this recorder's validated incident children
        event = self.incidents / str(value["wall_ns"])
        event.mkdir()
        (event / "trigger.json").write_text(json.dumps(value, indent=2))
        self.snapshot(event / "before")
        self.pending = (now + 45, event)

    def start(self, name, args):
        proc = subprocess.Popen(args, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, bufsize=0)
        os.set_blocking(proc.stdout.fileno(), False)
        self.selector.register(proc.stdout, selectors.EVENT_READ, [name, proc, b""])
        self.children[name] = proc
        self.record("collector", dict(start=name, pid=proc.pid, args=args))

    def journal_command(self):
        command = ["journalctl", "_UID=1000", "_SYSTEMD_USER_UNIT=spotifyd.service"]
        if self.journal_cursor:
            command.append(f"--after-cursor={self.journal_cursor}")
        else:
            command.append("--since=-10min")
        return [*command, "-f", "-o", "json", "--no-pager"]

    def proc_sample(self, tick):
        paths = glob.glob("/proc/pressure/*") + glob.glob("/proc/asound/card*/pcm*p/sub*/status")
        paths += ["/proc/stat", "/proc/meminfo", "/proc/diskstats", "/proc/interrupts", "/proc/softirqs"]
        if not glob.glob("/proc/pressure/*") and tick == 0:
            self.record("unavailable", "/proc/pressure (kernel PSI unavailable)")
        for candidate in Path("/proc").glob("[0-9]*/comm"):
            try:
                if candidate.read_text().strip() != "spotifyd":
                    continue
                proc = candidate.parent
                paths += [str(proc / p) for p in ["status", "io", "schedstat"]]
                paths += [str(p) for p in (proc / "task").glob("*/schedstat")]
                paths += [str(p) for p in (proc / "task").glob("*/comm")]
                paths += [str(p) for p in (proc / "task").glob("*/wchan")]
            except (OSError, ProcessLookupError):
                continue
        for path in paths:
            try:
                self.record(path, Path(path).read_text()[:65536])
            except OSError as error:
                self.record("unavailable", dict(path=path, error=str(error)))
        if tick % 30 == 0:
            for arg in ["get_throttled", "measure_temp", "measure_clock arm"]:
                try:
                    result = subprocess.run(
                        ["vcgencmd", *arg.split()], capture_output=True, text=True, timeout=2
                    )
                    self.record("pi", dict(command=arg, output=result.stdout, error=result.stderr))
                except subprocess.TimeoutExpired as error:
                    self.record(
                        "pi",
                        dict(command=arg, output="", error=f"timed out after {error.timeout} seconds"),
                    )
            self.record("disk_free", shutil.disk_usage(self.root).free)
            self.persist_journal_cursor()

    def run(self):
        self.start("journal", self.journal_command())
        self.start("vmstat", ["stdbuf", "-oL", "vmstat", "-t", "1"])
        self.start("iostat", ["stdbuf", "-oL", "iostat", "-xz", "-t", "1"])
        self.start("pidstat", ["stdbuf", "-oL", "pidstat", "-t", "-u", "-r", "-d", "-w", "-h", "-G", "spotifyd", "1"])
        self.start("dumpcap", ["dumpcap", "-q", "-i", "any", "-s", "128", "-f", "tcp and not port 22",
                               "-b", "duration:60", "-b", "filesize:4096", "-b", "files:12",
                               "-w", str(self.pcap / "network.pcapng")])
        sample_at, tick = time.monotonic(), 0
        try:
            while not self.stopping:
                for key, _ in self.selector.select(timeout=0.2):
                    name, proc, previous = key.data
                    chunk = os.read(key.fileobj.fileno(), 65536)
                    if not chunk:
                        self.selector.unregister(key.fileobj)
                        self.record("collector", dict(exited=name, status=proc.poll()))
                        # systemd restarts the entire group on a failed source.
                        raise RuntimeError(f"collector source exited: {name}")
                    lines = (previous + chunk).split(b"\n")
                    key.data[2] = lines.pop()[-262144:]
                    for line in lines:
                        self.record(name, line.decode(errors="replace"))
                now = time.monotonic()
                if now >= sample_at:
                    self.proc_sample(tick)
                    tick += 1
                    sample_at = now + 1
                    if shutil.disk_usage(self.root).free < 256 * MIB:
                        raise RuntimeError("disk reserve reached; recorder stopping")
                if self.pending and now >= self.pending[0]:
                    event = self.pending[1]
                    self.snapshot(event / "after")
                    (event / "complete.json").write_text(json.dumps(dict(wall_ns=time.time_ns())))
                    self.pending = None
        finally:
            self.persist_journal_cursor()
            for proc in self.children.values():
                if proc.poll() is None:
                    proc.terminate()
            for proc in self.children.values():
                try:
                    proc.wait(timeout=3)
                except subprocess.TimeoutExpired:
                    proc.kill()


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", default="/var/lib/spotifyd-diagnostics")
    args = parser.parse_args()
    os.umask(0o077)
    recorder = Recorder(args.root)
    signal.signal(signal.SIGTERM, lambda *_: setattr(recorder, "stopping", True))
    signal.signal(signal.SIGINT, lambda *_: setattr(recorder, "stopping", True))
    recorder.run()
