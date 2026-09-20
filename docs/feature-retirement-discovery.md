# Whole-feature retirement discovery

**Implementation relevance:** Playhead was retired in [#988](https://github.com/yan-h/harmonigraph/pull/988).
Public single-pane/custom export layouts are retired under [#974](https://github.com/yan-h/harmonigraph/issues/974), with internal single-pane pixel fixtures retained.
The owner names and preset/load examples below describe the pinned discovery baseline, not current interfaces.

Date: 2026-09-19.
Source inspected: `0b6868a8`;
product sources remain at `1786c5ba1a4e2994c20dc2672eb7d200fd605d9b`.
Continuation of the [requirements-value audit](requirements-value-audit.md),
prepared in PR #951.
Read-only source and consumer trace;
no removal prototype, new tests, timings or product changes.

## Direct usage evidence and recommendations

Yan uses Spiral occasionally.
He does not personally use Notes,
Console,
the standalone application,
the browser tuning laboratory,
Playhead exports,
single-pane exports or custom export arrangements.
He explicitly uses the controls to arrange and orient Lattice and Analyzer together.
These usage answers initially supplied evidence only.
Yan subsequently explicitly approved removing Playhead and the extra public export layouts,
while keeping the combined arrangement/orientation controls.
The four panes/developer interfaces still have no removal approval.
All accepted removals remain deferred in the [plan](maintainability-plan.md).

Discovery follow-ups are tracked in [#973: Playhead](https://github.com/yan-h/harmonigraph/issues/973),
[#974: extra public export layouts](https://github.com/yan-h/harmonigraph/issues/974) and [#975: unused panes and developer interfaces](https://github.com/yan-h/harmonigraph/issues/975).
Opening these issues did not approve product removal;
the later direct decision settles #973 and #974 only.

| Lead | Actual retirement opportunity | Recommendation |
| --- | --- | --- |
| Playhead export | Retire a distinct precomputed spectrogram/full-note-roll path and many fixed-timeline branches. | Accepted, implementation deferred. Retain Scrolling, Whole video and live history; check shared constants and tests below. |
| Single-pane/custom export options | Retire public preset choices and custom file input without deleting the shared preview/render layout engine. | Accepted, implementation deferred. Combined controls and internal single-pane test compositions remain useful. |
| Notes pane | Retire a held-voice debug table and its exclusive nearest-node lookup. | Small candidate; retain shared note tracking, naming and Analyzer off-lattice warning. |
| Console | UI is small, but its buffer is the only current in-app sink for several important failures. | Recommend keeping the existing small pane: the completed consumer check found no equivalent plugin diagnostic for two retained behaviors. This is an audit recommendation, not a new user requirement. |
| Standalone application | Retire a separate eframe/midir shell, mock input, screenshots and manual take-generation workflow. | Recommend leaving it in place at this checkpoint. Current launch frequency is unknown; no existing replacement for native interactive debugging was established. Do not create a replacement framework to justify retirement. |
| Browser tuning lab | Retire browser presentation/worker while retaining the model that generates Rust musical fixtures. | Browser-only retirement is a credible candidate. Retain the headless musical reference unless its replacement is separately justified. |

## Playhead is separate from Whole video

`crates/harmonigraph-offline/src/render.rs` handles Whole video by calling `span_history` once before rendering.
It then uses the ordinary scrolling picture path.
Playhead instead calls `WholeSong::precompute`,
stores an entire fixed render-window spectrogram,
and populates its note roll through `Replay::full_roll` before frames begin.
`TimeAxis` maps the fixed window differently from the ordinary moving history.
Removing that mode does not require sacrificing Whole video's longer scrolling span.

The feature's concrete owners are:

- `SpectrogramRender::Playhead` and its Video-page option/preview placeholder;
offline `--playhead`,
`Settings::whole_song_spectrogram`,
the precomputation block and its empty-window warning.
- `WholeSong` in UI `spectrum.rs` and `VisualRuntime::whole_song`;
the precomputed columns and full-roll ownership.
- Fixed-timeline branches in spectral axes,
roll,
names,
pane composition and spectrogram planning/folding.
The ordinary live spectrum/history/fold/cache code remains needed.
- Record-side `RenderRequest.playhead` and flag emission;
production constructors currently always set it to `None`,
while the take's saved mode independently enables Playhead.

Important shared consumers and limits:

- `TimeAxis::new` uses `WholeSong::MIN_WINDOW` even on its **live** arm.
Preserve the existing minimum window at an appropriate retained owner;
deleting the type does not delete that live invariant.
- `Replay::full_roll` is also used by tests of canonical history,
outages and replay ordering.
Retire its production-only consumer if appropriate,
but preserve those behavioral assertions through a suitable test helper or retained path.
- The offline golden suite has a Playhead frame alongside ordinary scrolling frames.
Retire only obsolete feature coverage;
retain ordinary analyzer/roll/picture checks and intentionally shared fixtures.
- Removing the persisted `Playhead` enum variant can reject an entire appearance containing it,
not merely default that field.
Follow the existing [persistence contract](../.claude/skills/persistence-contract/SKILL.md),
disclose the saved-state effect and preserve observable refusal.
A version-floor bump does not repair a parse that failed before the floor was read.
- The parked whole-song performance/coverage proposals are not reopened by this source trace.
If the product mode is retired,
their relevance can be updated rather than implementing them first.

The evidence establishes separable obligations,
not a measured runtime saving or a complete deletion diff.

## Export arrangement: preserve the useful composition

The normal Video preview calls `Layout::split(frame.lattice, frame.split)`.
The offline default calls the same method from the captured or overridden appearance.
It supports Left,
Right,
Top and Bottom with a shared proportion;
Analyzer orientation is another retained picture setting.
These are the controls Yan explicitly uses.

The extra public surface is `Layout::preset`'s `lattice`, `spectral` and `spiral` choices,
`Layout::load`'s custom RON input,
and offline layout argument/dump handling.
Retiring those options does not establish that `Layout`, `Placement`, rectangle resolution or all presets should disappear.
The ordinary side-by-side/stacked CLI choices were not separately established as unused.

Single-pane compositions have current developer consumers:
offline goldens repeatedly use `Layout::preset("spectral")`,
the eight picture probes in offline `frames.rs` use `Layout::preset("lattice")`,
and the existing look-prototype skill directs a builder to use a spectral-only render for comparisons.
Those consumers need a retained internal way to compose a pane if the user-facing options go.
Do not delete visual validation or preserve an external file format solely because a fixture needs a rectangle.
No new generalized layout abstraction is justified by this audit.

## Notes and Console are different cases

The shared `panes/notes.rs` file is 79 lines including comments and blanks,
and contains **both** pane implementations.
Notes sorts held voices and shows source,
channel,
pitch and nearest visible node.
Its dedicated nearest-node lookup can be retired with the pane.
`NoteTracker`,
shared naming helpers,
the current visible-node set and Analyzer's off-lattice warning remain used elsewhere.
The strongest keep argument is developer inspection of exact per-voice values;
no user-facing musical behavior requires the table.

Console's runtime buffer keeps at most 500 lines.
Writers include saved-state/appearance rejection,
background-analyzer failure,
tuning-command refusal,
GUI stalls and resize diagnostics,
plus standalone MIDI/startup messages.
The current `Console::log` stores text without mirroring it to another logger.
Separate tuning logs and offline stderr do not cover all these messages.
Removing only the tab while keeping its buffer would conceal existing failure explanations.

The Console is therefore a weak deletion candidate until a concrete diagnostic alternative is traced.
Preserving observable failure does not require inventing a new logging platform.
Both Notes and Console are persisted `Tab` variants;
dropping one can reject a saved dock even if Yan never opened that tab.
The version floor is not a substitute for disclosing and reporting the refusal.

The subsequent consumer check inspected main at `8b4edf4e`.
Tuning mailbox refusal and incomplete/publication history already have inline warnings,
but saved-state refusal and failure to start the background analyzer have no equivalent plugin-visible alternative.
The latter explains losing the closed-window spectrogram history Yan explicitly wants.
Existing offline stderr and tuning-specific logs do not cover those plugin failures.
Commit `8306e98` deliberately made refused saved state observable;
persist and background-restore tests still assert that diagnostic.
Keeping the small existing Console is the bounded recommendation,
not starting a logging project.

## Standalone and browser laboratory consumers

The standalone crate is a separate 1,018-line application file plus its manifest.
It is the only direct workspace user of eframe and midir,
and provides mock MIDI/audio,
optional hardware MIDI,
`LATTICE_TAKE` recording and `LATTICE_SCREENSHOT` capture.
Workspace checks compile it;
README/development instructions offer it as the no-DAW interaction path.
Removing it would lose that developer workflow,
not the shared core/UI/render/take crates.
Existing headless renderer probes do not by themselves reproduce interactive input and native window behavior.

Its eframe shell is also the known production consumer of the CPU font-atlas fallback,
because that shell cannot publish the current renderer texture at the required point in the frame.
Renderer tests exercise the fallback too.
Retiring the shell makes the fallback a follow-up lead;
it does not prove every fallback or hot-reload feature can be removed without another consumer trace.
No current usage frequency of the standalone by agents was established.

The follow-up history check found maintained capabilities,
including MPE handling (`86e62df2`),
event tracing (`312fffe`),
self-screenshot (`00a46265`),
font bounds ([#553](https://github.com/yan-h/harmonigraph/pull/553)) and shared runtime ownership ([#798](https://github.com/yan-h/harmonigraph/pull/798)).
Those changes and the current development instructions establish a maintained no-DAW tool,
not a measured frequency of interactive use.
The inspected #798 validation is explicitly headless;
no recent standalone launch record was established by this bounded search.
Private agent transcripts were not searched.
At this checkpoint,
leave the tool in place rather than ask Yan to decide how agents should debug native windows.
This is a provisional audit recommendation,
not a requirement to preserve it indefinitely or an invitation to keep investigating without a new reason.

The browser laboratory separates more cleanly:
`index.html`, `app.mjs`, `style.css` and `reach-worker.mjs` implement the interactive UI.
`model.mjs`, `examples.mjs`, `model.test.mjs` and `export-plugin-fixtures.mjs` provide the headless model and fixture pipeline.
The generator writes `harmonigraph-core/src/policy/fixtures.txt`,
which the Rust policy tests include directly.
Removing browser controls does not retire that model or its musical checks.
The model's reachability-specific parts could be re-evaluated with the accepted production-outline retirement,
but shared selection and musical fixture generation remain separate obligations.

No automated Node-test/generator invocation was found in current CI;
the documented manual commands and generated Rust fixture are the identified consumers.
Do not turn the absence of an automated invocation into a reason to add a CI job during this discovery.

Actual historical browser checks were reported in the introduction PR [#778](https://github.com/yan-h/harmonigraph/pull/778),
including musical examples,
controls and import/export round-trip behavior.
That establishes historical validation,
not current user demand.
The headless consumer is still active:
[#956](https://github.com/yan-h/harmonigraph/pull/956) updates the generator,
examples,
committed fixture and Rust reader together to carry every declared setting and reach the unsnapped branch.
The [historical change-cost audit](historical-change-cost.md) also traces #865's coordinated production/model/browser update.
Removing browser presentation can retire those UI edits without discarding the distinct reference-model check.

The bounded developer-interface investigation is complete for this checkpoint.
Notes and browser-only retirement remain concrete proposals for Yan;
Console and standalone are recommended keeps for now.
No new product removal beyond the accepted Playhead/export-layout decisions is inferred.
