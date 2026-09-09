# Bounded offline WAV input

Issue [#734](https://github.com/yan-h/harmonigraph/issues/734) replaces whole-file encoded and decoded allocations with one concrete seekable WAV reader.
Supported encodings remain unsigned PCM8, signed PCM16/24/32, and float32, including their existing WAVE_FORMAT_EXTENSIBLE handling.
No saved state or format compatibility changes are involved.

## Ownership and timing

`wav::Audio` owns a 64 KiB `BufReader<File>`, an encoded buffer and a decoded buffer.
The latter two hold at most 16,384 samples each, or one complete channel frame when the channel count exceeds that target.
For ordinary float32 stereo this is 192 KiB of raw-input buffer capacity per open source.
The bound depends on format width/channel count, never source duration or a requested logical range.
Chunks contain complete interleaved frames, and callbacks borrow their contents only until the next refill.
There is no prefetch thread, whole-file fast path, cache hierarchy or media framework.

Header parsing records the selected data chunk's absolute byte range and clamps its declared length to the physical file.
Only complete channel frames are decoded, preserving the old interrupted-bounce behavior.
The file must remain unchanged while open;
a subsequent seek/read failure aborts alignment or rendering rather than completing a partial result.
Sequential ranges reuse the buffered cursor, and discontinuities seek to an absolute source frame.

The consumers retain distinct measurement boundaries:

- Alignment scans whole sources into the existing small onset envelopes.
The clean source is reduced once, then recording and MIDI correlations reuse that sequence.
Energy accumulates sample by sample across original channels, before signed cancellation can occur.
The 5 ms envelope boundaries remain defined by source time, including fractional frame counts at 44.1 kHz.
- `WholeSong::precompute` owns the existing absolute hop grid, pre-roll and far-edge margin.
A supplied feeder pushes each requested absolute range into the same `ChannelBank`, possibly in several physical chunks, before measurement occurs.
The UI imports no offline WAV type.
- Frame analysis updates its clock anchor once from the logical batch's full frame count and actual newest source frame.
`AudioSpectrum::push_samples` and the chunked entry point share the same FFT, display smoothing and history loop.
Physical I/O chunks do not add anchor smoothing steps.

ffmpeg continues receiving the source path directly.
Its input pass is independent of analysis.
Onset envelopes, correlation arrays, notes, spectrogram storage and GPU/encoder resources can still grow with the work requested;
this change bounds raw-input ownership, not total export memory.

## Measurement method

Measured on 2026-09-09, Apple M1 Pro, 16 GB, macOS 26.6.2, release builds.
The immutable baseline is `4d2092ab71f15b4268770bbce45ac1c7ec8cdd81`.
The implementation is `6fac3d29eb8e9652881b5444bc40eb670123a8cd`.
The raw reader and short/excerpt timing binaries were built before committing, before the final alignment reuse factoring;
their reader and `--align off` render paths are unchanged in this commit.
The complete-export comparison uses the release binary rebuilt from this commit, and final alignment I/O is checked separately.

Inputs are materialized float32 stereo WAVs, not sparse files:
a 440 Hz sine with a 3 Hz amplitude envelope, right channel opposite in polarity.
The files contain 5 seconds at 48 kHz, 60 seconds at 192 kHz, and 600 seconds at 192 kHz.
The last source is 921,600,044 bytes including its 44-byte header.
A small take holds an A4 note from 0 to 4.5 seconds.
These are synthetic audio/scene workloads, not a representative complex musical take.

A scratch standalone Rust probe compiled each revision's production WAV module with `rustc -O`.
Baseline opens/decodes the source and observes the decoded length and final sample;
after opens/scans all frames through the bounded reader and observes the same values.
`/usr/bin/time -l` measures wall time and maximum resident set size.
I/O tracing runs separately through a scratch macOS `read`/`lseek` interposer restricted to the fixture WAV paths.
The interposer calls `F_GETPATH` to identify inputs, so its elapsed time is deliberately excluded from performance comparisons.
Reported read bytes are application read calls, not physical disk traffic;
OS caching was not flushed.

Exports use the normal renderer, GPU and ffmpeg, at 640×360 with `--align off --playhead`:

| Scenario | Source | Render interval | FPS | Video frames |
| --- | --- | --- | --- | --- |
| Short | 5 s, 48 kHz | 0–5 s | 30 | 150 |
| Late excerpt | 600 s, 192 kHz | 300–305 s | 2 | 10 |
| Full source | 600 s, 192 kHz | 0–600 s | 0.5 | 300 |

The full-source case exposes repeated sequential decoding passes and very large logical per-frame batches.
Its 300-frame count does not establish normal-frame-rate export throughput.
The late excerpt demonstrates bounded window access, while the short case includes a normal video frame rate.
Alignment correctness is tested separately;
these export timing cases do not measure correlation cost.

Short and excerpt timings use three paired baseline/after runs without tracing or concurrent compilation.
The complete-export case uses two pairs, reversing their execution order for the second pair.
Initial exploratory runs overlapped compilation and are excluded from the export summary.
An extreme 0.05 fps probe also exposed pre-existing muxed-audio truncation ([#799](https://github.com/yan-h/harmonigraph/issues/799));
its timings are excluded, and the accepted full-source case verifies both video and audio duration.
Commands and source generation are reproducible with the following recipe, using each revision's release renderer:

```python
import math, pathlib, struct
root = pathlib.Path('/tmp/bounded-audio')
root.mkdir(exist_ok=True)
for name, rate, seconds in [('short', 48000, 5), ('medium', 192000, 60), ('long', 192000, 600)]:
    data = bytearray()
    for i in range(rate):
        x = 0.2 * math.sin(2 * math.pi * 440 * i / rate) * (0.5 + 0.5 * math.cos(2 * math.pi * 3 * i / rate))
        data += struct.pack('<ff', x, -x)
    size = rate * seconds * 8
    with (root / (name + '.wav')).open('wb') as f:
        f.write(b'RIFF' + struct.pack('<I', 36 + size) + b'WAVEfmt ' + struct.pack('<IHHIIHH', 16, 3, 2, rate, rate * 8, 8, 32) + b'data' + struct.pack('<I', size))
        for _ in range(seconds):
            f.write(data)
(root / 'probe.take').write_text('Header((version:5))\nNote((t:0.0,source:1,channel:0,note:69,kind:On(velocity:0.8)))\nNote((t:4.5,source:1,channel:0,note:69,kind:Off))\n')
```

```sh
/usr/bin/time -l target/release/harmonigraph-offline /tmp/bounded-audio/probe.take \
  --audio /tmp/bounded-audio/long.wav --align off --size 640x360 \
  --fps 0.5 --start 0 --end 600 --playhead --out /tmp/bounded-audio/full.mp4
```

Use the table's source, FPS and interval for the other scenarios.

## Results

Standalone raw-reader probes, one uninstrumented run per source after fixture generation:

| Input | Baseline peak RSS | Bounded peak RSS | Baseline wall | Bounded wall |
| --- | --- | --- | --- | --- |
| 5 s / 48 kHz | 5.44 MB | 1.74 MB | 0.01 s | <0.01 s |
| 60 s / 192 kHz | 185.93 MB | 1.75 MB | 0.12 s | 0.05 s |
| 600 s / 192 kHz | 1,844.81 MB | 1.74 MB | 0.97 s | 0.40 s |

MB means decimal megabytes.
The fixed buffer capacities and stable probe RSS demonstrate bounded raw-input memory across increasing duration.
These probe timings include opening/decoding and are not export timings.

End-to-end export results (short/excerpt: three-run medians;
full source: two paired runs):

| Scenario | Baseline peak RSS | Bounded peak RSS | Baseline wall (range) | Bounded wall (range) |
| --- | --- | --- | --- | --- |
| Short | 133.45 MB | 134.37 MB | 0.83 s (0.79–0.87) | 0.85 s (0.78–0.86) |
| Late excerpt | 1,716.90 MB | 70.71 MB | 1.78 s (1.58–1.93) | 0.70 s (0.69–0.70) |
| Full source, pair 1 | 1,796.85 MB | 159.96 MB | 60.16 s | 59.85 s |
| Full source, pair 2 (reverse order) | 1,834.17 MB | 152.57 MB | 60.65 s | 59.40 s |

Short-export times overlap;
the complete-export timing difference is small and does not establish a general throughput gain.
Every accepted full-source output contains 300 video frames and 600 seconds of both video and audio.
The late excerpt avoids loading most of its source and improves substantially in this synthetic case.
Peak RSS is the value reported by `/usr/bin/time -l`, not an attribution of every allocation in the renderer/encoder process tree.

The raw-reader trace reads exactly 1,920,044 / 921,600,044 bytes for the short/long source at both revisions.
The old reader makes two reads and zero seeks;
the bounded reader makes 30 / 14,063 reads and zero seeks, using 64 KiB buffered input.
**Zero seeks and one pass describe that standalone sequential probe only.** Actual exports have the following renderer-process I/O:

| Export scenario | Baseline bytes / reads / seeks | Bounded bytes / reads / seeks |
| --- | --- | --- |
| Own short audio, scrolling | 1,920,044 / 2 / 0 | 1,920,044 / 30 / 0 |
| Own short audio, playhead | 1,920,044 / 2 / 0 | 3,840,044 / 60 / 1 |
| Replacement aligned to recording | 3,840,088 / 4 / 0 | 5,760,088 / 90 / 1 |
| Silent reference, MIDI fallback | 3,840,088 / 4 / 0 | 5,760,088 / 90 / 1 |
| Late excerpt, playhead | 921,600,044 / 2 / 0 | 14,811,136 / 226 / 2 |

The recording-match case uses the same short file as recording and replacement, with offset 0 and confidence 1.00.
The fallback case uses a silent recording plus a deterministic irregular click track and corresponding MIDI note-ons;
baseline and final code both report offset 0 and confidence 0.84.
Before factoring onset reuse, the bounded implementation's fallback read 7,680,088 bytes in 120 reads with two seeks.
Reusing the clean onset sequence removes that extra pass without retaining decoded audio.
The fallback with no confident MIDI match also scans the clean source only once before rendering.

ffmpeg runs in a separate traced PID and has unchanged I/O:
1,920,044 bytes / 61 reads / one seek for each short scenario;
15,204,396 bytes / 117 reads / two seeks for the late excerpt.
The renderer's larger full-window read counts are the explicit tradeoff for discarding raw samples between phases, not per-video-frame replay from source zero.

## Verification

Existing scrolling timing tests retain their sample grid across multiple FPS values, late starts and partial final batches.
The existing alignment polarity/different-rate regressions now use real temporary WAVs.
Additional coverage crosses physical buffers within large ranges, within an alignment energy bin, and within a huge WholeSong hop.
Format checks cover PCM8/16/24/32 and float32, extensible headers, odd metadata padding and truncated channel frames.
A file truncated after opening returns an I/O error.

The logical-batch test compares every history timestamp, every column byte, displayed spectrum, anchor and frame count against `push_samples` over awkward chunk sizes, both ordinary and very large time origins.
The WholeSong feeder test checks contiguous bounded margins, empty windows, huge logical hops and exact column equivalence, plus error propagation.
The fallback regression confirms the reused onset sequence gives exactly the direct MIDI alignment's offset and confidence.

All five existing offline spectral goldens remain unchanged, without blessing.
Decoded video frames from the short, late-excerpt and full-source exports are SHA-256 identical between baseline and bounded input.
The accepted full-source decoded-video hash is `1fb611b870836c01308b9f061f1cebca1383ab2af4d625c99fd250d7fa2b697b`.
Its decoded-audio hash also matches between baseline and bounded input.
Independent read-only reviews also compared 280 baseline/new WAV cases across formats, channel counts, chunk orders, seeks and clamps with exact results.

Full local `./ci.sh` passed, including workspace Clippy/tests, isolated plugin/render checks, vendored tests, rustdoc and script gates.
Both release packages were built from the implementation commit.
