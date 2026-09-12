import json
from pathlib import Path
import tempfile
import unittest
from recorder import Recorder, Ring


class RetentionTests(unittest.TestCase):
    def test_ring_retains_newest_and_bounds_storage(self):
        with tempfile.TemporaryDirectory() as root:
            ring = Ring(root, slots=3, limit=120)
            for n in range(40):
                ring.write(dict(n=n, data="x" * 60))
            ring.file.close()
            files = sorted(Path(root).glob("*.jsonl"))
            self.assertEqual(len(files), 3)
            self.assertLessEqual(sum(p.stat().st_size for p in files), 360)
            self.assertEqual(json.loads(files[-1].read_text())["n"], 39)

    def test_trigger_preserves_before_coalesces_and_bounds_incidents(self):
        with tempfile.TemporaryDirectory() as root:
            recorder = Recorder(root)
            recorder.record("telemetry", "before")
            (recorder.pcap / "test.pcapng").write_bytes(b"packet-evidence")
            for _ in range(5):
                recorder.pending = None
                recorder.last_incident = -1000
                recorder.record("journal", "snd_pcm_writei failed Broken pipe")
                event = recorder.pending[1]
                recorder.record("journal", "underrun occurred")
                self.assertEqual(recorder.pending[1], event)
                self.assertEqual((event / "before/pcap/test.pcapng").read_bytes(), b"packet-evidence")
            self.assertEqual(len(list(recorder.incidents.iterdir())), 3)
            recorder.ring.file.close()


if __name__ == "__main__":
    unittest.main()
