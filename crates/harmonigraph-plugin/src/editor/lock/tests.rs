//! The saved-state protocol, enumerated (#1757): every sequence of four host
//! and editor events, run over the real [`EditorLock`], [`Inbox`],
//! [`UiState`](crate::editor::UiState) and `SharedState`, against an oracle
//! written from the requirements rather than from this mechanism — the spec
//! of the protocol model the design was checked with, transcribed.
//!
//! An event stands for the real code it names, and is that code wherever a
//! test can run it:
//! - a restore and a save are `UiState::set` and `map`, as the host calls them;
//! - a save that cannot take the lock, and one re-entered from a callback the
//!   editor makes under it, are `UiState::save` with no wait while the lock is
//!   held — by a holder that took it before any restore still waiting;
//! - an open is [`open_window_session`], a close [`EditorLock::end_window_session`];
//! - a frame is a hold of the lock and the mark `root_ui` sets past the
//!   loading screen ([`harmonigraph_ui::testing::mark_drawn`], whose real
//!   site `shell.rs`'s frame tests run), an editing frame adds a look edit;
//! - a worker round, and a frame the loading screen takes, are a hold of the
//!   lock and nothing else.
//!
//! Four events reach every counterexample the model found, the shortest
//! being `[Open, Frame, CloseReentrantSave]`.

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::AtomicU32;
use std::sync::Arc;
use std::time::Duration;

use harmonigraph_ui::testing::{edit_look, mark_drawn, undo_depth};
use harmonigraph_ui::{SharedState, SpectralOrientation};
use nice_plug::params::persist::PersistentField;

use super::{open_window_session, EditorLock};
use crate::editor::{EditorShared, UiState, ASSUMED_SURFACE_FORMAT};

const DEPTH: usize = 4;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    /// Installs whole.
    Clean,
    /// Installs with a key cut: here, an orientation this build lacks.
    Lossy,
    /// Refused whole: here, a version below the floor.
    Refused,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Op {
    Restore(Kind),
    RestoreEmpty,
    Save,
    /// A save that cannot take the lock: another holder has it.
    SaveTimeout,
    Open,
    Close,
    Frame,
    EditFrame,
    Worker,
    /// A close whose gesture callback re-enters a save.
    CloseReentrantSave,
    /// A close whose gesture callback re-enters a restore.
    CloseReentrantRestore,
    /// A frame the loading screen takes.
    FrameSkipped,
    /// An editing frame whose parameter gesture re-enters a save (#1301's limit).
    FrameReentrantSave,
}

const OPS: [Op; 15] = [
    Op::Restore(Kind::Clean),
    Op::Restore(Kind::Lossy),
    Op::Restore(Kind::Refused),
    Op::RestoreEmpty,
    Op::Save,
    Op::SaveTimeout,
    Op::Open,
    Op::Close,
    Op::Frame,
    Op::EditFrame,
    Op::Worker,
    Op::CloseReentrantSave,
    Op::CloseReentrantRestore,
    Op::FrameSkipped,
    Op::FrameReentrantSave,
];

/// The settings, as far as the oracle tells them apart.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
struct Doc {
    /// 0 = fresh; otherwise the restore it was installed from.
    base: u8,
    /// Installed with the restore's lossy key at its default.
    partial: bool,
    edits: u8,
}

