#[path = "@REPO@/crates/harmonigraph-offline/src/sink.rs"]
mod sink;
use sink::{Sink, VideoOptions};
fn main() {
    let path = std::path::Path::new("@OUTPUT@/probe.png");
    let options = VideoOptions {
        size: [4, 4],
        fps: 60.0,
        frames: 3,
        audio: None,
        crf: 10,
        ffmpeg: None,
        audio_offset: 0.0,
    };
    let mut first = Sink::create(path, &options).unwrap();
    for _ in 0..3 {
        first.push(vec![255; 4 * 4 * 4]).unwrap();
    }
    first.finish(|_| {}).unwrap();
    let options = VideoOptions {
        frames: 1,
        ..options
    };
    let mut second = Sink::create(path, &options).unwrap();
    second.push(vec![0; 4 * 4 * 4]).unwrap();
    second.finish(|_| {}).unwrap();
    let mut names: Vec<_> = std::fs::read_dir(path.parent().unwrap())
        .unwrap()
        .filter_map(Result::ok)
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n.starts_with("probe-"))
        .collect();
    names.sort();
    println!("{}", names.join("\n"));
}
