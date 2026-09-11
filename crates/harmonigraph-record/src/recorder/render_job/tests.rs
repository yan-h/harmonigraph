use super::super::channel;
use super::*;
use harmonigraph_take::RenderConfig;

#[test]
fn a_finished_take_always_renders() {
    // Auto-render is not gated: stopping a take always kicks off a
    // render (recorded audio + playhead), so from_config is always `Some`.
    let config = RenderConfig { auto_render: false, ..Default::default() };
    assert!(RenderRequest::from_config(&config).is_some());
}

/// A blank renderer path must fall back to the default rather than becoming
/// an empty argument the renderer would reject.
#[test]
fn blank_settings_fall_back_rather_than_passing_empty_arguments() {
    let config = RenderConfig { renderer_path: "  ".into(), ..Default::default() };
    let request = RenderRequest::from_config(&config).unwrap();
    assert_eq!(request.program, default_renderer_path());
    assert_eq!(request.audio, None);
    assert_eq!(request.size, config.frame.pixels(config.short_edge));
}

/// The pane's Aspect and Resolution are the whole of what sizes the video.
#[test]
fn the_frame_sizes_the_render() {
    let portrait_4k = RenderConfig {
        frame: harmonigraph_take::RenderFrame { aspect_w: 9, aspect_h: 16, ..Default::default() },
        short_edge: 2160,
        ..Default::default()
    };
    let request = RenderRequest::from_config(&portrait_4k).unwrap();
    assert_eq!(request.size, [2160, 3840]);
}

/// The renderer is never told to use the whole-song playhead.
///
/// It ORs `--playhead` with the take's own recorded setting, so a flag
/// passed here can only add. Passing it unconditionally would make the
/// Video pane's "Scrolling" choice unreachable: the pane writes
/// `spectrogram: Scrolling` into the take and the flag would turn the
/// playhead straight back on. Leaving it unset is what lets the row decide
/// every way, so this holds the request to saying nothing.
#[test]
fn the_take_decides_the_spectrogram_not_a_forced_flag() {
    use harmonigraph_take::SpectrogramRender;
    for spectrogram in
        [SpectrogramRender::Scrolling, SpectrogramRender::WholeVideo, SpectrogramRender::Playhead]
    {
        let config = RenderConfig { spectrogram, ..Default::default() };
        assert_eq!(RenderRequest::from_config(&config).unwrap().playhead, None);
        let now = RenderRequest::render_now(&config, "(dummy)".into());
        assert_eq!(now.playhead, None, "Re-render take must not force it either");
    }
}

/// A second request for the same take supersedes the first, which is what
/// makes "Render now" during a render mean restart rather than a second
/// render writing the same video — and a request for a DIFFERENT take
/// supersedes nothing, because it is not about that video at all.
#[test]
fn a_later_request_supersedes_the_one_in_flight() {
    let control = RenderControl::default();
    let take = std::path::Path::new("/takes/take-1.take");
    let other = std::path::Path::new("/takes/take-2.take");
    let first = control.claim(take);
    assert!(!control.superseded(take, first), "the only request in flight is current");

    let second = control.claim(take);
    assert!(control.superseded(take, first), "the first must stand down");
    assert!(!control.superseded(take, second), "the second is now the live one");

    // Another take's request stands beside it rather than over it.
    let elsewhere = control.claim(other);
    assert!(!control.superseded(take, second), "take-2's request is not about take-1");
    assert!(!control.superseded(other, elsewhere));

    // Generations are not reused, so a render cannot be revived by a
    // later one happening to land on its number.
    let third = control.claim(take);
    assert!(third > elsewhere && elsewhere > second && second > first);
    assert!(control.superseded(take, first) && control.superseded(take, second));

    // A finished run gives its claim back, and a stale one cannot take
    // away the claim that replaced it.
    control.release(take, second);
    assert!(!control.superseded(take, third), "a stale release must not unseat the live run");
    control.release(take, third);
    assert!(control.claims.lock().get(take).is_none(), "the finished take is not kept");
}

