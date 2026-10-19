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

A **resize** never holds two sets of targets at once.
Every target keyed on a pane's size (the lattice's offscreen set, the roll's, the spiral glow's, the cloud's) is dropped before its replacement is built,
and a resize reconfigures the surface, which waits for the GPU first,
so wgpu frees the old set at the drop.
A drag between 2324 and 2560 pixels wide never raised the record's peak above its startup value.
Targets a dial rebuilds rather than a size (the lattice's star targets and material tile, the ink strip, a grown shadow atlas, the font atlas) are built before the old one drops;
the largest, the lattice's star light and image, is about 30 MB at this size, and no dial drag was measured.

A **style switch** does hold both, for under a frame, through no ordering of ours.
Watercolor to Stars peaked at 198 MB against 151 before and 160 after.
The table at that peak held all of Watercolor's cloud,
its 28 MB colour-memory pair included, while Stars' targets were being built.
Two causes, measured apart:

- wgpu keeps a texture until the frame that last used it has finished on the GPU,
  so the old cloud outlives the drop by a frame.
  Headless, with no frame in flight, the same switch peaks at exactly the Stars set.
  A non-blocking `device.poll` at the rebuild changed nothing;
  a blocking wait there cut the peak by 15 MB.
- The lattice prepares before the spectrogram,
  so it builds its Stars targets while the spectrogram pane still holds Watercolor's,
  which it releases only in its own prepare later in the frame.

Neither is worth a stall:
`footprint` keeps the freed pages for a second or two anyway,
so `phys_footprint_peak` would not move.

A process's `phys_footprint_peak` is the CPU heap's as much as the GPU's,
and nothing here measures the heap.
