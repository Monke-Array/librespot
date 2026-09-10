# Offline Vocal Separator Feasibility

Status: bounded feasibility only. No separator is accepted as the production
offline extractor until independent vocal-activity validation passes.

## Frozen experiment boundary

- Host: S-01, Linux x86-64, 62 GiB RAM.
- GPU: not used. The installed NVIDIA device was visible on PCI, but the driver
  was unavailable to `nvidia-smi` during this run.
- Environment manager: uv 0.12.12.
- Runtime: CPython 3.10.21.
- Canonical input: complete stereo s16le 44.1 kHz WAV, processed one track per
  process. The three private source identities and audio stay outside Git.
- Threads: four intra-op/OMP threads and one inter-op thread.

## Spleeter feasibility candidate

- Packages: Spleeter 2.4.2, TensorFlow 2.12.1, NumPy 1.24.3, SciPy 1.15.3.
- Model: `spleeter:2stems`, upstream v1.4.0 archive.
- Main model-data SHA-256:
  `7747f9fd2c782306dbec1504360fbb645a097a48446f23486c3ff9c89bc11788`.
- Representative 245.13 s singing track: 13.86 s inference, 0.0565 wall/audio
  factor, 4,042,080 KiB peak RSS (3.85 GiB).
- TensorFlow deterministic-op mode was not usable without modifying Spleeter's
  estimator graph because prediction construction includes an unseeded dropout
  random op. A seeded ordinary CPU run completed, but was not promoted.

Decision: reject Spleeter as the main candidate. Its one-track peak RSS
materially exceeds the approved approximately 2 GiB ceiling, independent of
its excellent throughput.

## HTDemucs challenger

- Packages: Demucs 4.0.1, PyTorch 2.2.2+cpu, TorchAudio 2.2.2+cpu, NumPy
  1.26.4.
- Model/settings: `htdemucs`, CPU, shifts 0, split mode, 7.8 s segments, 0.25
  overlap, one worker.
- Model SHA-256:
  `8726e21a993978c7ba086d3872e7608d7d5bfca646ca4aca459ffda844faa8b4`.
- PyTorch deterministic algorithms were enabled and the random seed was fixed.

| Challenge case | Audio s | Inference s | Wall/audio | Peak RSS KiB |
| --- | ---: | ---: | ---: | ---: |
| singing, run 1 | 245.13 | 75.42 | 0.3077 | 1,844,200 |
| singing, run 2 | 245.13 | 74.04 | 0.3020 | 1,882,804 |
| dense rap | 337.95 | 102.62 | 0.3037 | 2,030,896 |
| transient-heavy instrumental | 208.36 | 64.19 | 0.3081 | 1,725,356 |

Both singing runs produced identical float32 vocal-stem SHA-256
`a89b03a8c9595a50713cd46d1fe7045df9f4fd6bd91939dac778899169658eb1`.
The instrumental challenge had median vocal/mix RMS ratio 0.00337 and p95
0.23774, while singing and rap medians were 0.44673 and 0.32590. These values
only characterize separator leakage; they are not calibrated activity labels.

Decision: HTDemucs passes the bounded speed, memory, and repeatability
feasibility gate. It remains a challenger, not an accepted detector. The
upstream code distribution is MIT-licensed; an explicit separate license for
the downloaded pretrained weight is not identified in the frozen package
metadata, so model-weight deployment terms must be clarified before a
permanent extractor freeze.

## Validation gate

Production integration is intentionally paused before detector calibration.
The approved gates require audible-human vocal-presence ground truth, including
separate singing, rap, and spoken recall and 100/250 ms boundary-error
measurements. Separator output, synchronized lyrics, or Spotify metadata would
be circular or proxy labels and are not substituted.

The public Jamendo Voice Activity Detection corpus is an appropriate independent
source for overall sung/spoken versus no-voice validation, but its only archive
endpoint returned HTTP 403 for unusual network traffic during this run. It also
does not label singing, rap, and spoken delivery as separate modalities. A
small track-disjoint private annotation supplement is therefore still required
for the approved modality-specific gates. No detector threshold was frozen and
no M2 coverage was inspected.