const FRESH: Doc = Doc { base: 0, partial: false, edits: 0 };

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Text {
    /// A host restore's own document, verbatim.
    Raw(u8),
    /// The live settings, serialized.
    Live(Doc),
    /// The empty document.
    Nothing,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Delivery {
    Blob(u8, Kind),
    Empty,
}

/// What the owner should observe. A lock-taking event first installs the
/// newest restore not yet installed; a load that lost something is saved
/// verbatim until a frame has drawn after it; a save that cannot take the
/// lock returns the newest of the last restore, close, successful save and
/// first frame to draw the editor since it was fresh;
/// the undo history survives everything but a restore that installs.
#[derive(Clone)]
struct Spec {
    live: Doc,
    history: u8,
    pending: Option<Delivery>,
    hold: Option<u8>,
    /// False until a frame draws or a non-empty restore is installed: an
    /// editor nobody has seen and no project has filled saves nothing.
    touched: bool,
    fallback: Text,
    open: bool,
    /// Each restore's kind, by id; id 0 is the fresh state.
    kinds: Vec<Kind>,
}

impl Spec {
    fn new() -> Spec {
        Spec {
            live: FRESH,
            history: 0,
            pending: None,
            hold: None,
            touched: false,
            fallback: Text::Nothing,
            open: false,
            kinds: vec![Kind::Clean],
        }
    }

    fn prelude(&mut self) {
        match self.pending.take() {
            Some(Delivery::Blob(id, Kind::Clean)) => {
                self.live = Doc { base: id, partial: false, edits: 0 };
                self.history = 0;
                self.hold = None;
                self.touched = true;
            }
            Some(Delivery::Blob(id, Kind::Lossy)) => {
                self.live = Doc { base: id, partial: true, edits: 0 };
                self.history = 0;
                self.hold = Some(id);
                self.touched = true;
            }
            Some(Delivery::Blob(id, Kind::Refused)) => {
                self.hold = Some(id);
                self.touched = true;
            }
            // "" is what an untouched editor saves, so it restores to that:
            // the fresh look, its histories gone, and untouched again.
            Some(Delivery::Empty) => {
                self.live = FRESH;
                self.history = 0;
                self.hold = None;
                self.touched = false;
            }
            None => {}
        }
    }

    fn answer(&self) -> Text {
        match (self.hold, self.touched) {
            (Some(id), _) => Text::Raw(id),
            (None, true) => Text::Live(self.live),
            (None, false) => Text::Nothing,
        }
    }

    fn allows(&self, op: Op) -> bool {
        match op {
            Op::Open => !self.open,
            Op::Close
            | Op::Frame
            | Op::EditFrame
            | Op::CloseReentrantSave
            | Op::FrameSkipped
            | Op::FrameReentrantSave => self.open,
            Op::CloseReentrantRestore => self.open && self.kinds.len() < 4,
            Op::Restore(_) => self.kinds.len() < 4,
            _ => true,
        }
    }

    /// A frame draws the editor: a hold ends, and the save answers live.
    /// The first such frame since the state was fresh also leaves the live
    /// settings as the fallback, so a save that cannot take the lock never
    /// answers `""` once a save that can would not.
    fn drawn(&mut self, edit: bool) {
        let first = !self.touched;
        self.hold = None;
        self.touched = true;
        if edit {
            self.live.edits += 1;
            self.history += 1;
        }
        if first {
            self.fallback = self.answer();
        }
    }

    /// A new restore of `kind`, and its id.
    fn restore(&mut self, kind: Kind) -> u8 {
        self.kinds.push(kind);
        let id = (self.kinds.len() - 1) as u8;
        self.pending = Some(Delivery::Blob(id, kind));
        self.fallback = Text::Raw(id);
        id
    }
}

/// The documents a sequence restores, and how a saved one reads back.
struct Fixtures {
    /// The fresh document's, which the fresh state reads back as.
    fresh: Marks,
    /// What each restore sets, by id: its own camera yaw, an attack that
    /// edits move from, and an orientation its lossy twin names a variant
    /// this build lacks for.
    restored: [Marks; 4],
    clean: Vec<String>,
    lossy: Vec<String>,
    refused: Vec<String>,
}

/// The settings a fixture tells documents apart by.
#[derive(Clone, Copy)]
struct Marks {
    yaw: f32,
    attack: f32,
    orientation: SpectralOrientation,
}

impl Marks {
    fn of(state: &SharedState) -> Marks {
        let appearance = &state.picture.appearance;
        Marks {
            yaw: appearance.camera.yaw,
            attack: appearance.spectrum.attack,
            orientation: appearance.spectrum.orientation,
        }
    }
}

impl Fixtures {
    fn new() -> Fixtures {
        let fresh = Marks::of(&SharedState::new(ASSUMED_SURFACE_FORMAT));
        use SpectralOrientation::*;
        // Set off the fresh document's own values, whatever they are, so a
        // restore always reads apart from it and its cut key always shows.
        let orientation = [Left, Right, Top, Bottom].into_iter().find(|&o| o != fresh.orientation);
        let restored = std::array::from_fn(|id| Marks {
            yaw: fresh.yaw + 0.25 * id as f32,
            attack: 0.3,
            orientation: orientation.expect("an orientation besides the fresh one"),
        });
        let clean: Vec<String> = restored
            .iter()
            .map(|marks: &Marks| {
                let mut state = SharedState::new(ASSUMED_SURFACE_FORMAT);
                let appearance = &mut state.picture.appearance;
                appearance.camera.yaw = marks.yaw;
                appearance.spectrum.attack = marks.attack;
                appearance.spectrum.orientation = marks.orientation;
                state.save_persist()
            })
            .collect();
        let lossy = clean
            .iter()
            .zip(&restored)
            .map(|(blob, marks)| {
                let named = format!("orientation:{:?},", marks.orientation);
                let lossy = blob.replacen(&named, "orientation:Diagonal,", 1);
                assert_ne!(&lossy, blob, "{named} is not in the restore");
                lossy
            })
            .collect();
        let refused = clean
            .iter()
            .map(|blob| {
                let digits = &blob["(version:".len()..];
                let version: u32 = digits[..digits.find(',').unwrap()].parse().unwrap();
                blob.replacen(
                    &format!("(version:{version},"),
                    &format!("(version:{},", version - 1),
                    1,
                )
            })
            .collect();
        Fixtures { fresh, restored, clean, lossy, refused }
    }

    fn blob(&self, id: u8, kind: Kind) -> &str {
        match kind {
            Kind::Clean => &self.clean[id as usize],
            Kind::Lossy => &self.lossy[id as usize],
            Kind::Refused => &self.refused[id as usize],
        }
    }

    fn doc_of(&self, state: &SharedState) -> Doc {
        let marks = Marks::of(state);
        let base = (1..4u8).find(|&id| self.restored[id as usize].yaw == marks.yaw).unwrap_or(0);
        let from = if base == 0 { self.fresh } else { self.restored[base as usize] };
        let partial = base > 0 && marks.orientation != from.orientation;
        let edits = ((marks.attack - from.attack) / 0.01).round() as u8;
        Doc { base, partial, edits }
    }

    /// The settings a save stored, if it stored any this build reads whole.
    fn doc_saved(&self, saved: &str, cache: &mut HashMap<String, Option<Doc>>) -> Option<Doc> {
        if let Some(doc) = cache.get(saved) {
            return *doc;
        }
        let mut state = SharedState::new(ASSUMED_SURFACE_FORMAT);
        let doc = (state.install_document(saved) && !saved.is_empty()).then(|| self.doc_of(&state));
        cache.insert(saved.to_string(), doc);
        doc
    }
}

/// The real objects, as a plugin owns them.
struct Real {
    ui_state: UiState,
    lock: Arc<EditorLock>,
    ctx: egui::Context,
}

impl Real {
    fn new(ctx: &egui::Context) -> Real {
        let (_, notes) = harmonigraph_record::publication::channel();
        let (_, audio) = crate::audio_ingress::channel(64);
        let (_, take) = harmonigraph_record::channel();
        let shared = EditorShared::new(notes, audio, Arc::new(AtomicU32::new(0)), take);
        let ui_state = UiState::new();
        let lock = Arc::new(EditorLock::new(shared, ui_state.inbox()));
        ui_state.attach(&lock);
        Real { ui_state, lock, ctx: ctx.clone() }
    }

    fn save(&self) -> String {
        self.ui_state.map(String::clone)
    }

    /// A save that finds the lock held: no wait, the answer it settles for.
    fn save_refused_the_lock(&self) -> String {
        self.ui_state.save(Duration::ZERO).to_string()
    }

    /// A frame that draws the editor, through the inbox as the plugin's are.
    fn frame(&self, edit: bool) {
        let mut shared = self.lock.lock();
        self.lock.inbox().frame(&mut shared.ui, |ui| {
            mark_drawn(ui);
            if edit {
                edit_look(ui, false);
            }
        });
    }
}

/// The shortest counterexample per check.
type Failures = BTreeMap<&'static str, (Vec<Op>, String)>;

/// A saved document, short enough for a report.
fn brief(saved: &str) -> String {
    match saved.char_indices().nth(40) {
        Some((end, _)) => format!("{:?}… ({} bytes)", &saved[..end], saved.len()),
        None => format!("{saved:?}"),
    }
}

struct Run<'a> {
    fixtures: &'a Fixtures,
    saved: HashMap<String, Option<Doc>>,
    failures: Failures,
}

