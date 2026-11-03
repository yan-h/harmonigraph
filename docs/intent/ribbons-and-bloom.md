# Intent: MIDI ribbons and their bloom

Yan's answers from drift audits of this area (the `audit-drift` skill).
One line per decision; the audit reads this before asking anything.

## 2026-09-30 (first trial)

- A ribbon's bloom follows the opacity it is drawn at, as the lattice's does. #1289
- One note's shadow never darkens under another note, at any opacity. Low priority. #1293
- Bloom halo width depends on device pixels, so display scale and export resolution change it. Worth thinking about; undecided. #1294
- Note-name thinning should become one comprehensive placement pass across pitches. Low priority. #1295
- The ribbon width minimum (1.5 pt, applied before Thickness) is worth revisiting. #1296
- Tiny gaps and tiny notes are edge cases worth thinking about: butted notes merging into one ribbon, short notes stretched to a floor. #1297
- The rest of that trial's behaviour description: nothing obviously wrong. Don't re-ask item by item.

## 2026-10-02 (issue sweep decisions)

These settle the open questions from the first trial above.

- Bloom radius follows the owning pane's shorter dimension, independently of render scale, clipping, the spectrum/history divider and Lead. The previous nine-tap profile is the reference at a 1080-pixel shorter pane edge. #1294
- Artistic lattice edge softness is one logical point; actual antialiasing retains a one-fragment floor. Analyzer backdrop stripes are one logical point wide, with Gap expressed in points. #1294
- Exports use a 1280-point-wide composition at every output resolution; previews scale its point-sized details with the composition. #1294
- One note-name collision pass measures actual letters and marks across pitches. Held notes win, then previously visible labels, then onset, pitch and voice break ties. Loudness does not reorder labels. A displaced visible label stays suppressed until it leaves the viewport; a view adjustment releases suppressions while retaining visible priority, and backward time clears history. #1295
- Positive Thickness receives the 1.5-point ribbon width minimum after mapping. Exact zero removes body and shadow. #1296
- A touching re-strike gets a short narrowing at its actual onset. This local notch may fall below the ordinary width minimum. #1297
- The minimum note length is one logical point, preserving the Retina reference. A shorter note extends flat at its onset pitch into the past; actual bends keep their timestamps and slopes. Lower-resolution output may resolve that minimum more softly. #1297

## 2026-10-03 (repeat readability)

- Readable repeated notes need a visible gap; the onset notch was too subtle when zooming out.
- Reserve a one-point core per note and two points between neighbouring notes of the same source, channel and key. Trim the drawn ends around the factual boundary when needed; recorded MIDI times do not move.
- When those cores and gaps no longer fit, join the local repeated-note run into a solid ribbon with uniform inset hatching. Bold diagonal strokes repeat evenly every six composition points; the long edges remain continuous. The strokes span the middle two-thirds of a typical ribbon and dim its fill by 75% so bloom does not wash them out. The ornament signals density rather than counting individual attacks.
- Sustained notes remain plain. Resolvable rests and different voices remain separate. Pitch bends and expression history are retained inside dense runs.
- Minimum-length onset padding meets the first effective pitch segment, including tuning received at the note-on timestamp; it must not leave a short line at the untuned key. Later pitch bends retain their recorded geometry.

## 2026-10-20 (legibility over every backdrop)

- The goal: ribbons legible whatever their color and whatever the backdrop, looking good, in the plugin's ambient, spacey feel. Yan is not attached to shadows for it.
- Picked off look-prototype contact sheets (W8): a glow in each ribbon's own hue, dark (L* 7 by default), falling off by distance from the ribbon's edge. It replaces the black blur under ribbons; the spiral's dots keep their blur.
- The glow's reach, darkness and spread stay the shared Notes shadow group's. Ribbon glow lightness and color are two new bars, the roll's alone.
- This reverses #1499's every-shadow-is-a-blur for the ribbons only: a blur of a ribbon thinner than σ barely darkened round it, which is where legibility failed.
- Ribbon glow darkness is linear in the profile (Darkness × falloff), as W8 was prototyped, rather than #1314's stops curve; the dots keep stops, so the shared Darkness bar follows two curves.

## 2026-10-20 (backdrop cut, #1525)

- The glow never reaches full strength, so over very bright spectrogram energy the backdrop leaked through it and the halo read thin.
- Rejected on the way: Darkness 100% with a solid Spread band; dimming the spectrogram's level, its Oklab lightness or a per-channel ceiling under the glow, each of which shifted the heatmap's hue at the halo's edge.
- Picked off look-prototype rounds R3–R8: a composite-only backdrop cut, with no coupling to the spectrogram. The cut is whole at the ribbon's edge and fades over four σ along a smoothstep (S4), to black.
- Between close notes the strongest-wins cut folded into a crease along the bisector ("a weird grid artifact"); a smooth minimum of the notes' distances removes it, at σ / 2 by default (R8).
- Yan asked to tune it on sliders in the plugin rather than on more sheets: Backdrop cut, Cut reach, Cut curve, Cut fill and Halo blend, in the Ribbon glow block. A cut of 0 is the shipped glow exactly.

## 2026-10-20 (cut only, #1528)

- Once the cut is on it does all the backdrop removal, so the glow's only remaining job was a faint L* 7 tint. The surround is the cut alone.
- The Spiral's dots wear the same cut instead of their blur, and the Notes shadow group is gone: notes on both panes share one Note surround block.
- Cut fill stays, so the cut can fill with each note's hue; Fill lightness stays, and the fill's chroma is fixed at full.
- Cut curve (linear to smoothstep) is replaced by Cut shape, one bipolar bar: a steep drop at the note with a long tail at one end, linear in the middle, held near the note with a steep drop at the far end at the other.
- Halo blend is gone: Yan wants the strongest note's cut to stand alone, so cut and fill share one strongest-wins pass.
- Cut fill is gone, fixed at 100%: the cut always fills with the note's own hue, and Fill lightness 0 is the black cut. At the default lightness 7 the hue is close to black.

## 2026-11-02 (analyzer backdrop removed)

- The Analyzer backdrop is gone, stripes and gradient alike, with its Backdrop strength, Backdrop height and Stripe spacing bars. The stripe half of the 2026-10-02 line on edge softness no longer applies. The ribbons' Backdrop cut is a different thing and stays.

## 2026-11-02 (tint fade, round R9)

- Turned up from near black, the fill read as discordant: at 15 points of reach neighbouring notes' fills merge into a sheet of their hue over the heatmap, worst as rust over blue and teal round the high notes.
- It is not the fill going lighter than the backdrop: at L* 25 only 9% of the surround is lighter than the bare picture.
- Rejected: lower chroma goes grey-olive and loses the tint; a shorter reach is the worst for legibility; pulling hues toward violet ties the fill to one heatmap, Aurora.
- Picked (Y3): Tint fade 3 at Fill lightness 25, so the hue stays by the note and the cut carries on dark to its reach. Any lifted fill costs some legibility against the near-black one; the faster fade wins part of it back.
- Fill lightness now stops at 55, about the full tint: the default gradient's fills hold the most chroma there on average, and higher takes them toward white, which Yan never wants.

## 2026-11-02 (drift audit, #1635)

- At the shipped look (Fill lightness 25, Tint fade 3) a rest between fast repeats of one key reads about 0.53 of the body, and that still counts as the visible gap of 2026-10-03: the tint stays in the gaps. The gap test runs at the fresh defaults with a bar to match. #1635 Roll Q1, #1630
