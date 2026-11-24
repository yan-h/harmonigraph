//! The plugin's host parameters, as one fixed table of rows.
//!
//! A row is an integer or a float. The host sees every row as a value from 0
//! to its step count (0 to 1 for a float), the *host value*; the plugin reads
//! and writes the row's *plain* value, its integer or its float. The conversions repeat nice-plug's
//! `IntRange` and `FloatRange::Linear` arithmetic in `f32`, step for step, so
//! a project saves and automates as it did before the table replaced them
//! (#1814).
//!
//! A row holds its unmodulated plain value and a mono modulation offset, in
//! normalized units. Every read of the plain value the plugin acts on is
//! modulated; a save and the host's `get_value` read the unmodulated value,
//! and an editor write is taken as the value shown, modulation included.
//!
//! The editor writes a row at once, as the host's automation does, and the
//! row remembers what the host has not yet been told of it: a gesture begun
//! or ended, and the value the editor set. The shell tells the host at its
//! next process callback or flush, at offset 0 (#1814 Q1). The value told is
//! the editor's even where host automation or a flush has since written the
//! row: the host records what the editor supplied, and its own input takes
//! effect in callback order (Yan, 2026-11-18).

use std::collections::BTreeMap;

use atomic_float::AtomicF32;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};

use crate::plugin::ParamValue;
use crate::wrapper::hash_param_id;

/// An integer row's text, both ways.
pub type IntToText = Box<dyn Fn(i32) -> String + Send + Sync>;
pub type TextToInt = Box<dyn Fn(&str) -> Option<i32> + Send + Sync>;

/// What a row holds.
pub enum Kind {
    /// An integer from `min` to `max`, with optional text both ways. Without
    /// them it reads and parses as the integer.
    Int { min: i32, max: i32, to_text: Option<IntToText>, from_text: Option<TextToInt> },
    /// A float from `min` to `max`, linear, read and parsed as the number.
    Float { min: f32, max: f32 },
}

/// One row as the plugin declares it.
pub struct Spec {
    /// The stable id a save stores the value under, and the source of the
    /// CLAP id the host knows the row by.
    pub id: &'static str,
    pub name: &'static str,
    pub kind: Kind,
    /// The plain default: an integer as a float, or the float.
    pub default: f32,
    pub automatable: bool,
    /// Whether the host may modulate it. Only an automatable row is.
    pub modulatable: bool,
}

impl Spec {
    /// An automatable, modulatable integer.
    pub fn int(id: &'static str, name: &'static str, default: i32, min: i32, max: i32) -> Spec {
        assert!(min < max && (min..=max).contains(&default), "{id}: a range around its default");
        let kind = Kind::Int { min, max, to_text: None, from_text: None };
        Spec { id, name, kind, default: default as f32, automatable: true, modulatable: true }
    }

    /// An automatable, modulatable float.
    pub fn float(id: &'static str, name: &'static str, default: f32, min: f32, max: f32) -> Spec {
        assert!(min < max && (min..=max).contains(&default), "{id}: a range around its default");
        let kind = Kind::Float { min, max };
        Spec { id, name, kind, default, automatable: true, modulatable: true }
    }

    /// Neither automated nor modulated by the host.
    pub fn non_automatable(self) -> Spec {
        Spec { automatable: false, modulatable: false, ..self }
    }

    /// Automated, but not modulated, by the host.
    pub fn non_modulatable(self) -> Spec {
        Spec { modulatable: false, ..self }
    }

    /// An integer row's text from its value.
    pub fn with_to_text(mut self, f: impl Fn(i32) -> String + Send + Sync + 'static) -> Spec {
        match &mut self.kind {
            Kind::Int { to_text, .. } => *to_text = Some(Box::new(f)),
            _ => panic!("{}: only an integer row takes its own text", self.id),
        }
        self
    }

