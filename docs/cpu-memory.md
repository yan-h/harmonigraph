# CPU memory

What Harmonigraph keeps on the CPU heap, by owner, and how to measure it.
`docs/gpu-memory.md` is the GPU side.
Measured for #1511 on 2026-10-19, an M5 Pro, build `main @d5db292c` in Bitwig and the same commit in the standalone harness.

## Our heap is the overlay's number, not `footprint`'s

The Performance overlay's `memory` row reads `N MB heap + M GPU`, ours alone;
the breakdown adds the host process's footprint as `host mem`.
`heap` is what this binary's global allocator has handed out and not taken back (`harmonigraph_perf::heap`), in MiB stepped to 10.
It counts nothing another plugin allocates,
and nothing the system frameworks allocate on our behalf.

`footprint` charges the whole plug-in host.
With **Plug-in hosting: by vendor**, Harmonigraph shares its host process with every other plugin of the same vendor (otonal),
and with **together** it shares it with everything.
`vmmap -summary <pid>` shows how much of what `footprint` calls Malloc Large is not resident at all:
in the measured host, 148 MB of large blocks were 15 MB resident and 124 MB swapped or compressed, all of it charged as dirty.

In the measured host, the overlay's heap read 80 MB against 258 MB allocated in the process's malloc zones (`vmmap -summary`).
The other ~175 MB is otonal and the frameworks.
Our history is all 8 KB blocks, and everything else we construct is 14.9 MiB,
so at least 150 MiB of the host's 173 MiB of blocks over 256 KB are someone else's.
They come in threes — 3 × 19.0 MiB, 3 × 5.3 MiB, 3 × 5.0 MiB, 3 × 816 KiB —
the shape of otonal's snapshots, which it triple-buffers at construction (`otonal-plugin/src/snapshot.rs`).

## Harmonigraph's heap by owner

One Harmonigraph instance after more than ten minutes of audio.
Audio-side rows were measured in a release test, editor rows by stack in the standalone, and the history by both;
the total is checked against Bitwig's overlay below.

| owner | MiB | what scales it | when | freed |
|---|---:|---|---|---|
| Spectrogram history (`SpectrumHistory`) | 53.6 | 7,336 columns × 7,656 B at the 610 s cap | grows with audio, editor open or not | never |
| Recorder event ring (`harmonigraph_record`, `TAKE_RING_CAPACITY`) | 5.5 | 65,536 × 88 B | plugin construction | never |
| Recorder audio ring (`AUDIO_RING_CAPACITY` there) | 4.0 | 2²⁰ × `f32` | plugin construction | never |
| Recorder publication lanes, take and display | 2.1 | 2 × (4,096 items + 20 baselines) | plugin construction | never |
| Tuning hub (`tuning::hub::Hub`) | 1.7 | per-row voice state, one `Tune`'s 8,192-entry delay line | plugin construction | never |
| Spectrum ingress (`audio_ingress`) | 1.2 | 131,072-sample ring, its drain scratch, 4,096 block descriptors | plugin construction | never |
| Spectrogram slab grid, per visible spectrogram (`SlabGrid`) | 4.1 | up to 1,032 slabs of 3,828 B, each in a 4 KB block | first drawn | never; it lives in `SharedState` |
| egui font atlas and context | 2 | glyphs drawn | editor open | with the window |
| Other editor state: mark atlas, lattice node buffers, egui-wgpu staging | the rest, ~5 | window size, visible nodes | editor open | partly |
| Each Harmonigraph Tune instance | 0.6 | its 8,192-entry delay line | its construction | never |

The constructed plugin alone is 14.9 MiB, measured by reading `RUST_HEAP` around `Harmonigraph::default()` in a release test.
History plus construction is 68.5 MiB,
and the overlay's 80 (75–85) leaves about 7–17 for the editor.
The editor's "the rest" is what that leaves after its measured rows, and is not measured by itself.

**The history is three quarters of it.** It is 7,336 boxed columns of 3,828 bfloat16 bins.
macOS rounds each 7,656-byte request up to 8,192 bytes, so it holds 60 MB resident for 53.6 MiB requested.
The tiers reach (`SpectrumHistory`):
2,048 columns at 8 ms, then 1,024 at each doubling, so 16 s, 33 s, 66 s, 131 s, 262 s, 524 s,
and 168 columns of the last tier to reach 610 s.
It fills only while audio arrives, at 125 columns a second at first and slower as the tiers merge,
and reaches the cap at 610 s.

