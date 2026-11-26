# Intent: Tape mode

Yan’s decisions recorded for future drift audits.

## 2026-11-14 (drift audit, #1784)

- An audition keeps the Lattice Map selection and offset recorded at its spot, since Yan often automates the map over time, and draws and names under the live tuning axes, which are never automated (#1756). Resolves #1735. #1784 T1
- In Tape mode a press on the tape picture only cues, and pans nothing. Resolves #1738. #1784 T2
- Cut the starved timer, the shown-first order of a whole-tape re-analysis and the meter re-anchor past a short pass's end, and prime the stretcher with the recording before the spot as read, without its walk across gaps. #1784 T3
- 0.25× stays. Its test keeps loose tolerances, because the stretch crate exposes no seed for its phase randomisation. #1784 T4

## 2026-11-18 (maintainability audit, #1861)

Yan took the recommendations made after the audit was checked against the code ("go with all recommendations", 2026-11-18).

- The Tape heatmap shares the spectrogram's folded grid, keyed on a Tape revision count bumped when slabs settle; folding `SongAxis` into `TimeAxis` is optional; #1837 and #1842 are fixed in their own PRs. #1861 R4 (Built in #1892, with #1842's fix: the revision advances on every write to a copy, `put` and `clear`, since the copy the pane draws is written only by the sync and never settled. `SongAxis` was not folded into `TimeAxis`: it carries the stretch shown, `Window`, which the coverage band, the ruler, the playhead's speed and the heatmap read and `TimeAxis` has no place for.)

## 2026-11-24 (PR #1972)

- A pass stores no silence at its end, so a transport left rolling past the music neither stretches Tape mode's fit nor evicts the recording; silence is a block whose every sample is below −90 dBFS.
- While the transport is stopped, Tape mode's fit leaves out the reported position; a stopped mark past the recording is not drawn, and the readout still gives it.
- Silence a pass drops still clears the older recordings it beats, so a shortened or muted ending leaves no old one behind.
  A stop during a rest clears the older take there too, leaving a hole where an audition ends.
- A rest longer than 30 s while rolling is a gap where an audition stops, rather than covered silence.

## 2026-11-25 (harmony row, #1974)

Yan's picks over six rounds of prototype sheets, with his words where they decide something.

- Tape mode shows the recording's harmony as a row of small lattices, one per section of the song, each in one frame for the whole song and lighting the nodes that sound in it. "1 with 5 sounds good"
- A section starts where the sounding notes change enough, not on a bar grid or at a Lattice Map change, and a run of short notes is one harmony: "note clouds or rapid arpeggios … read as a single harmonic area".
- One rule finds the sections, the change across each attack, with two bars to fit it to a piece: Detail and Span. "A but with more splitting", "an intuitive slider or two", "Drop a+".
- A node is drawn as concentric rings, one per pitch sounding on it, lowest in the middle. "I think g3 makes sense given the size of the thumbnail"
- Colour is pitch height alone, and brightness means nothing: "either have brightness or new note distinguish … not both", "brightness … overlaps a bit with color for pitch height".
- A note struck again in a section counts as new there ("Red truck should count", dictated for re-struck), and new notes are told apart by animation, with no still mark: "I'm ok with prototyping with a simple animation for now". Yan chose this on a phone without seeing the animation clip.
- Note length is not shown, and the whole song drawn on one lattice (a ring on each node, the song running round it) waits until the row is in use. Yan ("Go with recommendations")
- Yan confirmed the four picks made while building ("Picks stand", 2026-11-25): the row sits in Tape mode above the overview, the whole song across it;
  new notes arrive in the thumbnail under the playhead as they are struck, and stopped, nothing marks them;
  a click on a thumbnail cues the audition from its section's first attack;
  Span is in beats at the tape's recorded tempo, 4 beats fresh.
- A pitch off the live tuning lights nothing in the row, as on the lattice, so a recorded 12-TET track under just or meantone axes is mostly unlit; this is a documented limit, with no snapping of tempered pitches to a node (Yan, "Document it", 2026-11-25).
- From PR #1975's review questions (Yan, "Go with recommendations", 2026-11-25):
  a struck note's flash settles over half a second of song, so it keeps step with the music at any audition speed;
  a note held across a recording seam is struck there, as the roll draws it, so it flashes and a section can start there;
  a pitch lights the one node it is named by, even where 12-TET axes without meantone draw a thumbnail as one column of fifths.