impl Run<'_> {
    fn note(&mut self, check: &'static str, trace: &[Op], detail: String) {
        let entry = self.failures.entry(check).or_insert_with(|| (trace.to_vec(), detail.clone()));
        if trace.len() < entry.0.len() {
            *entry = (trace.to_vec(), detail);
        }
    }

    /// Whether `got` stores what `want` names. A clean restore's own
    /// document and the state installed from it, unedited, are the same
    /// settings, so either answers for the other.
    fn stores(&mut self, got: &str, want: Text, spec: &Spec) -> bool {
        let want = match want {
            Text::Raw(id) if spec.kinds[id as usize] == Kind::Clean => {
                if got == self.fixtures.blob(id, Kind::Clean) {
                    return true;
                }
                Text::Live(Doc { base: id, partial: false, edits: 0 })
            }
            want => want,
        };
        match want {
            Text::Nothing => got.is_empty(),
            Text::Raw(id) => got == self.fixtures.blob(id, spec.kinds[id as usize]),
            Text::Live(doc) => self.fixtures.doc_saved(got, &mut self.saved) == Some(doc),
        }
    }

    fn check_live(&mut self, spec: &Spec, real: &Real, trace: &[Op], at: &str) {
        let shared = real.lock.undelivered();
        let (live, history) = (self.fixtures.doc_of(&shared.ui), undo_depth(&shared.ui));
        drop(shared);
        if live != spec.live {
            self.note(
                "the state holds the newest restore and every edit",
                trace,
                format!("at {at}: live {live:?}, want {:?}", spec.live),
            );
        }
        if history != usize::from(spec.history) {
            self.note(
                "undo history survives everything but a restore that installs",
                trace,
                format!("at {at}: history {history}, want {}", spec.history),
            );
        }
    }

    fn step(&mut self, spec: &mut Spec, real: &Real, op: Op, trace: &[Op]) {
        let fixtures = self.fixtures;
        match op {
            Op::Restore(kind) => {
                let id = spec.restore(kind);
                real.ui_state.set(fixtures.blob(id, kind).to_string());
            }
            Op::RestoreEmpty => {
                spec.pending = Some(Delivery::Empty);
                spec.fallback = Text::Nothing;
                real.ui_state.set(String::new());
            }
            Op::Save => {
                spec.prelude();
                let want = spec.answer();
                spec.fallback = want;
                let got = real.save();
                if !self.stores(&got, want, spec) {
                    self.note(
                        "a save stores the answer",
                        trace,
                        format!("got {}, want {want:?}", brief(&got)),
                    );
                }
            }
            Op::SaveTimeout => {
                let want = spec.fallback;
                let held = real.lock.undelivered();
                let got = real.save_refused_the_lock();
                drop(held);
                if !self.stores(&got, want, spec) {
                    self.note(
                        "a save that cannot take the lock is no older than the last restore, \
                         close or save",
                        trace,
                        format!("got {}, want {want:?}", brief(&got)),
                    );
                }
            }
            Op::Open => {
                spec.prelude();
                spec.open = true;
                open_window_session(&real.lock, &real.ctx);
                self.check_live(spec, real, trace, "open");
            }
            Op::Close => {
                spec.prelude();
                spec.fallback = spec.answer();
                spec.open = false;
                real.lock.end_window_session(|_| {});
                self.check_live(spec, real, trace, "close");
            }
            Op::Frame | Op::EditFrame => {
                spec.prelude();
                spec.drawn(op == Op::EditFrame);
                real.frame(op == Op::EditFrame);
                self.check_live(spec, real, trace, "frame");
            }
            Op::Worker | Op::FrameSkipped => {
                spec.prelude();
                drop(real.lock.lock());
                self.check_live(spec, real, trace, "a hold that draws nothing");
            }
            Op::CloseReentrantSave => {
                spec.prelude();
                spec.fallback = spec.answer();
                let want = spec.fallback;
                spec.open = false;
                let mut got = String::new();
                real.lock.end_window_session(|_| got = real.save_refused_the_lock());
                if !self.stores(&got, want, spec) {
                    self.note(
                        "a save re-entered from the close's callback holds the session",
                        trace,
                        format!("got {}, want {want:?}", brief(&got)),
                    );
                }
                self.check_live(spec, real, trace, "close");
            }
            Op::CloseReentrantRestore => {
                spec.prelude();
                spec.fallback = spec.answer();
                spec.open = false;
                let id = spec.restore(Kind::Clean);
                real.lock.end_window_session(|_| {
                    real.ui_state.set(fixtures.blob(id, Kind::Clean).to_string())
                });
                self.check_live(spec, real, trace, "close");
            }
            Op::FrameReentrantSave => {
                // #1301's stated limit: the save returns the fallback as it
                // stood, and the frame then draws and edits.
                spec.prelude();
                let want = spec.fallback;
                spec.drawn(true);
                let mut shared = real.lock.lock();
                let got = real.lock.inbox().frame(&mut shared.ui, |ui| {
                    mark_drawn(ui);
                    let got = real.save_refused_the_lock();
                    edit_look(ui, false);
                    got
                });
                drop(shared);
                if !self.stores(&got, want, spec) {
                    self.note(
                        "a save re-entered from a frame's callback returns the fallback as it stood",
                        trace,
                        format!("got {}, want {want:?}", brief(&got)),
                    );
                }
                self.check_live(spec, real, trace, "frame with a re-entered save");
            }
        }
    }
}

