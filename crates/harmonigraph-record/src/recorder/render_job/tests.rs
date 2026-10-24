use super::*;
use harmonigraph_take::RenderConfig;

/// Only manual requests override the recorded appearance and output size.
#[test]
fn only_manual_requests_override_captured_settings() {
    let config = RenderConfig {
        frame: harmonigraph_take::RenderFrame { aspect_w: 9, aspect_h: 16, ..Default::default() },
        short_edge: 2160,
        ..Default::default()
    };
    let automatic = RenderRequest::recorded();
    let manual = RenderRequest::render_now(&config, "current appearance".into());
    for request in [&automatic, &manual] {
        assert_eq!(request.program, default_renderer_path());
    }
    assert_eq!(automatic.size, None);
    assert_eq!(manual.size, Some([2160, 3840]));
    assert_eq!(automatic.appearance, None);
    assert_eq!(manual.appearance.as_deref(), Some("current appearance"));
}

/// A renderer stream with CR-delimited progress, followed by diagnostics.
const RENDER_STDERR: &str = "probe.take: 1.6s of events -> 108 frames at 30 fps, \
         320x180 @ 1.00x -> probe.rgba\n\
         progress: 0/108 frames (0%)\n\
         \rprogress: 30/108 frames (27%)\rprogress: 60/108 frames (55%)\rprogress: 90/108 frames (83%)\
         \rprogress: 108/108 frames (100%)\n\
         done: 108 frames -> probe.rgba\n\
         timing: a 108-frame export in 3.2 s, 2.8 s of it drawing at 38.6 fps — \
         ui+tess 5.20 ms/frame (20%), submit 3.10 ms/frame (12%), \
         readback 12.40 ms/frame (48%), encode 4.90 ms/frame (19%)\n\
           encode with: ffmpeg -f rawvideo -pix_fmt rgba -s 320x180 -r 30 -i probe.rgba out.mp4\n";

#[test]
fn diagnostics_and_malformed_records_do_not_retarget_progress() {
    let progress = Progress::default();
    progress.begin();
    let warning = "warning: only 30 frames fit the soundtrack";
    let tail = follow(
        format!("progress: 0/300 frames (0%)\n{warning}\nprogress: broken\n").as_bytes(),
        &progress,
    );
    assert_eq!(progress.read(), Some(RenderProgress { done: 0, total: 300 }));
    assert_eq!(tail.warning.as_deref(), Some(warning));
    assert_eq!(tail.last, warning);
    follow(
        b"progress: 120/120 frames (100%)\ndone: 120 frames\ntiming: 300 frames drawn".as_slice(),
        &progress,
    );
    assert_eq!(progress.read(), Some(RenderProgress { done: 120, total: 120 }));
}

#[test]
fn a_renderer_without_progress_records_still_reports_diagnostics() {
    let progress = Progress::default();
    progress.begin();
    let tail = follow(b"done: 120 frames".as_slice(), &progress);
    assert_eq!(progress.read().unwrap().fraction(), None);
    assert_eq!(tail.last, "done: 120 frames");
}

#[test]
fn the_rewritten_counter_reaches_the_bar_as_the_render_goes() {
    let progress = Progress::default();
    progress.begin();
    follow(RENDER_STDERR.as_bytes(), &progress);
    assert_eq!(progress.read(), Some(harmonigraph_take::RenderProgress { done: 108, total: 108 }));

    progress.end();
    assert_eq!(progress.read(), None, "no bar once nothing is rendering");
}

/// What the status line shows when a render fails is the renderer's own
/// last word — which means the counters have to be passed over on the way
/// to it, however recently they were printed.
#[test]
fn a_failed_renders_last_word_is_kept_over_the_counters() {
    let stream = "take.take: 5.0s of events -> 300 frames at 60 fps, 640x360 @ 1.00x \
                      -> take.mp4\n\
                      \rprogress: 30/300 frames (10%)\rprogress: 60/300 frames (20%)\n\
                      harmonigraph-offline: ffmpeg exited with status 1\n";
    let progress = Progress::default();
    progress.begin();
    assert_eq!(
        follow(stream.as_bytes(), &progress).last,
        "harmonigraph-offline: ffmpeg exited with status 1"
    );
}

