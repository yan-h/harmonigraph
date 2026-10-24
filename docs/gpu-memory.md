# GPU memory

Every texture and buffer the renderer allocates is on a record (`harmonigraph_render::gpu_memory`).
Two places show it while the plugin runs:
the Performance overlay's Frame breakdown has a `gpu mem` row,
and the Console's GPU memory button logs the whole table and copies it.

The row reads `ours · dev · peak`.
`ours` is what the recorded resources hold now.
`dev` is Metal's `currentAllocatedSize` for the device: ours plus egui's textures, wgpu's internals and the swapchain's drawables.
`peak` is the most `ours` has ever held;
the table names what was alive at that moment, and which allocation tipped it over.
All three are per process, not per plugin instance:
a second Harmonigraph in the same host adds to `ours` and `peak`,
another Metal plugin adds to `dev`,
and `peak` covers the whole host session.

## What a pane off screen still holds

Every pane's targets are kept per surface for as long as the renderer is,
and nothing frees them when a pane stops drawing (#1587).
So `ours` counts a hidden docked tab and a closed Render preview until the editor window closes,
and an export's placements until that export ends.
Per surface, that is roughly:

- the roll's instance buffer, bloom chain and cut targets, about 15–20 MB (61131fd1);
- the spectrogram's grid copy and the cloud's targets: the star atlas (about 19 MB), the tone target, and the baked tile, up to 75 MB at its 2048-texel ceiling;
- each text batch's glyph buffer and shadow atlas, not measured;
- the lattice's own targets, which have always been kept this way.

## Reading it against `footprint`

`footprint <pid>` charges GPU memory to three categories, and none of them is the record.
On an M5 Pro, the standalone harness at 2560×1600 pixels in the default look read:

| layer | MB | what it is |
|---|---:|---|
| record (`ours`) | 151 | our targets and buffers |
| device (`dev`) | 204 | + three 2560×1600 BGRA drawables (49 MB), egui's font texture, wgpu's internals |
| Owned physical footprint (unmapped) (graphics) | 326 | + a driver pool of about 120–140 MB |

Facts measured on that machine, by probe tests run and then removed:

- **Textures cost nothing until first used.**
  A created texture adds nothing to `footprint`;
  its first pass commits exactly its `allocatedSize`, for every format and usage tried.
  An allocated target that is never drawn into costs `dev` but not `footprint`.
- **The first GPU submission brings a driver pool of about 140 MB.**
  It appeared on the first pass into a 64×64 target, with no pipeline compiled,
  and was returned after about a second with the GPU idle.
  A picture that animates never idles, so the pool stays.
  Compiling every lattice pipeline added nothing to either graphics category.
- **Freed memory leaves `footprint` a second or two late.**
  `dev` drops the moment Metal frees a texture;
  the driver keeps the pages, and `footprint` keeps counting them, for one to two seconds.
  A target freed and replaced within that window is counted twice, whatever order the code ran in.
- **`IOAccelerator (graphics)` is not attributed.**
  It read 75–140 MB in the harness.
  Neither pipeline compilation (4 MB) nor a `write_buffer` of 1 MB a frame for three seconds (2 MB) explains it.

## Peaks

To attribute a peak, read the table's last block:
what was alive at the peak and has been freed since.
Two things put old and new targets on the record together without any ordering of ours:
wgpu keeps a texture until the frame that last used it has finished on the GPU,
and one pane can build its new targets before another pane, preparing later in the frame, releases its old ones.
A resize is the exception, because reconfiguring the surface waits for the GPU first.
PR #1506 measured a resize, a style switch and the driver behaviour above.

`phys_footprint_peak` counts the CPU heap as well, which `docs/cpu-memory.md` measures.

