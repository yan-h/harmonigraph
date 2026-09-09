# harmonigraph-analysis

Rolling real-input audio analysis shared by the live UI and offline renderer.
`SpectrumAnalyzer` windows one channel and resamples accumulated taper power onto the fixed pitch axis in `harmonigraph-core`;
`ChannelBank` averages independent channel power without phase cancellation.

RealFFT owns the planned half-size complex transform and real-input postprocessing.
Each channel retains its own plan and reusable input, output and scratch buffers.
Configuration changes still reset the audio window;
unchanged size/taper settings do nothing.
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
