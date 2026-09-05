# #615 Bitwig measurements

Measurements are in progress.
The completed trials below do not yet satisfy the full lifecycle and sub-block acceptance matrix in #615;
#617 and #616 remain gated on that result.
The apparatus is in [draft PR #630](https://github.com/yan-h/harmonigraph/pull/630), and its configuration and limitations are in [tuning-probe.md](tuning-probe.md).

## Fixture

Measured on 2026-09-04 in Bitwig Studio 6.1 at 44,100 Hz, with explicit engine buffers and `by Vendor` hosting.
Individual CLAP hosting overrides were off for both Harmonigraph classes.
Every trial placed the three tuner instances and the Master hub in one recorded process, with source callbacks moving between audio threads.
The Harmonigraph editor was never opened.
Physical headphone monitoring was disabled and cue/preview volume set to minimum;
WAV measurements were numerical and no captures were played.

The disposable project has a tuner before Polysynth, one before Vital x64 VST3, and one before an Instrument Layer containing Polysynth and FM-4. The full Harmonigraph CLAP is on Master.
Three clips produce five attacks each at 2, 4, 6, 10 and 12 seconds.
The four long notes last 44,100 samples;
Bitwig represents the short imported note as 2,757 samples.
Note IDs are present and the source keys are 60, 64 and 67. The initial accidental one-second transport loop was disabled before the measured attacks.

The candidate delay was fixed at 2,048 samples per activation, or 46.440 ms. Every source requested continuous callbacks and published silence progress.
All clock offsets initially equalled zero.
The raw evidence and disposable projects are locally archived under `/tmp/harmonigraph-615-evidence`;
the directory numbers below identify separate closed trace sets.

## Completed scheduling trials

| Traces | Build | Engine buffer | Mode | Tuned attacks accepted | Reply visibility after input callback | Observed deadline margin |
| --- | --- | --- | --- | --- | --- | --- |
| 01–03 | `637d4448` | 512 samples | Playback / real-time export | 15 per trial | 512 samples | 1,672–1,944 samples |
| 05 | `b664e1e3` | 512 samples | Offline export | 15 | 512 samples | 1,672–1,944 samples |
| 06 | `b664e1e3` | 128 samples | Real-time and offline export | 15 per mode | 128 samples | 1,928–1,968 samples |
| 07, normal replies | `b664e1e3` | 1,024 samples | Real-time and offline export | 14 per mode | 1,024 samples | 1,160–1,840 samples |

These are observed margins for the fixture's event offsets, not worst-case guarantees for arbitrary future scheduling.
All accepted non-late note edges were exactly input plus 2,048 samples, with preserved note ID, channel and key.
Every note-on had an accepted +0.5-semitone expression at the same output offset.
The hub made each normal assignment in the input callback's mapped interval;
the source consumed it in its next callback.
The host queried the CLAP latency extension and observed the tuners' 2,048-sample latency.

In the equal-branch real-time trials, matching source and hub callbacks by their process-local monotonic timestamps found equal steady clocks.
Their raw musical transports differed by the reported 2,048-sample compensation.
Mapping accepted output to the Master transport recovered the original attack times, and the first nonzero exported audio was at exactly 2 seconds.
This supports the candidate mapping for those graphs;
it does not turn CLAP's per-instance clock into a portable shared epoch.

Offline export in trial 05 processed the capture in approximately 0.414 seconds.
Nearest wall-time callback matching becomes ambiguous at that speed, often selecting an adjacent hub block.
Equal-steady transport matching was consistent with compensation, but is a conditional check of the candidate mapping rather than independent clock proof.
The two mode changes skipped 4,608 steady-clock samples in every instance.
These shared jumps occurred between activations and must not be described as gap-free continuity.

## Valid late answer

Trial 07 withheld source 0's first assignment until its deadline plus 50,000 samples.
In both real-time and offline export, hub publication rounded that threshold up by 40 samples, and the next source callback observed the reply 1,024 samples later.
The accepted attack and tuning were emitted at offset zero of that first available callback.
The resulting extra shift was 51,064 samples, and total input-to-output delay was 53,112 samples.
Exactly one `assignment_deadline_missed` diagnostic occurred per mode.

The note-off had arrived 9,012 samples before the reply became visible.
It remained queued and the emitted note still lasted exactly 44,100 samples.
All subsequent source 0 edges retained the same extra shift;
the other two sources retained the original 2,048-sample delay.
This verifies a usable late answer and the apparatus's whole-stream translation, not a production policy for returning to D or preserving already-sounding voices.
No CC arrived while this assignment was pending, so this run does not cover queued expression before the late reply.

## Pitch conversion

With Vital's MPE conversion disabled, the Master recording contained E4 at approximately 329.6276 Hz despite the accepted CLAP tuning event.
Polysynth and the Instrument Layer produced the intended approximately +50-cent pitches.
Enabling MPE in both Bitwig and Vital initially moved E4 to approximately 339.3061 Hz, or +50.10 cents.
That pitch was present in the first 100 ms as well as sustained windows.
The mixed recording does not isolate the first oscillation or prove first-sample instrument behavior.

After reloading the project with Vital's own BEND value still at 2, the measured shift became approximately +2.087 cents in both real-time and offline exports, even though both MPE switches and Bitwig's 48-semitone range remained enabled.
Setting Vital's BEND explicitly to 48 restored approximately +50.10 cents, including after another project reload and offline export in trials 08–09. The measured working setup therefore requires both MPE switches and matching explicit bend ranges.
Host event acceptance alone is insufficient to validate the destination.

## Branch latency and clock calibration

In trial 09, all five baseline onsets had equal raw mapped input samples on the three tracks.
A Time Shift of -1,536 samples after Vital moved that track's inputs earlier by exactly 1,536 samples, and increased its displayed compensation from 46.4 to 81.3 ms. Time Shift intentionally advances the audio;
that trial is not evidence that an ordinary effect's latency was compensated audibly.

After removing Time Shift, enabling ValhallaShimmer left the branch compensation at 46.4 ms. Adding Bitwig's native Peak Limiter raised it to 47.9 ms, and that track's input samples preceded the other tracks by exactly 64 samples.
Using zero offsets therefore splits a musically simultaneous group in the probe's input time domain.
An explicit +64-sample source clock offset is being measured against that fixed routing.
Automatic calibration and recovery after arbitrary graph edits are not established.

## Framework defect exposed by offline export

Trials 01, 02 and 04 stalled before any offline callback, including a Master-only export.
The completed preceding callbacks had no queued or sounding notes.
Probe-only main-thread diagnostics in `3954fe18` identified three initial latency restart requests;
the tuners never reached the later offline render-mode call.
This eliminated the suspected render-mode change as the source of those requests.

The CLAP wrapper marked itself active before dropping its initialization context.
That context then published the initial latency, which the wrapper treated as an active-state latency change requiring another restart.
Commit `b664e1e3` releases the plugin lock and drops the context before setting the active flag, so the notification happens during activation as the [CLAP latency contract](https://github.com/free-audio/clap/blob/main/include/clap/ext/latency.h) requires.
The exported-CLAP fixture now exposes a host latency extension and counts notifications and restart requests:
it failed with one unexpected restart before the fix and passed afterward.
Fresh Bitwig instances then completed offline export at every tested buffer size.
This framework fix is required to run the requested offline experiment.

## Trace integrity and outstanding cases

All completed measured instances have balanced callback traces, complete footers, zero reported record loss and successful file I/O.
Trials 01–09 used duplicate outer `sequence` keys on input records;
sequence-continuity checks retained the first value, while the second was the input stream sequence.
The trace field is renamed to `input_sequence` for subsequent builds.

Still required are individual late insertion and reactivation, explicit sub-block transport/automation splits and boundary offsets in Bitwig, stopped live input, loop and seek behavior with notes, mute/solo/sleep/bypass/removal with pending and sounding voices, same-key overlap, and initial/later note expression ordering in the host.
Hostless coverage of these paths is apparatus validation only.

The same-session grouping evidence complements [#623 Test A](https://github.com/yan-h/harmonigraph/issues/623#issuecomment-5547518334).
Discovery of the newly exported class required touching the bundle's `Contents/Info.plist` before restarting Bitwig;
the loader's unchanged metadata fingerprint is recorded separately in [#631](https://github.com/yan-h/harmonigraph/issues/631).