## Growth

The standalone in the saved Bitwig look, window visible and untouched, sampled every 30 s (`footprint`, every other sample shown):

| s | Malloc Small (MB) |
|---:|---:|
| 30 | 41 |
| 60 | 58 |
| 120 | 69 |
| 180 | 75 |
| 300 | 80 |
| 420 | 86 |
| 540 | 89 |
| 600 | 91 |
| 660 | 92 |
| 870 | 93 |

Malloc Large stayed at zero throughout.
**Small allocations plateau** once the history reaches its 610 s cap:
92 MB from 660 s to 840 s, against 0.6 MB/s in the first minute.
At 14 minutes the heap held 7,130 live history columns, 58 MB in 8 KB blocks.
That is 3% short of the 7,336 the cap allows,
because the harness takes audio only while it draws and three samples caught the window not drawing (graphics at 57 MB rather than 209).

Every live 8,192-byte block sampled with `malloc_history` (21 of 21) was a spectrogram column from `SpectrogramColumn::from_power`.
So the steady growth reported in #1511 is the history filling, and it stops at the cap.

## Methods

- **Attribute by stack in the standalone.**
  Launch it with `MallocStackLogging=lite`, then `heap <pid> -sortBySize -showSizes` for the census and `heap <pid> -addresses all` for addresses.
  `malloc_history <pid> <address>` prints the region's events;
  read the line that starts with `ALLOC`, because the `VM_ALLOC` line beside it is whoever mapped the 4 MB segment the block sits in.
- **Keep the window visible.**
  The harness feeds its analyzer from the frame loop, and macOS stops drawing a hidden window, so a hidden run measures nothing.
  A resize leaves its freed transient in `footprint`'s Malloc Large until the system takes it back;
  `heap` shows only what is live.
- **Seed the standalone with a session's look.**
  The harness stores the same `ui-state` RON the plugin saves, under the key `harmonigraph-ui-state` in `$HOME/Library/Application Support/Harmonigraph-—-dev-harness/app.ron`.
  `read-plugin-state.py`'s `find_states` reads it out of a `.bwproject`.
- **Measure audio-side construction in a release test.**
  The release test binary counts through `RUST_HEAP` too, so the live count around a constructor is its allocation.
  A dropped recorder channel is freed by its writer thread on its next poll (every 20 ms), not at the drop.
- **Read a live host without stack logging.**
  `heap <pid> -sortBySize -showSizes` lists every live block size by count,
  which is enough to match fixed buffers by arithmetic.
  Stacks there would need Bitwig relaunched with `MallocStackLogging` in its environment, which this measurement did not do.

## Ranked by size

1. **Spectrogram history, 53.6 MiB (60 MB resident).**
   Its reach is a product decision (`HISTORY_MAX_SECONDS`, the roll's 600 s span plus headroom);
   the last two tiers hold 262–610 s for 8.7 MiB, so halving the reach saves a sixth.
   Storing each tier's columns in one contiguous ring instead of one box per column would drop the 7% malloc rounding (about 4 MB) and 7,336 allocations.
   Halving the bytes per bin again is the only large lever, and it undoes what #1505 chose bfloat16 for.
2. **Recorder rings, 11.7 MiB, held whether or not anything ever records.**
   They are built once in `Default` so arming never touches the audio thread's producer (`harmonigraph_record::channel`).
   Making them lazy would need a hand-off of a new producer to the audio thread;
   11.7 MiB does not obviously pay for that.
3. **Editor state, about 10 MiB**, 4.1 of it a visible spectrogram's slab grid, the rest depending on window size and how many nodes are drawn;
   the lattice node buffers grow toward the node cap and are kept at their largest (`LatticeFrames`).
4. **Everything else is under 2 MiB each.**

Nothing found here is large enough to explain the host's size by itself:
in a shared host most of the heap was otonal's, and that is otonal's to cut.

## Not settled

- **One block is missing from the live host.**
  The recorder's 4 MiB audio ring should show in `heap` as a single 4,194,304-byte block, and it does not,
  although a plain 4 MiB `malloc` does show in a test program and the 5.5 MiB event ring beside it does.
- **The 1,065 MB peak in #1511 was not reproduced.**
  Nothing measured here is a CPU transient that large;
  the GPU side's style switch is bounded at +47 MB in `docs/gpu-memory.md`.
