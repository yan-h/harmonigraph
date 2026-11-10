use super::*;

/// Only manual requests override the recorded appearance.
#[test]
fn only_manual_requests_override_the_recorded_appearance() {
    let automatic = RenderRequest::recorded();
    let manual = RenderRequest::render_now("current appearance".into());
    for request in [&automatic, &manual] {
        assert_eq!(request.program, default_renderer_path());
    }
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

/// What a job's detail shows when its render fails is the renderer's own
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
/// its job's detail line, which is the only place a plugin user sees the
/// renderer's output.
///
/// The fixture is shaped like the real stream rather than like a warning on
/// its own. The warning is printed before the render starts, and what comes
/// after it is the counters and then ffmpeg's own summary — ffmpeg is a
/// grandchild sharing this pipe, so it gets the last word on every
/// successful render. `last` is therefore NOT the warning by the end, which
/// is the whole reason the warning is carried separately.
#[test]
fn a_render_that_warned_carries_it_onto_the_detail_it_succeeded_with() {
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

    let detail = rendered_detail(None, tail.warning.as_deref(), None);
    assert!(detail.contains("note history 6..=8 is missing"), "{detail}");
    // A clean render says nothing extra, or every export would read as one
    // that went wrong.
    assert_eq!(rendered_detail(None, None, None), "");
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
/// the job's detail holding whatever had arrived before the signal, for a
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
 archive) printf 'copy' > "${3%.mp4}-archive.mp4" ;;
 nocopy) echo 'warning: take ends mid-record' >&2; echo 'warning: no archive copy: ffmpeg exited with 1: Unknown encoder' >&2 ;;