    /// An integer row's value from text, already trimmed.
    pub fn with_from_text(
        mut self,
        f: impl Fn(&str) -> Option<i32> + Send + Sync + 'static,
    ) -> Spec {
        match &mut self.kind {
            Kind::Int { from_text, .. } => *from_text = Some(Box::new(f)),
            _ => panic!("{}: only an integer row parses its own text", self.id),
        }
        self
    }

    /// The host steps: `None` for a float, which the host sees as 0 to 1.
    pub fn steps(&self) -> Option<usize> {
        match &self.kind {
            Kind::Int { min, max, .. } => Some((max - min) as usize),
            Kind::Float { .. } => None,
        }
    }

    /// The step count as the host scales by it, 1 for a float.
    fn scale(&self) -> usize {
        self.steps().unwrap_or(1)
    }

    /// Plain to normalized, clamped: `IntRange::normalize` and
    /// `FloatRange::Linear`'s. An integer's offset from the minimum is taken
    /// in `i64`, so typed text far past the range clamps to its near end.
    pub fn normalize(&self, plain: f32) -> f32 {
        let (min, max) = match &self.kind {
            Kind::Float { min, max } => return (plain.clamp(*min, *max) - min) / (max - min),
            Kind::Int { min, max, .. } => (*min, *max),
        };
        ((plain as i64 - i64::from(min)) as f32 / (max - min) as f32).clamp(0.0, 1.0)
    }

    /// Normalized to plain, clamped and, for an integer, rounded:
    /// `IntRange::unnormalize` and `FloatRange::Linear`'s.
    pub fn unnormalize(&self, normalized: f32) -> f32 {
        let normalized = normalized.clamp(0.0, 1.0);
        let (min, max) = match &self.kind {
            Kind::Float { min, max } => return (normalized * (max - min)) + min,
            Kind::Int { min, max, .. } => (*min, *max),
        };
        ((normalized * (max - min) as f32).round() as i32 + min) as f32
    }

    /// A plain value as the host reads it. An integer's is its exact step from
    /// the minimum: CLAP reads a stepped value by truncating it (`params.h`),
    /// so a step computed through the normalized value, a hair below the
    /// integer, would read as the step below.
    pub fn to_host(&self, plain: f32) -> f64 {
        match &self.kind {
            Kind::Int { min, max, .. } => {
                ((plain as i64).clamp(i64::from(*min), i64::from(*max)) - i64::from(*min)) as f64
            }
            Kind::Float { .. } => self.normalize(plain) as f64,
        }
    }

    /// A host value as normalized: divided in `f32`, as nice-plug did.
    pub(crate) fn host_to_normalized(&self, host: f64) -> f32 {
        host as f32 / self.scale() as f32
    }

    /// The plain value a host value sets: normalized, then rounded for an
    /// integer, all in `f32`.
    fn plain_of_host(&self, host: f64) -> f32 {
        self.unnormalize(self.host_to_normalized(host))
    }

    /// The text for a host value.
    pub fn text(&self, host: f64) -> String {
        let plain = self.plain_of_host(host);
        match &self.kind {
            Kind::Int { to_text: Some(f), .. } => f(plain as i32),
            Kind::Int { to_text: None, .. } => (plain as i32).to_string(),
            Kind::Float { .. } => plain.to_string(),
        }
    }

    /// The host value for text, if it parses.
    pub fn parse(&self, text: &str) -> Option<f64> {
        let text = text.trim();
        let plain = match &self.kind {
            Kind::Int { from_text: Some(f), .. } => f(text)? as f32,
            Kind::Int { from_text: None, .. } => text.parse::<i32>().ok()? as f32,
            // A NaN would be saved as `null`, which no load reads back.
            Kind::Float { .. } => text.parse::<f32>().ok().filter(|value| !value.is_nan())?,
        };
        Some(self.to_host(plain))
    }