/// #712's other half: a render that WARNED and then succeeded says so on
/// the status line, which is the only place a plugin user sees the
/// renderer's output.
///
/// The fixture is shaped like the real stream rather than like a warning on
/// its own. The warning is printed before the render starts, and what comes
/// after it is the counters and then ffmpeg's own summary — ffmpeg is a
/// grandchild sharing this pipe, so it gets the last word on every
/// successful render. `last` is therefore NOT the warning by the end, which
/// is the whole reason the warning is carried separately.
#[test]
fn a_render_that_warned_carries_it_onto_the_status_line_it_succeeded_on() {
    let warning = "warning: take-1.take: note history 6..=8 is missing \
                       (PublicationFull) — rendering the records that survived.";
    let muxed = "video:512kB audio:0kB subtitle:0kB muxing overhead: 1.234567%";
    // The renderer's second warning, in the order `run()` prints them: the
    // take's own trouble, then the command line's. Present so "first, not
    // last" is a claim this fixture can tell apart.
    let later = "warning: take names take-1.wav but it is not beside the take";
    let stream = format!(
        "take-1.take: 2.0s of events -> 120 frames at 60 fps, 640x360 @ 1.00x \
             -> take-1.mp4\n\
             {warning}\n\
             {later}\n\
             \rprogress: 60/120 frames (50%)\rprogress: 120/120 frames (100%)\n\
             done: 120 frames -> take-1.mp4\n\
             {muxed}\n"
    );
    let progress = Progress::default();
    progress.begin();
    let tail = follow(stream.as_bytes(), &progress);
    assert_eq!(tail.warning.as_deref(), Some(warning), "the take's warning, not the flags'");
    assert_eq!(
        tail.last, muxed,
        "the warning is kept BESIDE the last word, which ffmpeg has taken",
    );

    let status =
        rendered_status(std::path::Path::new("/takes/take-1.mp4"), None, tail.warning.as_deref());
    assert!(status.contains("rendered /takes/take-1.mp4"), "{status}");
    assert!(status.contains("note history 6..=8 is missing"), "{status}");
    // A clean render says nothing extra, or every export would read as one
    // that went wrong.
    assert_eq!(
        rendered_status(std::path::Path::new("/takes/take-1.mp4"), None, None),
        "rendered /takes/take-1.mp4",
    );
}

/// A render killed mid-frame leaves its last counter unterminated. It is
/// still the truest thing known about how far it got, so the tail of the
/// stream counts even without a separator to end it.
#[test]
fn an_unterminated_last_counter_still_counts() {
    let progress = Progress::default();
    progress.begin();
    follow(
        "x.take: -> 300 frames at 60 fps\n\rprogress: 240/300 frames (80%)".as_bytes(),
        &progress,
    );
    assert_eq!(progress.read(), Some(harmonigraph_take::RenderProgress { done: 240, total: 300 }));
}

#[test]
fn the_default_renderer_path_is_where_update_plugin_installs_it() {
    let path = default_renderer_path();
    assert!(path.ends_with("Harmonigraph/harmonigraph-offline"), "{path:?}");
}

/// A signal arriving mid-read is not the end of the render, and a real
/// error is.
///
/// `Interrupted` means a signal landed — and one does, every time
/// `cancel_in_flight` kills a child — not that the renderer stopped
/// talking. Treating it as the end would freeze the bar partway and leave
/// the status line holding whatever had arrived before the signal, for a
/// render still going perfectly well. Treating a REAL error as a signal is
/// the opposite mistake: the loop would go round again on a pipe that has
/// nothing more to give.
#[test]
fn a_signal_mid_read_is_not_the_end_of_the_render() {
    /// A stderr pipe handing back a scripted sequence of chunks and the
    /// errors that arrive between them.
    struct Scripted {
        steps: std::vec::IntoIter<Result<&'static [u8], std::io::ErrorKind>>,
    }
    impl std::io::Read for Scripted {
        fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
            match self.steps.next() {
                Some(Ok(chunk)) => {
                    out[..chunk.len()].copy_from_slice(chunk);
                    Ok(chunk.len())
                }
                Some(Err(kind)) => Err(std::io::Error::from(kind)),
                None => Ok(0),
            }
        }
    }

    let progress = Progress::default();
    progress.begin();
    let signalled = Scripted {
        steps: vec![
            Ok(&b"x.take: -> 300 frames at 60 fps\n"[..]),
            Err(std::io::ErrorKind::Interrupted),
            Ok(&b"\rprogress: 240/300 frames (80%)\nharmonigraph-offline: ffmpeg died\n"[..]),
        ]
        .into_iter(),
    };
    assert_eq!(
        follow(signalled, &progress).last,
        "harmonigraph-offline: ffmpeg died",
        "everything after the signal still belongs to this render"
    );
    assert_eq!(progress.read(), Some(harmonigraph_take::RenderProgress { done: 240, total: 300 }));

    // A real error ends the read, so nothing past it is consumed.
    let broken = Scripted {
        steps: vec![
            Ok(&b"harmonigraph-offline: writing frames\n"[..]),
            Err(std::io::ErrorKind::BrokenPipe),
            Ok(&b"harmonigraph-offline: never read\n"[..]),
        ]
        .into_iter(),
    };
    assert_eq!(
        follow(broken, &progress).last,
        "harmonigraph-offline: writing frames",
        "a broken pipe is the end, not something to read past"
    );
}