/// Cancelling with nothing running is a no-op rather than a panic: the
/// first render of a session takes exactly that path, since every request
/// cancels before it spawns.
///
/// The Video pane's button reaches the same state — it is drawn off a bar
/// the shell read at the top of the frame, so a render that ends before the
/// press is consumed is cancelled after it is already gone — and answers
/// that there was nothing to stop, which is what keeps the status line from
/// reporting a cancellation that cancelled nothing.
#[test]
fn cancelling_an_idle_render_control_does_nothing() {
    let control = RenderControl::default();
    control.cancel_in_flight(std::path::Path::new("/nowhere/take-1.take"));
    assert!(control.child.lock().is_none());
    assert!(!control.cancel(), "an idle control claimed to have stopped a render");
}

/// Spin until `done` or the budget runs out, so the concurrency tests
/// below wait on the thing they mean rather than on a fixed sleep.
#[cfg(unix)]
fn wait_until(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while std::time::Instant::now() < deadline {
        if done() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("timed out waiting for {what}");
}

/// Pressing "Re-render take" during a render kills the one in flight instead
/// of running a second alongside it.
///
/// Against a real process, because that is the whole claim: the generation
/// bookkeeping above proves only that the arithmetic is right, and the
/// thing that goes wrong — two renderers writing one video — lives
/// entirely in the part that spawns and kills.
#[cfg(unix)]
#[test]
fn a_second_request_kills_the_render_in_flight() {
    use std::os::unix::fs::PermissionsExt;

    let dir = std::env::temp_dir().join(format!("harmonigraph-cancel-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    // Stands in for the renderer: `exec` makes the tracked child the
    // sleeper itself, so killing it also closes the stderr pipe the render
    // thread waits on.
    let fake = dir.join("slow-renderer");
    std::fs::write(&fake, "#!/bin/sh\nexec sleep 300\n").expect("write fake renderer");
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).expect("chmod");

    let take = dir.join("take-1.take");
    let control = Arc::new(RenderControl::default());
    let status = Arc::new(Mutex::new(String::new()));
    let progress = Arc::new(Progress::default());
    let start = || {
        spawn_render(
            RenderRequest {
                program: fake.clone(),
                audio: None,
                align: None,
                appearance: None,
                size: [16, 16],
                playhead: None,
            },
            take.clone(),
            status.clone(),
            progress.clone(),
            control.clone(),
        )
    };

    start();
    wait_until("the first render to start", || control.child.lock().is_some());
    let first = control.child.lock().as_ref().map(|f| f.child.id()).expect("a first child");

    start();
    // The second cannot run until the first has been reaped, so seeing a
    // different process here is the cancellation having completed rather
    // than merely been asked for.
    wait_until("the second render to replace the first", || {
        control.child.lock().as_ref().is_some_and(|f| f.child.id() != first)
    });

    // The cancelled render must not have reported anything: its failure
    // was one we caused, and its replacement owns the status line.
    assert!(
        !status.lock().contains("failed"),
        "a cancelled render reported failure: {}",
        status.lock()
    );
    // And exactly one render is in flight, not two.
    assert!(progress.read().is_some(), "the bar still shows a render");

    control.cancel_in_flight(&take);
    wait_until("the last render to be reaped", || control.child.lock().is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

/// Cancelling a running render kills it and takes the part of the video it
/// had written with it.
///
/// Against a real process and real files, for the reason the supersede test
/// spawns one: the claim bookkeeping proves only that the arithmetic is
/// right, and what the button is for — a renderer still running, and an mp4
/// that is a fragment of one — lives entirely in the killing and the
/// cleanup.
///
/// The fixture asks for a `appearance` blob as well, so the sweep covers both
/// things a run leaves beside the take: a partial that is only ever renamed
/// into place on success, and the look a "Re-render take" wrote out for it.
#[cfg(unix)]
#[test]
fn cancelling_a_render_kills_it_and_deletes_what_it_had_written() {
    use std::os::unix::fs::PermissionsExt;

    let dir = std::env::temp_dir().join(format!("harmonigraph-cancelled-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    // Stands in for the renderer: writes at `--out` — argument 3, which is
    // where `spawn_render` puts it — and then never finishes, which is the
    // state the button exists for.
    let fake = dir.join("slow-renderer");
    std::fs::write(&fake, "#!/bin/sh\necho half a video > \"$3\"\nexec sleep 300\n")
        .expect("write fake renderer");
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).expect("chmod");

    let take = dir.join("take-1.take");
    let control = Arc::new(RenderControl::default());
    let status = Arc::new(Mutex::new(String::new()));
    let progress = Arc::new(Progress::default());
    spawn_render(
        RenderRequest {
            program: fake.clone(),
            audio: None,
            align: None,
            appearance: Some("(dummy)".into()),
            size: [16, 16],
            playhead: None,
        },
        take.clone(),
        status.clone(),
        progress.clone(),
        control.clone(),
    );
    // Everything the run put beside the take under a name of its own.
    let strays = || {
        std::fs::read_dir(&dir)
            .expect("the take's directory")
            .filter_map(Result::ok)
            .filter(|entry| {
                let name = entry.file_name().to_string_lossy().into_owned();
                name.contains("rendering-") || name.contains("rendernow-")
            })
            .count()
    };
    wait_until("the render to start", || control.child.lock().is_some());
    wait_until("the renderer to write some of the video", || strays() == 2);

    assert!(control.cancel(), "there was a render in flight to stop");
    wait_until("the part-written video to go", || strays() == 0);
    wait_until("the cancelled render to be reaped", || control.child.lock().is_none());
    assert!(progress.read().is_none(), "the bar outlived the render it was measuring");
    // A cancellation is not a failure: the run ended because it was asked
    // to, and the line belongs to whoever asked.
    assert!(
        !status.lock().contains("failed"),
        "a cancelled render reported failure: {}",
        status.lock()
    );
    // And nothing is left holding the take, so it can be rendered again.
    let again = control.claim(&take);
    assert!(!control.superseded(&take, again), "a cancelled take cannot be rendered again");

    let _ = std::fs::remove_dir_all(&dir);
}

/// A render of ANOTHER take queues behind the one in flight rather than
/// killing it.
///
/// Auto-render fires for every finished take (see
/// [`a_finished_take_always_renders`]) and each take is a new file, so
/// recording twice in a row is two requests naming two different videos.
/// Cancelling on that would drop the first take's video on the floor
/// silently — a superseded run cleans up its partial and returns without
/// touching the status line, so there is nothing on screen to say the
/// video is never coming, and `last_take` has already moved on so
/// "Re-render take" cannot reach it either.
#[cfg(unix)]
#[test]
fn a_render_of_another_take_waits_rather_than_replacing_this_one() {
    use std::os::unix::fs::PermissionsExt;

    let dir = std::env::temp_dir().join(format!("harmonigraph-queue-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let fake = dir.join("slow-renderer");
    std::fs::write(&fake, "#!/bin/sh\nexec sleep 300\n").expect("write fake renderer");
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).expect("chmod");

    let control = Arc::new(RenderControl::default());
    let status = Arc::new(Mutex::new(String::new()));
    let progress = Arc::new(Progress::default());
    let start = |take: std::path::PathBuf| {
        spawn_render(
            RenderRequest {
                program: fake.clone(),
                audio: None,
                align: None,
                appearance: None,
                size: [16, 16],
                playhead: None,
            },
            take,
            status.clone(),
            progress.clone(),
            control.clone(),
        )
    };

    start(dir.join("take-1.take"));
    wait_until("the first take's render to start", || control.child.lock().is_some());
    let first = control.child.lock().as_ref().map(|f| f.child.id()).expect("a first child");

    // A second take finishes while the first is still rendering.
    start(dir.join("take-2.take"));
    // Long enough that a cancellation would have landed: the existing
    // same-take test sees the replacement inside this window.
    std::thread::sleep(std::time::Duration::from_millis(300));
    assert_eq!(
        control.child.lock().as_ref().map(|f| f.child.id()),
        Some(first),
        "take-1's render was killed by take-2's, so take-1.mp4 is never produced"
    );
    assert!(
        !status.lock().contains("failed"),
        "the surviving render reported failure: {}",
        status.lock()
    );

    control.cancel_in_flight(&dir.join("take-1.take"));
    wait_until("the queued render to take over", || {
        control.child.lock().as_ref().is_some_and(|f| f.child.id() != first)
    });
    control.cancel_in_flight(&dir.join("take-2.take"));
    wait_until("the last render to be reaped", || control.child.lock().is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

/// Verbatim shapes from `harmonigraph-offline`'s stderr — the opening line
/// that announces the frame count, and the counter it rewrites as it goes.
#[test]
fn the_renderers_own_output_is_what_puts_numbers_on_the_bar() {
    let header = "/music/Harmonigraph Takes/take-1.take: 16.5s of events \
                      -> 990 frames at 60 fps, 1920x1080 @ 1.50x -> take-1.mp4";
    assert!(matches!(parse_report(header), Some(Report::Total(990))));
    assert!(matches!(
        parse_report("  120/990 frames (12%)"),
        Some(Report::Frames { done: 120, total: 990 })
    ));

    // Everything else is a diagnostic, and belongs in the status line
    // instead of quietly moving the bar. A path with a slash in it is the
    // one that could be mistaken for a done/total pair.
    assert!(parse_report("harmonigraph-offline: no take file given").is_none());
    assert!(parse_report("note: no scratch recording, assuming take zero").is_none());
    assert!(parse_report("/music/odd frames/take.take: could not be read").is_none());
    assert!(parse_report("").is_none());
}

/// A whole run of a real render's stderr, captured verbatim from
/// `harmonigraph-offline` (the paths shortened, nothing else): the opening
/// line, the counter rewritten in place four times, and the closing line.
///
/// The `\r`s are the point. They mean the counter is one terminal line
/// being overwritten, so splitting on newlines alone delivers the whole run
/// of it as a single line, once, at the end — a progress bar that fills
/// only when the render is already over.
const REAL_RENDER_STDERR: &str = "probe.take: 1.6s of events -> 108 frames at 30 fps, \
         320x180 @ 1.00x -> probe.rgba\n\
         \r  30/108 frames (28%)\r  60/108 frames (56%)\r  90/108 frames (83%)\
         \r  108/108 frames (100%)\n\
         done: 108 frames -> probe.rgba\n";

#[test]
fn the_rewritten_counter_reaches_the_bar_as_the_render_goes() {
    let progress = Progress::default();
    progress.begin();
    follow(REAL_RENDER_STDERR.as_bytes(), &progress);
    // 108, not 30: the closing `done: 108 frames` line names a total and
    // says nothing about frames written, so it must not reset the count
    // the render finished on either.
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
                      \r  30/300 frames (10%)\r  60/300 frames (20%)\n\
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
             \r  60/120 frames (50%)\r  120/120 frames (100%)\n\
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
        rendered_status(std::path::Path::new("/takes/take-1.mp4"), tail.warning.as_deref());
    assert!(status.contains("rendered /takes/take-1.mp4"), "{status}");
    assert!(status.contains("note history 6..=8 is missing"), "{status}");
    // A clean render says nothing extra, or every export would read as one
    // that went wrong.
    assert_eq!(
        rendered_status(std::path::Path::new("/takes/take-1.mp4"), None),
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
    follow("x.take: -> 300 frames at 60 fps\n\r  240/300 frames (80%)".as_bytes(), &progress);
    assert_eq!(progress.read(), Some(harmonigraph_take::RenderProgress { done: 240, total: 300 }));
}

/// The bar survives a render ending while another is in flight.
///
/// `RenderControl` serialises renders, so nothing should reach two — but
/// the count is what makes that a belt rather than the only strap, and a
/// flag cleared by whichever finished first would blank the bar out from
/// under a running render. This holds the counting to that.
#[test]
fn the_bar_lasts_until_the_last_render_ends() {
    let progress = Progress::default();
    progress.begin();
    progress.begin();
    progress.end();
    assert!(progress.read().is_some(), "one render is still in flight");
    progress.end();
    assert_eq!(progress.read(), None);
}

#[test]
fn the_default_renderer_path_is_where_update_plugin_installs_it() {
    let path = default_renderer_path();
    assert!(path.ends_with("Harmonigraph/harmonigraph-offline"), "{path:?}");
}

/// The Video pane's bar reads the counter the render thread writes.
#[test]
fn the_panes_bar_reads_the_render_threads_progress() {
    let (_rec, ctrl) = channel();
    assert_eq!(ctrl.render_progress(), None, "no bar while nothing is rendering");
    ctrl.progress.begin();
    ctrl.progress.done.store(7, Ordering::Relaxed);
    ctrl.progress.total.store(9, Ordering::Relaxed);
    assert_eq!(
        ctrl.render_progress(),
        Some(harmonigraph_take::RenderProgress { done: 7, total: 9 })
    );
    ctrl.progress.end();
    assert_eq!(ctrl.render_progress(), None);
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
            Ok(&b"\r  240/300 frames (80%)\nharmonigraph-offline: ffmpeg died\n"[..]),
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