/// Every sequence of [`DEPTH`] events the spec allows.
fn sequences(spec: &Spec, trace: &mut Vec<Op>, out: &mut Vec<Vec<Op>>) {
    if trace.len() == DEPTH {
        out.push(trace.clone());
        return;
    }
    for op in OPS.into_iter().filter(|&op| spec.allows(op)) {
        // Only what `allows` reads: whether a window is open, and how many
        // restores have been made.
        let mut next = spec.clone();
        match op {
            Op::Open => next.open = true,
            Op::Close | Op::CloseReentrantSave => next.open = false,
            Op::CloseReentrantRestore => {
                next.open = false;
                next.kinds.push(Kind::Clean);
            }
            Op::Restore(kind) => next.kinds.push(kind),
            _ => {}
        }
        trace.push(op);
        sequences(&next, trace, out);
        trace.pop();
    }
}

#[test]
fn every_sequence_of_host_and_editor_events_keeps_the_protocol() {
    let fixtures = Fixtures::new();
    let ctx = egui::Context::default();
    let mut all = Vec::new();
    sequences(&Spec::new(), &mut Vec::new(), &mut all);
    let mut run = Run { fixtures: &fixtures, saved: HashMap::new(), failures: Failures::new() };
    for trace in &all {
        let (mut spec, real) = (Spec::new(), Real::new(&ctx));
        for at in 1..=trace.len() {
            run.step(&mut spec, &real, trace[at - 1], &trace[..at]);
        }
    }
    let report: Vec<String> = run
        .failures
        .iter()
        .map(|(check, (trace, detail))| format!("{check}\n    {trace:?}\n    {detail}"))
        .collect();
    assert!(report.is_empty(), "{} sequences; failed:\n{}", all.len(), report.join("\n"));
}
