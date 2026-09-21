# Offset range host probe

This isolated CLAP plugin tests the assumption behind [#1051](https://github.com/yan-h/harmonigraph/issues/1051):
does a host preserve signed plain automation when a parameter's range expands?
It does not use nice-plug's zero-based integer adapter,
and it does not change the production Harmonigraph plugin.
The [Bitwig measurement](../../docs/evidence/1051/README.md) found that the host doubles old automation values when expanding from ±5 to ±10.

The probe advertises one signed stepped `Offset (steps)` parameter,
initially −5 to +5,
plus an `Expand range to +/-` control.
The latter is flagged automatable only because Bitwig hides non-automatable parameters from its generic device panel;
edit it manually and do not create an automation lane for it.
Expansion is monotonic up to 4096 within an instance;
restore can reinstate a smaller saved span.
The plugin always outputs silence.
Use a disposable project,
never insert it into a track whose audio you want to hear.

## Build and test on macOS

```sh
cargo test --manifest-path tools/offset-range-probe/Cargo.toml --locked
cargo build --release --manifest-path tools/offset-range-probe/Cargo.toml --locked
python3 tools/offset-range-probe/bundle.py
```

The bundle is `tools/offset-range-probe/target/Harmonigraph Offset Range Probe.clap`.
Copy that separate bundle to a host's CLAP scan directory,
or add the probe's target directory to its scan locations.
Never replace the production Harmonigraph bundle.
The probe has its own plugin ID and vendor name.
On rebuild,
stop the test host instance before installing a new bundle;
replacing mapped executable bytes in place is unsafe on macOS.

1. Add the probe to an otherwise silent disposable project.
2. Draw negative and positive offset automation,
   including a ramp,
   and play it through at ±5.
3. Stop playback and set the expansion control to 10.
   In Bitwig's generic device panel,
   double-click the knob itself to enter a number.
4. Confirm the host performs a restart and queries offset endpoints −10 and +10.
5. Replay without editing the offset envelope.
   Compare actual incoming values in the trace,
   not the unchanged screen position of its points.
6. Save,
   close and reopen the project,
   then replay without opening any plugin editor.
7. Check modulation separately before making any claim about modulation depth or clipping.

Each instance creates `/tmp/harmonigraph-offset-range-probe-<pid>-<serial>.log`.
The log identifies the host/version,
state restore,
activation,
range metadata and raw CLAP parameter events.
`kind=5` means a parameter value;
`kind=6` means a modulation amount.
Events have the process steady sample and within-block time,
or sample −1 for a flush.
An `INVALID: dropped ...` record invalidates that trace.
The bounded queue is written only by the host-serialized process/flush callbacks;
file writes occur on the main thread.

## Lifecycle and limits

An active range edit requests a restart.
`deactivate` only requests a callback:
CLAP still considers that callback active until it returns.
The new range and `RESCAN_ALL` are published either by a later inactive main-thread callback or at the start of `activate`,
which runs while inactive.
Active state loads stage the saved span/value and request the same transition.
The two hostless tests cover both callback orderings and a short-reading state stream.

This is measurement apparatus,
not the proposed production range implementation.
Its raw trace is authoritative for host input;
its integer readback truncates fractional host input as described by CLAP's stepped flag and can display a neighboring integer for a value such as `3.999999999999999`.
Do not use that readback alone to infer rescaling.
There is no VST3 probe or preservation claim here.
Remove this apparatus when the host question is settled and production coverage replaces it;
retain the measurements.
