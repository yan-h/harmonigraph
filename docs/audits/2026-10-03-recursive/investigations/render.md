# R1–R3: rendering and GPU resources

Investigator: render, gpt-6-astra high; read-only at ad1c6e6b.
Status: stated cache/lifetime questions deeply examined by code/fixture inspection; the coordinator measured the narrow R3 prepare stage, not native complete-frame cost.

## R1 — label/atlas ownership

Sheets separates allocation and publication identity (`render/text.rs:390,541–591`); same allocation updates reuse resources, replacement invalidates bindings.
Lattice encodes labels during prepare, spectral text paints after all prepares (`lattice_prepare.rs:73–99`), so different carry-forward rules are necessary.
#1012 (`14e41421`) already eliminated duplicate sheet ownership and texture-view creation.
Fixture `lattice_tests/labels.rs:635` retains old BindGroup handles and distinguishes unchanged reuse from mark-sheet growth.
Standalone still uses the fallback font texture (`standalone/src/main.rs:56–82`); plugin/offline publish the current texture.
Mark atlas append-only behavior inside a pass protects emitted UVs; repacking occurs at pass boundaries (`ui/text.rs:985–1120`).

Reject broad atlas/cache restructuring; no measured case for caching animated node packing (`lattice_frame.rs:211–225`).

## R2 — effect paths and cache inputs

Stars already share geometry/transport/shader via star_plan and stars.rs.
Generated Metal files inflate raw history counts; filtered history is the relevant maintenance signal.
Uniform split remains owner-approved; fixture `spectrogram/tests/star_split.rs:193` crosses 2560×1441, asserts extra pass, <=1-byte parity and partial-region unsplit behavior.
Solo keeps all-layer history deliberately (`spectrogram/color_memory_tests.rs:444`); skipping invisible layers would alter accepted continuity.
GridBuffer keys allocation shape and absolute slab identity, with immutable snapshot identity and byte-equality reuse (`spectrogram.rs:279–310`); palette/style are correctly absent.
Tile key tracks geometry/orientation/effective density, excluding clock/palette/post-bake controls (`spectrogram/atmosphere.rs:297–377`).
Shadows retain distinct per-destination geometry/painter order while sharing packer and kernels (`spectral_shadow.rs:229`); receiver grid #1114 replaces all-later-node scans (`shadow.rs:944–1023`).
Gaussian allocation is conditional (`lattice_prepare.rs:558–575`); disabled/offscreen casters consume no cells (`shadow.rs:468–484`).
PassAged already factors pass lifecycle (#932); caller pass clock is needed because prepare precedes paint across callbacks.

Reject a new generalized renderer/cache framework.
Existing #1348 dead lanes and #886 dense refolds are not new findings.

## R3 — target replacement

#1389 already separated lattice ink history from viewport targets.
Fixture `lattice_tests/targets.rs:178` crosses size/scale boundaries, checks zero new strips and retained handles, separate panes and byte-exact release parity.
Spectrogram history uses 64-pixel allocation buckets (`atmosphere.rs:191`); `color_memory_tests.rs:753–818,511–556` verifies retention/replacement and halo-quality history continuity.

Suspicion: seven allocation-shape comparisons (`spectrogram.rs:846–854`) trigger aggregate Targets::new (`atmosphere.rs:1149–1171`), replacing transient atlas/light targets even when only another dimension changes.
Splitting could preserve expensive unchanged resources, but adds several rebind predicates and attachment/read lifetimes.
The executed 1080p Uniform halo-only probe confirms source replacement while other allocation shapes remain equal.
Thirty alternating pairs measured CPU prepare median 119.354us stable versus 372.292us changing, a 252.938us delta that includes necessary halo allocation.
Verdict: defer splitting; this stage cost does not establish a visible complete-frame hitch or justify more ownership rules.
A 4K/width-drag experiment remains conditional on a saved user-visible symptom; it was not run merely to broaden the matrix.
Preserve only the demonstrated expensive resource, not a generic resource graph.

Fixture limit: tile rebake scheduling test (`spectrogram.rs:4281,4360–4369`) deliberately disables blur time to keep its target counter; it does not prove tile carry through actual Targets replacement.
Carry code appears sound, but a future split should retain tile handle/bake state across real replacement.
