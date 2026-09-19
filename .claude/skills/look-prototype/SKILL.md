---
name: look-prototype
description: Explore a visual idea as a numpy prototype rendered over a real take, shown to Yan as labelled contact sheets he picks from, before anything is ported to the plugin on dials. Use when Yan describes a look he wants ("I have an idea for…", "make it look more like…", a reference image), when a look has been described and built more than once without landing, or when he asks for another round "like the cloud prototype".
---

# Prototype a look on contact sheets, then port the pick on dials

A look's acceptance test is Yan's eye, and words do not carry a look:
seven describe → build → describe rounds on PR #888 each executed one reading of an under-determined sentence, and the only real jumps came from him pointing at pictures.
So the loop is pictures first, in seconds per iteration, and the three-minute plugin build happens once, after he has picked.

The kit beside this file (`kit/`) is the reusable half.
It needs `numpy` and `ffmpeg` and nothing else —
there is no PIL on Yan's Mac, which is why `png.py` exists.

| file | what it gives you |
|---|---|
| `png.py` | `write_png`, a 5×7 label font (`draw_text`, `label_cell`), and `sheet(cells, cols, title=…)` to montage labelled cells |
| `spectro.py` | `decode` a take's audio through ffmpeg, `stft_log` to a log-frequency dB picture, `light_field`, a palette, `gauss_blur` |
| `pane.py` | `pane_light(W, H, flat=None)` — a cached excerpt of a real take as the 0..1 light field a pane would show, or a featureless field with `flat=` |
| `measure.py` | `line_spacing` and `legibility(base, out, spacing)` — whether the harmonic lines survive an effect, as a number for the label |
| `example_sheet.py` | the whole pattern in one short file: a two-axis sweep of a toy effect, labelled, written as one PNG. Copy it, don't import it |

## 0. Work in scratch, not in the repo

Copy `kit/` into `$CLAUDE_JOB_DIR/tmp/proto/` (or any scratch directory) and write the prototype there.
The kit caches the decoded take beside itself, renders are large, and none of it is a tracked file.
What is worth keeping at the end goes to the places in step 6.

## 1. Pin the brief down with a veto, not a description

Ask for a reference image or an annotated screenshot before adjectives.
Then hand back a short numbered list of MECHANICAL readings of it —
"each lobe is a near-flat wash with its own tone", "separate bodies with empty sky between" —
and let him strike lines.
On #888 two of five readings were vetoed in one message, and the vetoes were the brief.
Quote his words verbatim into your notes; every later decision gets checked against them.

## 2. Build the prototype as a per-pixel function

Write the effect as a function of pixel position, the light field from `pane_light`, a clock, and a dict of parameters —
nothing a fragment shader could not also do, so the port is a translation rather than a redesign.
No neighbourhood operations the shader has no texture for, no global passes, no sorting.
Count what it would cost as you go (hashes, texture taps, ring size), because the first question after the pick is whether it can run.

**Render at `ss=1`.** Supersampling flatters a procedural edge the shader will draw aliased;
#888's first round was `ss=2` and its second round had to retract what that hid.

**Render over the real take**, at a size where the music's own detail is visible.
Pure tones and test patterns hide every edge artifact a dense ragged band shows.
Also render the `flat=` field: an effect that invents structure out of a featureless input is drawing itself, not the sound.

## 3. Sheets are for choosing, so build them to be chosen from

- **One sheet, one question.**
  A sheet of complete looks ("which of these six"), or a sheet sweeping one or two parameters ("how much of this"), or a sheet isolating each mechanism alone at three strengths ("what does this dial do").
  Mixed sheets get mixed answers.
- **Label every cell** with a short ID he can say back (`J2`, `E3`) plus the parameters that differ, plus any measured score.
  His reply will be "J1, J2 and J5" or "B3 with A1's edges" — that is the format working.
- **Add a 2× crop sheet.**
  His display is Retina; a texture that reads as grain at 1:1 reads as soft lobes at 2×, and the pick can flip on it.
- **Measure what he would otherwise have to squint for**, and put the number in the label.
  `measure.legibility` is the worked example: it confirmed his "is this one harmonic?" before he had to argue it.
  Do not optimise the number; past the useful band it keeps climbing while the picture gets worse.
- Motion the stills cannot show goes in a short side-by-side mp4, not a paragraph.

READ your own sheets before sending them.
Say what you see, including what still misses the reference — and never tell him a sheet looks good; verifying properties is your job, the look is his pick.

## 4. Send, and ask for a pick

Send the PNGs with `SendUserFile` (paths in a terminal are no use on his phone), with three or four lines on how to read them: which sheet answers which question, what the label numbers mean, what this round found.
Then ask for a pick, not an opinion.
More dials is a standing preference of his: when a quality is in doubt, plan to put it on a bar rather than deciding it on a sheet.

## 5. After each round, write down what it ruled out

The expensive product of a round is the list of what the look is NOT:
every rendered dead end with the one-line reason it failed ("z-buffered spheres → near-straight crossings, cracked mud").
A future round, or the port, will otherwise re-try them.
Record his pick verbatim, then what the picked cells have in common mechanically —
three picked looks that are one construction at different settings is a build plan; three different constructions is another round.

## 6. Port the pick — on dials, beside what exists

- Before building, say mechanically what the change adds and what it REMOVES from the screen, so he can veto before a build.
  If he has said he likes what is there now, the new look goes in BESIDE it as a selectable style, never over it.
- Every quality that differed between his picked cells becomes a slider; the defaults reproduce one picked cell, and the hand-over includes a table of slider values that reach the others.
- One builder at a time in a shader file.
  The coordinator stays out of the code, reviews evidence, and commits;
  enter the worktree before spawning the builder.
- The builder proves the port **by picture**: the plugin's render of each picked cell beside the prototype's, through the offline frame harness (a scratch `#[cfg(test)]` on `crate::frames::Renderer` with `Layout::preset("spectral")` and real audio, saved with `image::save_buffer`; delete it before committing).
  Compare structure, not colour — the kit's palette is not the plugin's.
- What the existing look's golden frames prove is that the new style did not disturb it: they pass UNBLESSED or something is wrong.
- Measure cost against the look it sits beside, back to back, and report it rather than quietly trading the look for speed.

Then the ordinary contract applies: both release packages built, draft PR, the `build-handover` skill for the load line.
After he has turned the dials in Bitwig, read the values back with the `capture-daw-state` skill rather than asking him to describe where they ended up.

## Where things are kept

- The prototype scripts and final sheets of a round that led somewhere: a directory beside the session's memory note for that look, with the note indexing it (construction, dead ends, picks, what still misses).
- The kit only changes when something GENERIC was learned — a new measure, a label feature. A look's own renderer never goes in `kit/`.
- The first full run of this loop, with both rounds' sheets and the renderer, is PR #888's wash look.