/// The default renderer path is ABSOLUTE.
///
/// A relative one is resolved against the DAW's working directory, which is
/// the thing a fixed location exists to avoid: a host can start the plugin
/// from anywhere, so the same relative path finds the renderer on one
/// machine and nothing at all on the next.
#[test]
fn the_default_renderer_path_is_absolute() {
    // The absoluteness comes from `HOME`; with it unset there is nothing to
    // be absolute against and `.` is the documented fallback.
    let Ok(home) = std::env::var("HOME") else { return };
    let path = default_renderer_path();
    assert!(path.is_absolute(), "resolved against the host's cwd: {path:?}");
    assert!(path.starts_with(&home), "{path:?} is not under {home}");
}

#[cfg(unix)]
struct Fixture {
    dir: std::path::PathBuf,
    control: Arc<RenderControl>,
    progress: Arc<Progress>,
    status: Arc<Mutex<String>>,
}
#[cfg(unix)]
impl Fixture {
    fn new(name: &str) -> Self {
        use std::os::unix::fs::PermissionsExt;
        let dir =
            std::env::temp_dir().join(format!("harmonigraph-queue-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir(&dir).unwrap();
        std::fs::write(dir.join("renderer"), r#"#!/bin/sh
look=$(cat "$5")
case "$look" in
 retry) if [ ! -e "$1.retry" ]; then touch "$1.retry"; echo 'transient encoder failure' >&2; exit 7; fi ;;
 slow) sleep 300 & echo partial > "$3"; echo 'progress: 1/10 frames (10%)' >&2; wait ;;
esac
printf '%s %s' "$look" "$7" > "$3"
echo 'progress: 10/10 frames (100%)' >&2
"#).unwrap();
        std::fs::set_permissions(dir.join("renderer"), std::fs::Permissions::from_mode(0o755))
            .unwrap();
        Self {
            dir,
            control: Arc::new(RenderControl::default()),
            progress: Arc::new(Progress::default()),
            status: Arc::new(Mutex::new(String::new())),
        }
    }
    fn request(&self, look: &str, size: [u32; 2]) -> RenderRequest {
        RenderRequest {
            program: self.dir.join("renderer"),
            appearance: Some(look.into()),
            size: Some(size),
            notice: None,
        }
    }
    fn enqueue(&self, request: RenderRequest) {
        spawn_render(
            request,
            self.dir.join("music.take"),
            self.status.clone(),
            self.progress.clone(),
            self.control.clone(),
        );
    }
    fn jobs(&self) -> Vec<ExportJob> {
        self.control.snapshots(&self.progress)
    }
    fn finished(&self) {
        wait_until("queue to finish", || !self.control.queue.lock().working);
    }
}
#[cfg(unix)]
impl Drop for Fixture {
    fn drop(&mut self) {
        self.control.shutdown();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}
#[cfg(unix)]
fn wait_until(label: &str, mut condition: impl FnMut() -> bool) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !condition() {
        assert!(std::time::Instant::now() < deadline, "timed out: {label}");
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

#[cfg(unix)]
#[test]
fn fifo_variants_capture_settings_keep_existing_files_and_continue_after_failure() {
    let f = Fixture::new("fifo");
    std::fs::write(f.dir.join("music.mp4"), "existing video").unwrap();
    let gate = f.control.running.lock();
    f.enqueue(f.request("first", [16, 18]));
    f.enqueue(f.request("retry", [20, 22]));
    f.enqueue(f.request("third", [24, 26]));
    assert!(f.jobs().iter().all(|j| j.state == ExportStatus::Pending));
    assert_eq!(
        f.jobs()
            .iter()
            .map(|j| j.output.file_name().unwrap().to_string_lossy().into_owned())
            .collect::<Vec<_>>(),
        ["music-1.mp4", "music-2.mp4", "music-3.mp4"]
    );
    // Another writer wins a chosen destination after enqueue. Publication must
    // choose another name rather than overwrite it.
    std::os::unix::fs::symlink(f.dir.join("missing-video"), f.dir.join("music-1.mp4")).unwrap();
    drop(gate);
    f.finished();
    let jobs = f.jobs();
    assert_eq!(
        jobs.iter().map(|j| j.state).collect::<Vec<_>>(),
        [ExportStatus::Completed, ExportStatus::Failed, ExportStatus::Completed]
    );
    assert!(jobs[1].detail.contains("transient encoder failure"));
    assert_eq!(std::fs::read_to_string(&jobs[0].output).unwrap(), "first 16x18");
    assert_eq!(std::fs::read_to_string(&jobs[2].output).unwrap(), "third 24x26");
    assert_eq!(jobs[2].progress, RenderProgress { done: 10, total: 10 });
    f.control.retry(2, f.status.clone(), f.progress.clone());
    f.finished();
    let retried = f.jobs().into_iter().find(|j| j.id == 2).unwrap();
    assert_eq!(retried.state, ExportStatus::Completed);
    assert_eq!(std::fs::read_to_string(retried.output).unwrap(), "retry 20x22");
    assert_eq!(std::fs::read_to_string(f.dir.join("music.mp4")).unwrap(), "existing video");
    assert_eq!(std::fs::read_link(f.dir.join("music-1.mp4")).unwrap(), f.dir.join("missing-video"));
}

#[cfg(unix)]
#[test]
fn cancellation_reaps_descendants_cleans_only_owned_files_and_preserves_other_jobs() {
    let f = Fixture::new("cancel");
    let orphan = f.dir.join(format!("music.export-{}-1-0", std::process::id()));
    std::fs::create_dir(&orphan).unwrap();
    std::fs::write(orphan.join("keep"), "owned elsewhere").unwrap();
    f.enqueue(f.request("slow", [16, 16]));
    wait_until("descendant and partial output", || f.progress.read().is_some_and(|p| p.done == 1));
    f.enqueue(f.request("cancel pending", [18, 18]));
    f.enqueue(f.request("survivor", [20, 20]));
    f.control.cancel_job(2);
    f.control.cancel_job(1);
    f.finished();
    assert_eq!(
        f.jobs().iter().map(|j| j.state).collect::<Vec<_>>(),
        [ExportStatus::Cancelled, ExportStatus::Cancelled, ExportStatus::Completed]
    );
    assert!(orphan.join("keep").exists());
    assert_eq!(
        std::fs::read_dir(&f.dir)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| e.file_type().unwrap().is_dir())
            .count(),
        1,
        "only the unowned directory remains"
    );
    let completed = f.jobs()[2].output.clone();
    f.control.cancel_job(3);
    assert_eq!(std::fs::read_to_string(completed).unwrap(), "survivor 20x20");
}

#[cfg(unix)]
#[test]
fn cancellation_before_publication_and_shutdown_never_publish_or_start_late_jobs() {
    let f = Fixture::new("publish");
    let gate = f.control.before_publish.lock();
    f.enqueue(f.request("finished bytes", [16, 16]));
    wait_until("renderer to finish", || {
        f.progress.read().is_some_and(|p| p.done == 10) && f.control.child.lock().is_none()
    });
    f.control.cancel_job(1);
    drop(gate);
    f.finished();
    assert_eq!(f.jobs()[0].state, ExportStatus::Cancelled);
    assert!(!f.jobs()[0].output.exists());
    f.enqueue(f.request("slow", [16, 16]));
    wait_until("renderer to start", || f.progress.read().is_some_and(|p| p.done == 1));
    f.enqueue(f.request("pending", [16, 16]));
    f.control.shutdown();
    assert!(f.jobs().iter().all(|j| j.state == ExportStatus::Cancelled));
    assert!(f.control.child.lock().is_none());
    f.enqueue(f.request("late recorder completion", [16, 16]));
    assert_eq!(f.jobs().len(), 3);
}

#[cfg(unix)]
#[test]
fn two_instances_publish_distinct_videos_and_recorded_appearance_is_captured_at_enqueue() {
    let f = Fixture::new("instances");
    let take = f.dir.join("music.take");
    let header =
        harmonigraph_take::Header { appearance: Some("recorded".into()), ..Default::default() };
    harmonigraph_take::Writer::create(&take, &header).unwrap().flush().unwrap();
    let gate = f.control.running.lock();
    let mut request = f.request("", [16, 16]);
    request.appearance = None;
    request.size = None;
    f.enqueue(request);
    harmonigraph_take::Writer::create(
        &take,
        &harmonigraph_take::Header { appearance: Some("changed after enqueue".into()), ..header },
    )
    .unwrap()
    .flush()
    .unwrap();
    let other = Arc::new(RenderControl::default());
    let other_progress = Arc::new(Progress::default());
    spawn_render(
        f.request("other instance", [18, 18]),
        take,
        f.status.clone(),
        other_progress.clone(),
        other.clone(),
    );
    drop(gate);
    f.finished();
    wait_until("second instance", || !other.queue.lock().working);
    let first = f.jobs().remove(0);
    let second = other.snapshots(&other_progress).remove(0);
    assert_ne!(first.output, second.output);
    assert_eq!(std::fs::read_to_string(first.output).unwrap(), "recorded ");
    assert_eq!(std::fs::read_to_string(second.output).unwrap(), "other instance 18x18");
    other.shutdown();
}
