# harmonigraph-analysis

Rolling real-input audio analysis shared by the live UI and offline renderer.
`SpectrumAnalyzer` takes one transform of a Hann-windowed frame per column, zero-padded to twice its length,
and reads every bucket of the fixed pitch axis in `harmonigraph-core` at its own frequency through a Gaussian kernel;
`ChannelBank` averages independent channel power without phase cancellation.
The kernel also narrows the frame to the bucket's listen:
every bucket listens through one Gaussian toward the frame's newest end, as sharp as a Hann window two thirds of the frame and lagging as little.
`AnalyzerConfig::column_lag` says where on a time axis a column belongs, for the analyzer and the export alike.

RealFFT owns the planned half-size complex transform and real-input postprocessing.
Each channel retains its own plan and reusable input, output and scratch buffers.
Callers adopt one final `AnalyzerConfig` for all channels.
Only a window-size change replans the transform,
and a window-size or sample-rate change replans the kernels and the frame's window.
A sample-rate change resets the audio window;
a window-size change carries the newest samples across, and unchanged settings do nothing.
New channels start at the requested settings,
while a source restart clears retained audio and keeps its estimator resources.
There is no retained planner or shared plan cache.

This is the minimum extraction needed to leave the reusable pitch-math crate dependency-free.
Axis constants, Hz/MIDI conversion and spectrum history stay in core;
GUI and GPU types stay outside this crate.
The [adoption report](../../docs/realfft-adoption.md) records numerical, picture and cost comparisons.

## License

`MIT OR Apache-2.0`, preserving the moved code's license.
See [LICENSE-MIT](LICENSE-MIT) and [LICENSE-APACHE](LICENSE-APACHE).
RealFFT is MIT licensed;
its RustFFT dependency is MIT OR Apache-2.0.