    /// What a save stores for a plain value.
    fn save(&self, plain: f32) -> ParamValue {
        match &self.kind {
            Kind::Int { .. } => ParamValue::I32(plain as i32),
            Kind::Float { .. } => ParamValue::F32(plain),
        }
    }

    /// The plain value a restore takes from `saved`: the nearest bound for
    /// one out of range, and `None` for one of the wrong type, which a
    /// restore sets to the default.
    fn read(&self, saved: &ParamValue) -> Option<f32> {
        match (&self.kind, saved) {
            (Kind::Int { min, max, .. }, ParamValue::I32(value)) => {
                Some((*value).clamp(*min, *max) as f32)
            }
            (Kind::Float { min, max }, ParamValue::F32(value)) => Some(value.clamp(*min, *max)),
            _ => None,
        }
    }
}

/// A row: its declaration, its values, and what the host has not been told.
struct Row {
    spec: Spec,
    hash: u32,
    /// The unmodulated plain value.
    plain: AtomicF32,
    /// The host's mono modulation, normalized.
    offset: AtomicF32,
    /// The editor's side of the host's report, [`OPEN`], [`BEGUN`] and
    /// [`DIRTY`]: the editor sets them, and the shell's report claims
    /// `BEGUN` and `DIRTY` in one step.
    report: AtomicU8,
    /// The host value the editor last set, as `f64` bits: what [`DIRTY`]
    /// says the host has not been told.
    pending: AtomicU64,
    /// The host has been told a gesture began and not that it ended. Only the
    /// shell's report touches it, from process or flush, which the host never
    /// runs at once.
    told_open: AtomicBool,
}

/// The editor holds a gesture open on the row.
const OPEN: u8 = 1;
/// The editor began a gesture the host has not been told of.
const BEGUN: u8 = 2;
/// The editor set a value the host has not been told of, in `pending`.
const DIRTY: u8 = 4;

/// One thing the shell tells the host about a row.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Report {
    Begin,
    /// The value the editor set, as a host value.
    Value(f64),
    End,
}

impl Row {
    fn modulated(&self) -> f32 {
        let plain = self.plain.load(Ordering::Relaxed);
        let offset = self.offset.load(Ordering::Relaxed);
        if offset == 0.0 {
            plain
        } else {
            self.spec.unnormalize(self.spec.normalize(plain) + offset)
        }
    }
}

/// The host parameters: a fixed list of rows, in the order the host lists
/// them. Each value is an atomic, so any thread reads and writes it without
/// a lock.
pub struct Table {
    rows: Box<[Row]>,
}

impl Table {
    /// The rows in host order. Ids, and the CLAP ids hashed from them, must be
    /// unique.
    pub fn new(specs: impl IntoIterator<Item = Spec>) -> Table {
        let rows: Box<[Row]> = specs
            .into_iter()
            .map(|spec| Row {
                hash: hash_param_id(spec.id),
                plain: AtomicF32::new(spec.default),
                offset: AtomicF32::new(0.0),
                report: AtomicU8::new(0),
                pending: AtomicU64::new(0),
                told_open: AtomicBool::new(false),
                spec,
            })
            .collect();
        for (index, row) in rows.iter().enumerate() {
            assert!(
                rows[..index].iter().all(|other| other.hash != row.hash),
                "{}: a CLAP id another row has",
                row.spec.id
            );
        }
        Table { rows }
    }

