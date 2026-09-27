# Off-pane glow owners

Issue [#1189](https://github.com/yan-h/harmonigraph/issues/1189) is reproduced at `b3dceb19` on Apple M1 Pro / Metal.
No production fix is enabled here.

Apply `probe.patch` and run:

```sh
HARMONIGRAPH_REQUIRE_GPU=1 cargo test --release -p harmonigraph-render scrolled_window_keeps_the_halos_of_off_pane_nodes -- --nocapture
```

The probe compares the live scrolled window against the same window padded by five lattice steps on each edge.
It uses a held five-note chord,
one sheet,
the current fresh view,
and a 512×512 camera at horizontal offsets 0 and 0.25.
It is a scratch reproduction rather than committed test coverage:
the assertion expects the eventual fix to match the padded reference.

| Camera offset | Current nodes | Padded reference nodes | Changed RGBA channels | Largest byte difference |
|---|---:|---:|---:|---:|
| 0 | 225 | 625 | 1,380 | 1 |
| 0.25 | 210 | 600 | 32,277 | 4 |

Applying `wide-margin.patch` as well sizes the window margin from the renderer's full halo radius.
The same comparisons become byte-identical,
but the production window grows to 361 and 342 nodes respectively:
60–63% more derived positions in this fixture.
This is a count of work,
not a measured frame-time slowdown.

The candidate also breaks the existing node-budget guarantee:
a fully zoomed-out cabinet pane at aspect 3,
nine sheets,
and shear 1 asks for 21,681 nodes against the 20,480 cap.
The existing cap then trims the enlarged window.
The narrow-window and zoom-extent tests also fail.

The broad-margin candidate was reverted.
Raising the cap would trade more CPU/GPU work for a small edge-light correction and would weaken an existing resource bound.
A narrower candidate would derive only illuminated extra halo owners while preserving the existing visible window and its cap,
but that needs an explicit design for scene ownership and filtering rather than a bigger rectangle.
No sparse-owner implementation or timing claim is made here.
