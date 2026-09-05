# #615 Bitwig measurements

**Verdict:
viable with narrower constraints.** In the measured Bitwig graph, complete source intervals, central assignment and accepted delayed output fit a fixed D of 2,048 samples (46.440 ms).
This establishes a usable host configuration for #615, not a portable CLAP clock guarantee or a finished tuning feature.
The production transport-stop and late-stream contract remains unresolved in [#632](https://github.com/yan-h/harmonigraph/issues/632) and must be settled before #616 implements it.
The apparatus is in [draft PR #630](https://github.com/yan-h/harmonigraph/pull/630), and its configuration and limitations are in [tuning-probe.md](tuning-probe.md).

At this setting, live notes passing through the tuner gain 2,048 samples (46.440 ms) of input-to-instrument delay, in addition to the rest of the monitoring path.
Bitwig's measured compensation aligns scheduled playback and export;
it cannot anticipate a live key press or remove this delay from live playing.
The stopped-input trial confirms the delayed event path for one zero-duration gesture, not the total keyboard-to-audio latency of a performance setup.
The investigation retained D = 2,048 across all tested engine buffers and did not search for the minimum reliable delay.
Reducing the live-playing cost requires fresh measurements at smaller D values under explicitly supported conditions.

## Supported configuration and limits

| Area | Measured support / required constraint |
| --- | --- |
| Host and topology | Bitwig Studio 6.1 on macOS, 44,100 Hz, `by Vendor`, one Master hub and three tuners in the same process; individual hosting overrides off |
| Engine interval | Explicit fixed 128-, 512- or 1,024-sample buffers, with negotiated minimum equal to maximum; no larger or variable enclosing callback is claimed |
| Delay and margin | D = 2,048 samples per activation; normal replies visible within the next two source callbacks, with a smallest observed margin of 1,021 samples; this is a measured candidate, not a proven minimum or future scheduling guarantee |
| Time mapping | Raw `steady_time` + configured instance offset + Rust sub-block start + local event offset; offsets remain fixed within an activation |
| Unequal branches | Measured 64-sample limiter branch uses source B offset +64; raw physical input-to-output delay remains 2,048 samples |
| Input completeness | Three expected participants publish exclusive endpoints, including silence; unavailable callbacks stall completeness rather than imply an empty interval |
| Processing modes | Playback, real-time/offline export, the measured loops, idle seeks and the stopped zero-duration live-input gesture; the editor remained closed |
| Lifecycle | Idle insertion/reactivation, held mute/solo/bypass/removal and pending reset measured; resume only with known lifetimes and a validated clock mapping |
| Sleep and routing edits | Explicitly request continuous callbacks; actual sleep/wake, automatic clock calibration and arbitrary graph changes remain outside the proven configuration |
| Pitch destination | Polysynth, Instrument Layer and Vital VST3 measured; Vital requires host/instrument MPE enabled and matching explicit 48-semitone bend ranges |
| Boundaries and identity | Real automation sub-blocks, equal local offsets at distinct times, sample zero, sample 1,023 and cross-callback events measured; attempted same-key overlap arrived as ordered release/retrigger, with attack IDs preserved |
| Transport and reset limits | Bitwig supplied only callback-start transport; nonzero-offset transport events, a manual seek while held and changed hub epochs with unknown downstream voices are not declared supported |

The fixed topology and reset preconditions matter more than the nominal D.
Mute and solo preserve a lifetime;
missing callbacks do not prove its release.
Revalidate offsets after changing branch latency, and do not resume an unknown held set based only on local queue cleanup.
The long-late-answer tests validate a usable answer and a visible timing fault;
they do not select a production return-to-D or cancellation policy.

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

The candidate delay was fixed at 2,048 samples per activation, or 46.440 ms. Unless noted otherwise, every source requested continuous callbacks and published silence progress.
All clock offsets initially equalled zero.
The [evidence archive](evidence/615/README.md) preserves the raw traces, disposable projects, analyses and measured source history with checksums and a run index.
The original capture directory was `/tmp/harmonigraph-615-evidence`;
the directory numbers below identify separate closed trace sets in the archive.

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
Trial 10 applied an explicit +64-sample source clock offset to that fixed routing.
All ten real-time/offline cohorts then had equal mapped samples and returned to A/B/C assignment order, while physical output still occurred exactly 2,048 samples after raw input.
All 60 note edges, 15 forwarded CCs and 30 added tuning events were accepted.
The observed reply margin was 1,160–1,840 samples on B and 1,224–1,904 on A/C.
Both captures began at exactly 2 seconds;
the mixed signal and reverb tails do not isolate B's individual compensation.
Automatic calibration and recovery after arbitrary graph edits are not established.

An independent ordering check of trial 10 found every A/B/C callback-entry permutation across 29,264 complete raw-clock intervals.
The ten simultaneous attack cohorts arrived in four different source orders, but all were assigned A/B/C.
Every assignment followed three exclusive progress endpoints strictly beyond the attack sample, with frontier margins of 144–824 samples.
Callback arrival order therefore did not omit a simultaneous source in these measured cohorts.

## Independent idle reactivation

Trial 11 deactivated source A with Bitwig's `Active` control, reactivated it, removed it, and restored it with Undo.
The hub and B/C remained loaded and processing throughout.
Both replacement A instances began their local callback count at one but immediately received the running engine's steady clock:
1,283,072 and 2,325,504 samples respectively.
Independent nearest-time pairing with the hub found equal raw steady time throughout all 819 and 3,225 callbacks of those replacement instances.
The two gaps without A contained 603 and 199 uninterrupted hub callbacks and continuing B/C progress, with no A progress.

A's generations advanced from 2 to 4 to 6;
B/C remained at 2. All 15 attacks played after reinsertion aligned in complete cohorts, retained D = 2,048 and preserved tuning identity and duration.
The observed reply margin remained 1,160–1,840 samples.
The hub recorded nonzero audio in the corresponding note-on blocks and exact silence after musical time 12.376 seconds, with no nonfinite samples.
All six instances closed without faults or trace loss.
These replacements occurred without pending or held notes, and do not establish safe replacement while a voice is alive.

## Held voices, bypass, mute and removal

Trials 12–14 isolated Polysynth on A with a single note lasting 60 seconds at 20 BPM, from musical time 12 to 72 seconds.
B/C had no clips but continued processing as silent participants.
The hub's numerical audio-level trace observes the internal stereo signal before disabled hardware monitoring;
it bounds transitions by a 1,024-sample block and does not expose the instrument's private voice state.

In trial 12, disabling the tuner's `Enable` control during the held note sent it a note-off and stopped its callbacks before that delayed release could be emitted.
The hub nevertheless reached an entirely zero block about 325 ms after the input release, or 302 ms after processing stopped.
On re-enable, the tuner's actual reset record reported one queued event and one held lifetime and cleared them.
Its callbacks rejoined the current engine clock approximately 20.27 seconds before the note's natural end.
No attack was retriggered, and every subsequent measured hub block stayed zero.
This host bypass operation terminated the observed signal independently of the tuner's delayed output.

In trial 13, track mute silenced the held note from musical time 25.843810 to 42.167438 seconds;
soloing silent track B silenced it again from 49.203084 to 66.478730 seconds.
All four instances continued processing, and every source watermark reached the end of every callback during both intervals.
Neither operation caused a reset, note-off, new attack or generation change.
Audio returned with the same held lifetime when mute/solo were removed.
The natural note-off still arrived at 72 seconds and was accepted exactly D later, preserving the 2,646,000-sample note duration.

In trial 14, removing the tuner logged one held lifetime and no queued event, without any received or emitted tuner note-off.
The observed downstream signal became zero approximately 309 ms after deactivation.
Undo created a fresh instance approximately 27.21 seconds before the original note's end, immediately aligned with the running hub clock.
No attack or natural release was delivered to that new instance, no unknown-note fault occurred, and the audio remained zero.
The 777 hub/B/C callbacks while A was absent were uninterrupted.
These specific host actions provide cancellation boundaries;
they do not license treating every missing callback or mute as a dead voice.

## Queued expression, release and transport stop

Trial 16 withheld the isolated ten-second note's assignment for 1,048,576 samples beyond its deadline.
The release arrived approximately 13.886 seconds before the reply became visible;
three tuning events and a CC also arrived while the attack waited.
Hub publication rounded the release threshold up by 888 samples, and the next source callback accepted the attack at offset zero.
Total delay was 1,052,536 samples, including an extra shift of 1,050,488 samples.
The accepted release preserved the exact 441,000-sample input duration.
The three +0.25-semitone inputs became +0.75-semitone outputs at attack-relative offsets zero, zero and 888 samples, with the added +0.5 correction preceding them.
There was exactly one intentional deadline diagnostic.
The raw trace identifies the queued CC's class and timing but does not record its controller/value bytes.

Trial 15 stopped transport after a similarly withheld attack had already sounded.
The apparatus's retained stream shift delayed the host's stop-generated note-off too:
the isolated downstream signal continued for approximately 24.125 seconds after transport stopped, until the translated release and instrument tail completed.
This trial does not cover stopping an unanswered attack.
It exposes a required production decision rather than selecting a fallback:
[#632](https://github.com/yan-h/harmonigraph/issues/632) records the exact input, output and audio-level evidence for transport-stop cancellation of pending and held lifetimes.

## Pending reset with an unavailable source

Trial 17 deactivated silent source C before playback while the expected membership remained three.
The hub could not complete A's attack cohort, and A retained the attack, three tuning events, release and CC.
Bypassing and resuming A logged a real reset with six pending events and no held lifetime.
Its generation changed from 2 to 3;
the old request was never assigned or emitted, including after C returned.
All 4,003 hub callbacks while C was absent, and the subsequent 1,178 callbacks before a fresh attack, contained zero audio.

A resumed on the hub's current steady clock, and independently reactivated C did likewise.
The new A attack used generation 3 and a different host note ID;
all its events were accepted exactly 2,048 samples after input, with a 1,160-sample reply margin and the original ten-second duration.
The missing participant caused one expected deadline diagnostic.
The result establishes cancellation through the observed reset and successful new work after membership restoration;
it does not establish automatic recovery of the unanswered original request.

## Automation, loops and stopped input

Trial 18 automated `Probe split`, producing sixteen 64-sample Rust sub-blocks within a 1,024-sample CLAP callback.
Every partition covered its enclosing callback exactly, and every source published the correct exclusive endpoint.
All 23 performance inputs were accepted exactly D later, including events in nonzero sub-blocks.
An attack at raw callback offset 1,016 and an expression at the next callback's offset zero preserved their eight-sample separation through delayed output.
Two 14-second transport wraps preserved steady time and source generations;
the hub observed each wrap 2,048 raw samples after the sources, consistent with compensation.

Trial 19 shortened the loop to four seconds, cutting the two-to-twelve-second note while it was held.
Eight loop cuts delivered a host release at musical time four seconds before the next attack.
Each accepted output preserved the resulting 88,200-sample duration and D = 2,048. Two manual seeks also preserved steady time, but occurred after the note's natural release;
they are idle-seek evidence only.
All 11 assignments had complete source frontiers, and every output was accepted without a timing fault.

With transport stopped, computer-keyboard input delivered one key-36 attack and release at the same input sample, both at callback offset zero.
The host accepted note-on, +0.5 tuning and note-off in that order exactly 2,048 samples later.
The isolated downstream signal returned to exact zero eight hub blocks later and stayed zero throughout the remaining 1,815 blocks.
This establishes a stopped live-input request/reply path for the measured zero-duration gesture, not a general bound for arbitrary live monitoring graphs.

Trial 20 returned `CLAP_PROCESS_CONTINUE_IF_NOT_QUIET` instead of requesting continuous processing.
Bitwig nevertheless kept all four instances processing, including approximately 19.48 seconds of initial stopped silence and 35.90 seconds of final stopped silence.
All 3,422 common hub callbacks had complete source intervals, and the one performed note retained its duration and D.
Actual suspension and wake-up were not observed;
the supported configuration therefore explicitly requests continuous callbacks.

All observed enclosing callbacks had the activation's fixed negotiated size.
Transport records remained at offset zero, including the loop and seek trials.
The framework advances its derived song position by the sub-block start even while stopped;
that is wrapper extrapolation, not a measured host transport movement.
The probe uses raw steady time for scheduling.
Variable Rust sub-blocks, variable enclosing callbacks and nonzero-offset transport events are distinct cases;
only the first was observed here.

## Changing pitch expression across sub-blocks

Trial 21 gave the isolated ten-second Polysynth note a pitch ramp from +0.25 to +1.25 semitones while `Probe split` automation remained active.
Both real-time playback and offline export delivered 3,103 tuning inputs.
Every output was accepted at input plus 2,048 samples with exactly the floating-point sum of the input expression and the central +0.5 correction.
Both passes preserved the 441,000-sample note duration and supplied 345 callbacks containing distinct performance events at the same local offset in different sub-blocks.
For example, local offset eight in sub-blocks starting at 128 and 256 mapped to input samples 22,846,600 and 22,846,728;
accepted outputs were 22,848,648 and 22,848,776. The 128-sample separation survived both mappings.

The isolated offline recording first became nonzero at exactly two seconds.
Numerical spectral windows measured approximately +74.95 cents near the attack and +174.60 cents after the ramp, consistent with the intended +75-to-+175-cent result.
Intermediate windows followed the traced pitch within approximately 0.75 cents.
These estimates support early and sustained pitch behavior;
they do not identify the instrument's response on its first oscillator sample.
Mode changes caused shared positive steady-clock gaps between activations, with no in-activation gap or probe fault.

## Same-key retrigger and isolated VST3 compensation

Trial 23 placed Bitwig's native Note Length before tuner A, with tempo synchronization off, trigger on press and a confirmed five-second duration.
The clip retriggered key 60 at two, four, six, ten and twelve seconds, attempting overlap upstream of the tuner that advertises no overlapping-voice support.
Raw input instead released each previous lifetime before the new attack, with resulting durations of two, two, four, two and five seconds.
Attack IDs were 16, 32, 48, 64 and 80;
releases used the channel/key address with wildcard note ID -1. Accepted output preserved release, new attack and new-ID tuning in that order at the same delayed sample.
All 15 cross-track assignments had complete input frontiers and all outputs retained physical D = 2,048. No same-key overlap reached the probe;
without an upstream control trace, this does not identify which Bitwig stage performed the termination.

Trial 24 isolated Vital by removing A/C's clips while leaving their tuners processing.
ValhallaShimmer was bypassed and Peak Limiter retained its 64-sample branch latency, with B's clock offset still +64. Both 14-second Master exports first became nonzero at sample 88,201, one sample after two seconds.
Each of the five attacks began one sample after its nominal position, with silence beforehand;
there was no residual 64-sample branch displacement.
Early 50-ms windows and sustained windows measured approximately +50.10 cents in both real-time and offline captures.
This isolates the destination and branch compensation that the earlier mixed recordings could not separate.
It still does not prove a literal first-oscillator-sample pitch response.

A final playback pass at displayed tempo 121.6 BPM placed the first attack at callback offset zero and the next two attacks at the exact last sample, offset 1,023. This was a real Bitwig event-boundary measurement, independent of the hostless fixture.
For note IDs 192 and 208, accepted raw outputs were 15,533,055 and 15,620,095, exactly 2,048 samples after inputs 15,531,007 and 15,618,047. Both attacks and their ID-addressed tuning retained output offset 1,023. These calibrated B inputs map into the following common interval, requiring later A/C silence progress before the hub can assign them;
their reply margin was 1,023 samples.
The final two attacks, at offsets 1,022 and 1,021, took the same extra callback and reduced the smallest observed margin to 1,021 samples.
For the minimum case, B's mapped request was 61 samples beyond A/C's current exclusive frontier;
the hub correctly waited for their next intervals, then the second following B callback received the reply.
Accepted physical output still occurred at input plus 2,048 samples.
Across all 15 trial-24 attacks, eleven replies took one following source callback and four took two.
The archived project preserves this tempo separately from the normal 120-BPM audio-capture fixture.

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

## Trace integrity and excluded cases

All completed measured instances have balanced callback traces, complete footers, zero reported record loss and successful file I/O.
Trials 01–10 used duplicate outer `sequence` keys on input records;
sequence-continuity checks retained the first value, while the second was the input stream sequence.
The trace field is renamed to `input_sequence` for subsequent builds.

Unobserved cases are excluded in the configuration table rather than counted as passing host tests.
In particular, this host never supplied nonzero-offset transport or a variable enclosing callback, sleep-eligible return values did not produce actual suspension, manual seeks happened after release, and the overlap attempt was already serialized before the tuner.
The exported-CLAP fixture exercises the framework's otherwise unobserved split/short-callback paths and adverse interleavings;
that remains apparatus validation rather than Bitwig scheduling evidence.
The empty MIDI-import attempt in archive 22 provides no timing evidence.
The retained opt-in apparatus and its exported-CLAP regression fixture provide a repeatable reader for the boundary hooks and subsequent #632 retiming experiments.

After the trials, Bitwig was left stopped, its original automatic 512-sample buffer preference restored, physical monitoring disabled and cue/preview volume at minimum.

The same-session grouping evidence complements [#623 Test A](https://github.com/yan-h/harmonigraph/issues/623#issuecomment-5547518334).
Discovery of the newly exported class required touching the bundle's `Contents/Info.plist` before restarting Bitwig;
the loader's unchanged metadata fingerprint is recorded separately in [#631](https://github.com/yan-h/harmonigraph/issues/631).
