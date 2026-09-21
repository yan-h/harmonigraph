//! Exercise the executable boundary as well as the encoder's unit fixtures.
use harmonigraph_take::{Header, NoteKind, NoteRecord, Record, RenderTrigger, WavWriter, Writer};
use std::path::Path;
use std::process::Command;

fn take(path: &Path, seconds: f64, trigger: RenderTrigger) {
    let mut appearance = harmonigraph_ui::AppearanceDocument::default();
    appearance.render.trigger = trigger;
    let mut wav = WavWriter::create(path.with_extension("wav"), 48_000.0, 1).unwrap();
    wav.write(&vec![0.0; (seconds * 48_000.0) as usize]).unwrap();
    wav.finish().unwrap();
    let mut take = Writer::create(
        path,
        &Header {
            appearance: Some(appearance.serialize()),
            audio_file: Some(
                path.with_extension("wav").file_name().unwrap().to_str().unwrap().into(),
            ),
            audio_start: Some(0.0),
            ..Default::default()
        },
    )
    .unwrap();
    for (t, kind) in [(0.0, NoteKind::On { velocity: 0.8 }), (0.25, NoteKind::Off)] {
        take.write(&Record::Note(NoteRecord {
            t,
            kind,
            note: 60,
            source: 1,
            ..Default::default()
        }))
        .unwrap();
    }
    take.flush().unwrap();
}

fn render(take: &Path, out: &Path, extra: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_harmonigraph-offline"))
        .arg(take)
        .arg("--out")
        .arg(out)
        .args(["--size", "320x180", "--fps", "20"])
        .args(extra)
        .output()
        .unwrap()
}

#[test]
fn cli_preserves_default_tail_explicit_end_late_start_and_loop_tail() {
    if Command::new("ffprobe").arg("-version").output().is_err() {
        eprintln!("skipping real export test: ffprobe unavailable");
        return;
    }
    let dir =
        std::env::temp_dir().join(format!("harmonigraph-cli-duration-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for (name, seconds, trigger, extra, frames) in [
        ("default-tail", 0.25, RenderTrigger::OnDisarm, vec![], 85),
        ("explicit-end", 10.0, RenderTrigger::OnDisarm, vec!["--end", "1"], 20),
        ("late-start", 0.25, RenderTrigger::OnDisarm, vec!["--start", "3", "--end", "4"], 20),
        ("loop-tail", 0.25, RenderTrigger::AtLoopEnd, vec![], 85),
    ] {
        let path = dir.join(format!("{name}.take"));
        take(&path, seconds, trigger);
        let out = path.with_extension("mp4");
        let result = render(&path, &out, &extra);
        let stderr = String::from_utf8_lossy(&result.stderr);
        if stderr.contains("no usable GPU adapter") {
            assert!(
                std::env::var("HARMONIGRAPH_REQUIRE_GPU").as_deref() != Ok("1"),
                "CI requires the actual CLI render to run: {stderr}"
            );
            eprintln!("skipping real export test: {stderr}");
            std::fs::remove_dir_all(dir).unwrap();
            return;
        }
        assert!(result.status.success(), "{name}: {stderr}");
        assert!(stderr.contains(&format!("done: {frames} frames")), "{name}: {stderr}");
        let probe = Command::new("ffprobe")
            .args([
                "-v",
                "error",
                "-select_streams",
                "v:0",
                "-show_entries",
                "stream=nb_frames",
                "-of",
                "csv=p=0",
            ])
            .arg(out)
            .output()
            .unwrap();
        assert!(probe.status.success());
        assert_eq!(String::from_utf8_lossy(&probe.stdout).trim(), frames.to_string());
    }
    std::fs::remove_dir_all(dir).unwrap();
}

/// Both zero and nonzero early exits must finalize the encoder after the
/// render's write error; a nonzero exit must lead with the encoder verdict.
#[test]
#[cfg(unix)]
fn cli_finalizes_the_encoder_after_a_render_write_error() {
    use std::os::unix::fs::PermissionsExt;
    let dir = std::env::temp_dir().join(format!("harmonigraph-cli-failure-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("take.take");
    take(&path, 0.25, RenderTrigger::OnDisarm);
    for exit in [0, 7] {
        let encoder = dir.join("ffmpeg.sh");
        // Closing stdin forces the >64KiB frame through the broken-pipe path.
        std::fs::write(&encoder, format!("#!/bin/sh\nexec 0<&-\nsleep 0.1\nexit {exit}\n"))
            .unwrap();
        std::fs::set_permissions(&encoder, std::fs::Permissions::from_mode(0o755)).unwrap();
        let result = render(&path, &dir.join("out.mp4"), &["--ffmpeg", encoder.to_str().unwrap()]);
        let stderr = String::from_utf8_lossy(&result.stderr);
        if stderr.contains("no usable GPU adapter") {
            assert!(
                std::env::var("HARMONIGRAPH_REQUIRE_GPU").as_deref() != Ok("1"),
                "CI requires the actual CLI render to run: {stderr}"
            );
            eprintln!("skipping real render test: {stderr}");
            std::fs::remove_dir_all(dir).unwrap();
            return;
        }
        assert!(!result.status.success(), "{stderr}");
        let error = stderr.lines().last().unwrap();
        assert!(error.contains("writing a frame to ffmpeg failed"), "{error}");
        if exit == 0 {
            assert!(error.contains("encoded 0 frames; expected 85"), "{error}");
        } else {
            assert!(error.contains("ffmpeg exited with exit status: 7"), "{error}");
        }
        assert!(!stderr.contains("done: 85 frames"));
    }
    std::fs::remove_dir_all(dir).unwrap();
}