    /// A table with no rows.
    pub fn empty() -> Table {
        Table::new([])
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// An integer row's value, modulated.
    pub fn int(&self, row: usize) -> i32 {
        self.rows[row].modulated() as i32
    }

    /// A float row's value, modulated.
    pub fn float(&self, row: usize) -> f32 {
        self.rows[row].modulated()
    }

    pub fn spec(&self, row: usize) -> &Spec {
        &self.rows[row].spec
    }

    /// The CLAP id of `row`.
    pub(crate) fn hash(&self, row: usize) -> u32 {
        self.rows[row].hash
    }

    /// The row the host knows by `hash`.
    pub(crate) fn row(&self, hash: u32) -> Option<usize> {
        self.rows.iter().position(|row| row.hash == hash)
    }

    /// What the host reads through `params.get_value`: the unmodulated value,
    /// scaled by the step count. CLAP hears a parameter as its value plus its
    /// modulation (`events.h`), so the modulation is the host's to add, not
    /// part of the value (Yan, 2026-11-23, as in otonal).
    pub(crate) fn base_host_value(&self, row: usize) -> f64 {
        let row = &self.rows[row];
        row.spec.to_host(row.plain.load(Ordering::Relaxed))
    }

    /// The row the host knows by `hash`, and the plain value a host value
    /// sets it to: the table's own decoding, which the plugin's reading of an
    /// automation event shares, so the timeline and the table agree on every
    /// value (#1814). `None` for an id no row has, or a value that is not
    /// finite, which the shell does not apply.
    pub fn decode(&self, hash: u32, host: f64) -> Option<(usize, f32)> {
        let row = self.row(hash)?;
        host.is_finite().then(|| (row, self.rows[row].spec.plain_of_host(host)))
    }

    /// Take a value the host or the editor set, as a host value: the table's
    /// one write, so a value the editor sets rounds as the host's does.
    pub fn set_host(&self, row: usize, host: f64) {
        let row = &self.rows[row];
        row.plain.store(row.spec.plain_of_host(host), Ordering::Relaxed);
    }

    /// The editor begins a gesture on `row`.
    pub(crate) fn begin_edit(&self, row: usize) {
        self.rows[row].report.fetch_or(OPEN | BEGUN, Ordering::AcqRel);
    }

    /// The editor sets `row` to `shown`, at once, and owes the host the
    /// value that gives. The editor reads a row modulated, so it writes one as
    /// it shows it: the value set is `shown` less the modulation's present
    /// shift, and the modulation rides on top of it.
    pub(crate) fn edit(&self, row: usize, shown: f32) {
        let r = &self.rows[row];
        let plain = shown - (r.modulated() - r.plain.load(Ordering::Relaxed));
        let host = r.spec.to_host(plain);
        self.set_host(row, host);
        self.rows[row].pending.store(host.to_bits(), Ordering::Release);
        // After the value, so a report that claims the bit reads it.
        self.rows[row].report.fetch_or(DIRTY, Ordering::AcqRel);
    }

    /// The editor ends its gesture on `row`.
    pub(crate) fn end_edit(&self, row: usize) {
        self.rows[row].report.fetch_and(!OPEN, Ordering::AcqRel);
    }

    /// Tell the host, through `push`, what the editor did since the last
    /// report: per row a gesture's begin, the value the editor last set, and
    /// the gesture's end. Each row's edit is claimed in one step, so an edit
    /// racing the report lands whole in this report or the next. Edits
    /// between two reports share one gesture, and the host is never told of
    /// an end without its begin. Stops at the first refusal, whose part is
    /// told again at the next report; false then.
    pub(crate) fn report(&self, mut push: impl FnMut(u32, Report) -> bool) -> bool {
        for row in self.rows.iter() {
            let claimed = row.report.fetch_and(!(BEGUN | DIRTY), Ordering::AcqRel);
            let mut told_open = row.told_open.load(Ordering::Relaxed);
            if claimed & (BEGUN | DIRTY) != 0 && !told_open {
                if !push(row.hash, Report::Begin) {
                    row.report.fetch_or(claimed & (BEGUN | DIRTY), Ordering::AcqRel);
                    return false;
                }
                told_open = true;
                row.told_open.store(true, Ordering::Relaxed);
            }
            if claimed & DIRTY != 0 {
                let value = f64::from_bits(row.pending.load(Ordering::Acquire));
                if !push(row.hash, Report::Value(value)) {
                    row.report.fetch_or(DIRTY, Ordering::AcqRel);
                    return false;
                }
            }
            // A gesture still open at the claim is ended by a later report.
            if told_open && claimed & OPEN == 0 {
                if !push(row.hash, Report::End) {
                    return false;
                }
                row.told_open.store(false, Ordering::Relaxed);
            }
        }
        true
    }

    /// Apply a host parameter event to the row it names: a value, or a mono
    /// modulation. An id no row has and a value that is not finite are
    /// skipped, as [`Table::decode`] skips them.
    pub(crate) fn apply(&self, id: u32, value: f64, modulation: bool) {
        let Some(row) = self.row(id).filter(|_| value.is_finite()) else { return };
        if modulation {
            self.modulate_host(row, value);
        } else {
            self.set_host(row, value);
        }
    }

    /// Take the host's mono modulation of `row`, by a host amount. False for a
    /// row the host may not modulate, which keeps its offset.
    pub(crate) fn modulate_host(&self, row: usize, amount: f64) -> bool {
        let row = &self.rows[row];
        if !row.spec.modulatable {
            return false;
        }
        row.offset.store(row.spec.host_to_normalized(amount), Ordering::Relaxed);
        true
    }

    /// Every row's unmodulated value, keyed by id, as a save stores them.
    pub(crate) fn save(&self) -> BTreeMap<String, ParamValue> {
        self.rows
            .iter()
            .map(|row| (row.spec.id.to_owned(), row.spec.save(row.plain.load(Ordering::Relaxed))))
            .collect()
    }

    /// Set every row from a save, by [`Spec::read`]'s rules or to its default,
    /// and write one log line naming the ids the save holds that no row has
    /// and one naming those [`Table::defaulted`] sets to the default. Leaves the
    /// modulation as it is. A value the editor set and the host has not been
    /// told is dropped: the restore supersedes it, and the host reads the
    /// restored state by rescanning (Yan, 2026-11-18).
    pub(crate) fn restore(&self, saved: &BTreeMap<String, ParamValue>) {
        for row in &self.rows {
            // A finished click goes whole; a gesture still open keeps its
            // begin, for the end the editor will send.
            let mut bits = row.report.load(Ordering::Acquire);
            loop {
                let kept = if bits & OPEN == 0 { bits & !(BEGUN | DIRTY) } else { bits & !DIRTY };
                match row.report.compare_exchange_weak(
                    bits,
                    kept,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                ) {
                    Ok(_) => break,
                    Err(now) => bits = now,
                }
            }
            let plain = saved.get(row.spec.id).and_then(|value| row.spec.read(value));
            row.plain.store(plain.unwrap_or(row.spec.default), Ordering::Relaxed);
        }
        let unknown = self.unknown(saved);
        if !unknown.is_empty() {
            crate::shell_warn!("The saved state names parameters this build has not: {unknown:?}");
        }
        let defaulted = self.defaulted(saved);
        if !defaulted.is_empty() {
            crate::shell_warn!(
                "The saved state holds values these parameters cannot read, which took their \
                 defaults: {defaulted:?}"
            );
        }
    }

    /// The ids `saved` holds for a row whose value is not of that row's type,
    /// one this build cannot decode among them, which a restore sets to the
    /// default.
    pub fn defaulted<'a>(&self, saved: &'a BTreeMap<String, ParamValue>) -> Vec<&'a str> {
        self.rows
            .iter()
            .filter_map(|row| saved.get_key_value(row.spec.id).map(|entry| (row, entry)))
            .filter(|(row, (_, value))| row.spec.read(value).is_none())
            .map(|(_, (id, _))| id.as_str())
            .collect()
    }

    /// The ids `saved` holds that no row has, which a restore skips.
    pub fn unknown<'a>(&self, saved: &'a BTreeMap<String, ParamValue>) -> Vec<&'a str> {
        saved
            .keys()
            .map(String::as_str)
            .filter(|id| self.rows.iter().all(|row| row.spec.id != *id))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A restore sets every row by one rule, whatever the save holds for it:
    /// the default for a value that is missing or of the wrong type, and the
    /// nearest bound for one out of range.
    #[test]
    fn a_restore_takes_the_default_or_the_nearest_bound() {
        let table = Table::new([
            Spec::int("int", "Int", 1, -2, 2),
            Spec::float("float", "Float", 0.5, 0.0, 1.0),
        ]);
        let restore = |saved: &[(&str, ParamValue)]| {
            for row in 0..table.len() {
                table.set_host(row, 0.0);
            }
            table.restore(&saved.iter().map(|(id, v)| (id.to_string(), v.clone())).collect());
            (table.int(0), table.float(1))
        };
        assert_eq!(restore(&[]), (1, 0.5), "missing");
        assert_eq!(
            restore(&[("int", ParamValue::F32(1.0)), ("float", ParamValue::I32(1))]),
            (1, 0.5),
            "of the wrong type"
        );
        // A wrong-typed value is named, one that does not decode among them,
        // and one under an id no row has is named only as unknown.
        let unread = || ParamValue::Unread(Default::default());
        let wrong = [("int", ParamValue::F32(1.0)), ("float", unread()), ("retired", unread())];
        let wrong = wrong.map(|(id, value)| (id.to_owned(), value)).into();
        assert_eq!(table.defaulted(&wrong), ["int", "float"]);
        assert_eq!(table.unknown(&wrong), ["retired"]);
        assert_eq!(
            restore(&[("int", ParamValue::I32(9)), ("float", ParamValue::F32(3.0)),]),
            (2, 1.0),
            "out of range"
        );
        assert_eq!(
            restore(&[
                ("int", ParamValue::I32(-1)),
                ("float", ParamValue::F32(0.25)),
                ("retired", ParamValue::I32(3)),
            ]),
            (-1, 0.25),
            "in range, beside an id no row has"
        );
    }

    /// Typed text far past an integer row's range clamps to the near end, and
    /// a float row refuses `nan`, which a save could not hold.
    #[test]
    fn typed_text_out_of_range_clamps_and_nan_is_refused() {
        let int = Spec::int("int", "Int", 0, -9, 9);
        assert_eq!(int.parse("2147483600"), Some(18.0));
        assert_eq!(int.parse("-2147483600"), Some(0.0));
        let float = Spec::float("float", "Float", 0.0, -1.0, 1.0);
        assert_eq!(float.parse("nan"), None);
        assert_eq!(float.parse("inf"), Some(1.0));
    }

    /// A value the editor sets while the host is being told the last one, from
    /// inside the push, is told by the next report, inside the gesture the
    /// host already holds open. The claim of a row's bits in one step, which
    /// closes the window before the first push, is the next test's subject.
    #[test]
    fn an_edit_made_during_a_push_is_told_by_the_next_report() {
        let table = Table::new([Spec::float("float", "Float", 0.0, 0.0, 1.0)]);
        let report = |during: &dyn Fn(Report)| {
            let mut told = Vec::new();
            table.report(|_, report| {
                told.push(report);
                during(report);
                true
            });
            told
        };
        table.begin_edit(0);
        table.edit(0, 0.25);
        let told = report(&|report| {
            if report == Report::Begin {
                table.edit(0, 0.375);
            } else if report == Report::Value(0.375) {
                table.edit(0, 0.5);
            }
        });
        assert_eq!(told, [Report::Begin, Report::Value(0.375)], "the gesture stays open");
        assert_eq!(report(&|_| {}), [Report::Value(0.5)]);
        table.end_edit(0);
        assert_eq!(report(&|_| {}), [Report::End]);
        assert!(report(&|_| {}).is_empty());
    }

    /// Under real threads, one editing as the editor does (clicks and drags)
    /// and one reporting as the shell does with some pushes refused, the host
    /// is told a well-formed gesture stream, and once the editor is done it
    /// holds the editor's last value. A claim split into a load and a clear
    /// fails within the first rounds (#1924).
    #[test]
    fn a_report_racing_editor_edits_tells_a_well_formed_stream() {
        use std::sync::Arc;
        const ROUNDS: u32 = 2000;
        let table = Arc::new(Table::new([Spec::float("float", "Float", 0.0, 0.0, 1.0)]));
        let mut seed = 0x9e3779b97f4a7c15u64;
        let mut refuse = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed.is_multiple_of(5)
        };
        for round in 0..ROUNDS {
            let done = Arc::new(AtomicBool::new(false));
            let editor = {
                let (table, done) = (table.clone(), done.clone());
                std::thread::spawn(move || {
                    let mut last = 0.0f64;
                    for gesture in 0..8u32 {
                        let drag = gesture % 2 == 0;
                        if drag {
                            table.begin_edit(0);
                        }
                        for step in 0..(1 + gesture % 3) {
                            let plain = ((round * 31 + gesture * 7 + step) % 1000) as f32 / 1000.0;
                            table.edit(0, plain);
                            last = table.base_host_value(0);
                            if step % 2 == 0 {
                                std::thread::yield_now();
                            }
                        }
                        if drag {
                            table.end_edit(0);
                        }
                    }
                    done.store(true, Ordering::Release);
                    last
                })
            };
            let mut open = false;
            let mut told = None;
            let mut report = |refuse: bool| {
                table.report(|_, report| {
                    if refuse {
                        return false;
                    }
                    match report {
                        Report::Begin => {
                            assert!(!open, "round {round}: Begin inside an open gesture");
                            open = true;
                        }
                        Report::Value(value) => {
                            assert!(open, "round {round}: a value outside a gesture");
                            told = Some(value);
                        }
                        Report::End => {
                            assert!(open, "round {round}: End with no gesture open");
                            open = false;
                        }
                    }
                    true
                })
            };
            while !done.load(Ordering::Acquire) {
                report(refuse());
            }
            let last = editor.join().unwrap();
            report(false);
            report(false);
            assert!(report(false), "round {round}: nothing left to refuse");
            assert!(!open, "round {round}: the host was left holding a gesture open");
            assert_eq!(told, Some(last), "round {round}: the host holds a stale value");
        }
    }

    /// A value the host sets while its modulation holds the row at a bound is
    /// still the one saved, though the modulated value does not move.
    #[test]
    fn a_value_set_under_modulation_held_at_a_bound_is_saved() {
        let table = Table::new([Spec::float("float", "Float", 0.5, 0.0, 1.0)]);
        assert!(table.modulate_host(0, 1.0));
        table.set_host(0, 0.2);
        assert_eq!(table.float(0), 1.0, "held at the bound");
        table.set_host(0, 0.3);
        assert_eq!(table.float(0), 1.0);
        assert!(matches!(table.save()["float"], ParamValue::F32(value) if value == 0.3));
    }

    /// The editor writes a modulated row as it shows it: a drag that moves
    /// the shown value by a step each frame moves it by that step, and the
    /// value saved moves with it, the modulation riding on top.
    #[test]
    fn an_editor_drag_under_modulation_follows_the_pointer() {
        let table = Table::new([Spec::float("float", "Float", 0.5, 0.0, 1.0)]);
        assert!(table.modulate_host(0, 0.25));
        for frame in 1..=3 {
            let shown = table.float(0) - 0.05;
            table.edit(0, shown);
            assert!((table.float(0) - shown).abs() < 1e-6, "frame {frame}: {}", table.float(0));
        }
        let ParamValue::F32(saved) = table.save()["float"] else { panic!() };
        assert!((saved - 0.35).abs() < 1e-6, "saved {saved}");
    }
}
