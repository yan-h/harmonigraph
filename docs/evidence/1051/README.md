# #1051 signed CLAP range expansion in Bitwig

**Result: Bitwig Studio 6.1.1 on macOS rescales existing automation when a signed CLAP parameter expands from ±5 to ±10.** The raw values doubled after the range change and remained doubled after saving and reopening.
This disproves the proposed preservation behavior in this tested host,
even without nice-plug's zero-based index translation.
It does not establish how other CLAP hosts behave.

Measured on 2026-09-20 at 44,100 Hz in a separate disposable project.
The [probe](../../../tools/offset-range-probe/README.md) uses a unique class ID and vendor;
the production Harmonigraph executable and the user's project contents were not modified.
No plugin editor exists or was opened.
The test device outputs silence.

## Observations

| Trial | Advertised offset range | Smallest raw automation value | Largest raw automation value |
| --- | --- | --- | --- |
| Original envelope | −5 to +5 | −2.9999999701976776 | 4.380245804786682 |
| Same envelope after expansion | −10 to +10 | −5.999999940395355 | 8.760491609573364 |
| Saved expanded project reopened | −10 to +10 | −5.999999940395355 | 8.760491609573364 |

Both extrema doubled exactly in floating-point arithmetic.
The envelope had four points displayed as −3,
+3,
0 and +4,
connected by linear ramps.
Only the first point was entered numerically;
the others were drawn with the mouse.
Bitwig sent fractional values despite the stepped flag,
so the table deliberately records raw values instead of treating displayed integer labels as exact data.
This matters for the probe's truncating readback near integer boundaries,
but cannot explain the factor-of-two change in the raw host events.

The expansion trace records this sequence:

```text
request_restart
deactivate
range 5 -> 10; offset=4
rescan ALL (deactivated)
activate span=10 rate=44100
deactivate
activate span=10 rate=44100
get_info id=0 min=-10 max=10 active=true
```

There were no offset-envelope edits between the two playback trials.
The current unautomated value was retained across the range operation;
that does not preserve the host's stored envelope.
The plugin explicitly publishes the same signed parameter identity at both ranges.
It never multiplies incoming values by the span.

The reopened trace starts with:

```text
load span=10 offset=-3 active=false
range 5 -> 10; offset=-3
rescan ALL (deactivated)
activate span=10 rate=44100
get_info id=0 min=-10 max=10 active=true
```

Its first automated event is approximately −6.
Thus the widened span survives restore before processing,
and reopening does not recover the original absolute envelope values.

## Evidence and reproduction

The compressed logs contain only the probe's own host/version,
lifecycle and parameter traces:
[baseline](baseline.log.gz),
[expanded](expanded.log.gz),
and [reopened](reopened.log.gz).
No trace contains an overflow marker.
The baseline executable used the same signed input and trace paths but preceded the inactive-rescan correction;
no range edit or restore was attempted in that baseline capture.
The expansion and reopen trials used the corrected lifecycle present in the committed source.
Hostless regressions independently keep the mock host active throughout `deactivate` and reject any premature full rescan.

Decompressed SHA-256 hashes:

```text
0da6bf8d95008f361b6c0d5dc934cc6463dca80924a0d9a99d86f20be3276272  baseline.log
25f89452bc9ab1e88b1d01c558402629b6d60db54535978bf046f67a332786a3  expanded.log
f67fc6624b101a3d0874c18f9779a984d3528f012e597a52baee25b5ab998f40  reopened.log
```

The private disposable project snapshots are retained locally under `/tmp/harmonigraph-1051-evidence/` as `before-expansion.bwproject` and `after-expansion.bwproject`.
They are not committed because the project was created from the user's default template and retains unrelated master-track settings.
The [probe instructions](../../../tools/offset-range-probe/README.md) reproduce the experiment without those files.

This test does not measure relative automation,
modulation depth,
launcher clips,
other DAWs,
VST3 or range shrinking.
Those are not needed to reject the absolute-automation preservation claim already falsified here.

## Chosen product direction

Each axis has two independent additive automation lanes:

| Lane | Range | Step |
| --- | --- | --- |
| Offset | −9 to +9 | 1 |
| Extension | −90 to +90 | 10 |

The total is Offset + Extension,
covering every integer from −99 to +99 on each axis.
Both host parameter ranges stay fixed,
so using Extension preserves the existing Offset envelope.
The same design applies in every DAW;
no host detection or dynamic range rescan is needed.
This supersedes the initial per-axis ranges of ±5,
±3 and ±2 and the proposed warned-rescaling fallback.

The existing ±4096 Offset parameter IDs are narrowed once to ±9;
old automation is reinterpreted and out-of-range saved plain values are clamped.
That release transition is distinct from subsequently extending a passage with the new additive lane.

The [CLAP specification](https://github.com/free-audio/clap/blob/main/include/clap/ext/params.h) recommends plain-value storage but warns that range changes are not safe in every host.
`RESCAN_INFO` does not cover min/max changes;
the critical-change flag is `RESCAN_ALL` while inactive.
This corrects the notification proposed in the original issue.
