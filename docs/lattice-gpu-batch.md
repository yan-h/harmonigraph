# First lattice GPU batch

Parent:
`70e9f48dddb3c14e5cb2d2173d5604f8a4c34e84` (PR #652).
This batch covers #642 A, #641, #644 A/B and #645 A.
The depth removal subsumes the intermediate discard-store slice.
Detailed timing, history ownership, view retirement and later GPU algorithms remain separate work.

## Implementation slices

- The docked owner's two beginning-of-pass timestamps now bracket all lattice preparation:
shadows, ink history and convolution, glow, ordered scene composition and optional bloom.
A small opening pass precedes every optional stage;
the dedicated tail pass and asynchronous readback cycle remain.
The final egui composite belongs to the host pass and is excluded.
Previews do not advance or consume the docked timer.
- `vs_ink_blur` collapses nonpositive-glow instances exactly as `vs_ink_strip` already does.
Both replacement-write passes still clear obsolete rows.
An active owner of row zero performs its own convolution;
glow-free audio rings no longer repeat it.
- The unread `Depth32Float` texture, scene attachment and pipeline depth declarations are removed.
Painter order remains responsible for occlusion, including interleaved markers and each label's shadow then ink.
- An optional `LatticeBloom` owns the independent nodes-only attachment and three bloom intermediates.
It exists while bloom strength is positive and a drawable scene needs it.
Single- and dual-attachment `ScenePipelines` are built by the same factory at startup and hot reload.
Glyph ink still writes only the visible picture;
label shadows preserve their existing treatment of both images when bloom is on.
A transparent shared texel supplies the unused composite binding when bloom is off.

Bloom's retained key is its enabled state within an `Offscreen` of fixed scene and native sizes.
Changing a positive strength updates uniforms only.
Crossing zero replaces only bloom allocations and the final composite binding;
main colour, shadows, glow and ink history remain untouched.
Resize retains the existing same-capacity strip transfer.
Capacity growth retains the existing replacement/reseed behaviour, including loss of a release colour that has no current ink.
There is no saved-state or scene/instance API change.

## Allocation effect

These are texture-descriptor sizes, excluding driver overhead, compression and tiling.
They establish allocation removal, not physical traffic or an FPS prediction.

| Pane and scale | Removed depth | Bloom allocations avoided while off |
|---|---:|---:|
| 256 × 256 native, scale 1 | 262,144 bytes | 720,896 bytes |
| 3840 × 2160 native, scale 2 | 126.6 MiB | 276.9 MiB |

The bloom sum includes the scene-resolution nodes-only image and a native half-size image plus two quarter-size images.
At 4K/scale 2 those components are 253.1 and 23.7 MiB respectively.
Bloom-off scene passes also avoid clearing, writing and storing their second attachment, including the fullscreen glow draw.
Repeated positive strengths keep the same allocation;
each off/on cycle creates exactly the four bloom images required on re-entry.
The main picture and glow history do not churn with those cycles.

## Verification and measurements

The focused target-lifetime fixture checks actual GPU handles and byte-exact stable pixels through repeated bloom on/off/on during an inkless release, native resize and render scale changes.
It separately crosses capacity 1 → 2 to verify the existing reseed baseline, reuses row zero with a different colour, and exercises glow off/on.
The cull fixture compares five frames against the retained unculled blur shader with 25 shipped nodes, covering mixed audio/MIDI, release, glow ending while audio remains and row reuse.
The reload fixture rebuilds both attachment variants from real shader files and draws names, shadows and glow through both.
The timer fixture requires a positive returned measurement and verifies that a preview leaves its owner's readback state and published value unchanged.
Existing lattice and offline goldens are required without blessing.

The ignored complete-preparation probe uses beginning timestamps at both ends and reports CPU wall time around the complete callback as well as GPU time.
`PROBE_BLOOM=0` and `PROBE_BLOOM=1` select comparable runs without changing geometry.
Cold CPU figures include pipeline and target creation;
steady figures exclude ten warmup frames.
Measurement results and validation outcomes are recorded after the coordinated run.