esac
printf '%s %s' "$look" "${6:-unsized}" > "$3"
echo 'progress: 10/10 frames (100%)' >&2
"#).unwrap();
        std::fs::set_permissions(dir.join("renderer"), std::fs::Permissions::from_mode(0o755))
            .unwrap();
        Self {
            dir,
            control: Arc::new(RenderControl::default()),
            progress: Arc::new(Progress::default()),
        }
    }
    fn request(&self, look: &str) -> RenderRequest {
        RenderRequest {
            program: self.dir.join("renderer"),
            appearance: Some(look.into()),
            notice: None,
        }
    }
    fn enqueue(&self, request: RenderRequest) {
        self.enqueue_sized(request, None);
    }
    fn enqueue_sized(&self, request: RenderRequest, size: Option<[u32; 2]>) {
        spawn_render(
            request,
            self.dir.join("music.take"),
            size,
            None,
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
    f.enqueue_sized(f.request("first"), Some([16, 18]));
    f.enqueue_sized(f.request("retry"), Some([20, 22]));
    f.enqueue_sized(f.request("third"), Some([24, 26]));
    assert_eq!(
        f.jobs().iter().map(|j| j.size).collect::<Vec<_>>(),
        [Some([16, 18]), Some([20, 22]), Some([24, 26])]
    );
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
    assert_eq!(std::fs::read_to_string(&jobs[0].output).unwrap(), "first unsized");
    assert_eq!(std::fs::read_to_string(&jobs[2].output).unwrap(), "third unsized");
    assert_eq!(jobs[2].progress, RenderProgress { done: 10, total: 10 });
    f.control.retry(2, f.progress.clone());
    f.finished();
    let retried = f.jobs().into_iter().find(|j| j.id == 2).unwrap();
    assert_eq!(retried.state, ExportStatus::Completed);
    assert_eq!(retried.size, Some([20, 22]));
    assert_eq!(std::fs::read_to_string(retried.output).unwrap(), "retry unsized");
    assert_eq!(std::fs::read_to_string(f.dir.join("music.mp4")).unwrap(), "existing video");
    assert_eq!(std::fs::read_link(f.dir.join("music-1.mp4")).unwrap(), f.dir.join("missing-video"));
}

/// The renderer leaves an archive copy beside its output when the look asks;
/// the queue publishes it beside the video under the video's name, and a name
/// whose copy is already taken is skipped like a taken video's.
#[cfg(unix)]
#[test]
fn an_archive_copy_is_published_beside_its_video_without_overwriting_one() {
    let f = Fixture::new("archive");
    std::fs::write(f.dir.join("music-archive.mp4"), "earlier copy").unwrap();
    let gate = f.control.running.lock();
    f.enqueue(f.request("archive"));
    f.enqueue(f.request("plain"));
    assert_eq!(f.jobs()[0].output, f.dir.join("music-1.mp4"));
    assert_eq!(f.jobs()[1].output, f.dir.join("music-2.mp4"));
    // Another writer takes each chosen name's copy after enqueue: a plain
    // export must not land beside a copy that is not its own either.
    std::fs::write(f.dir.join("music-1-archive.mp4"), "raced copy").unwrap();
    std::fs::write(f.dir.join("music-2-archive.mp4"), "raced plain").unwrap();
    drop(gate);
    f.finished();
    let jobs = f.jobs();
    assert!(jobs.iter().all(|j| j.state == ExportStatus::Completed), "{jobs:?}");
    assert_eq!(std::fs::read_to_string(&jobs[0].output).unwrap(), "archive unsized");
    assert_eq!(std::fs::read_to_string(archive_path(&jobs[0].output)).unwrap(), "copy");
    assert!(!f.dir.join("music-1.mp4").exists());
    assert_eq!(std::fs::read_to_string(f.dir.join("music-archive.mp4")).unwrap(), "earlier copy");
    assert_eq!(std::fs::read_to_string(f.dir.join("music-1-archive.mp4")).unwrap(), "raced copy");
    assert!(!f.dir.join("music-2.mp4").exists());
    assert_eq!(std::fs::read_to_string(&jobs[1].output).unwrap(), "plain unsized");
    assert!(!archive_path(&jobs[1].output).exists());
    assert_eq!(std::fs::read_to_string(f.dir.join("music-2-archive.mp4")).unwrap(), "raced plain");
}

/// A copy that failed says so on its job's detail even when the take warned
/// first, and the take's warning stays beside it.
#[cfg(unix)]
#[test]
fn a_failed_archive_copy_is_in_the_detail_beside_the_takes_warning() {
    let f = Fixture::new("nocopy");
    f.enqueue(f.request("nocopy"));
    f.finished();
    let job = &f.jobs()[0];
    assert_eq!(job.state, ExportStatus::Completed);
    assert!(job.detail.contains("take ends mid-record"), "{}", job.detail);
    assert!(job.detail.contains("no archive copy: ffmpeg exited"), "{}", job.detail);
}

#[cfg(unix)]
#[test]
fn cancellation_reaps_descendants_cleans_only_owned_files_and_preserves_other_jobs() {
    let f = Fixture::new("cancel");
    let orphan = f.dir.join(format!("music.export-{}-1-0", std::process::id()));
    std::fs::create_dir(&orphan).unwrap();
    std::fs::write(orphan.join("keep"), "owned elsewhere").unwrap();
    f.enqueue(f.request("slow"));
    wait_until("descendant and partial output", || f.progress.read().is_some_and(|p| p.done == 1));
    f.enqueue(f.request("cancel pending"));
    f.enqueue(f.request("survivor"));
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
    assert_eq!(std::fs::read_to_string(completed).unwrap(), "survivor unsized");
}

#[cfg(unix)]
#[test]
fn cancellation_before_publication_and_shutdown_never_publish_or_start_late_jobs() {
    let f = Fixture::new("publish");
    let gate = f.control.before_publish.lock();
    f.enqueue(f.request("finished bytes"));
    wait_until("renderer to finish", || {
        f.progress.read().is_some_and(|p| p.done == 10) && f.control.child.lock().is_none()
    });
    f.control.cancel_job(1);
    drop(gate);
    f.finished();
    assert_eq!(f.jobs()[0].state, ExportStatus::Cancelled);
    assert!(!f.jobs()[0].output.exists());
    f.enqueue(f.request("slow"));
    wait_until("renderer to start", || f.progress.read().is_some_and(|p| p.done == 1));
    f.enqueue(f.request("pending"));
    f.control.shutdown();
    assert!(f.jobs().iter().all(|j| j.state == ExportStatus::Cancelled));
    assert!(f.control.child.lock().is_none());
    f.enqueue(f.request("late recorder completion"));
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
    let mut request = f.request("");
    request.appearance = None;
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
        f.request("other instance"),
        take,
        None,
        None,
        other_progress.clone(),
        other.clone(),
    );
    drop(gate);
    f.finished();
    wait_until("second instance", || !other.queue.lock().working);
    let first = f.jobs().remove(0);
    let second = other.snapshots(&other_progress).remove(0);
    assert_ne!(first.output, second.output);
    assert_eq!(std::fs::read_to_string(first.output).unwrap(), "recorded unsized");
    assert_eq!(std::fs::read_to_string(second.output).unwrap(), "other instance unsized");
    other.shutdown();
}
