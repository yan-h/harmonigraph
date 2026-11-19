//! A diagnostic probe for Yan's Bitwig pass (#1814 step 6, #1804). Not for merging.
//!
//! Appends lines to `~/Library/Logs/Harmonigraph/bitwig-probe-<pid>.txt`:
//! each activation's sample rate, each `mark_dirty`, each host state save and load,
//! and the transport counts the audio thread keeps in atomics, written with each save.
//! Only main-thread callbacks write the file; the audio thread only counts.

use std::io::Write;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::time::{SystemTime, UNIX_EPOCH};

/// Host transport events found in a process call's input list.
pub static TRANSPORT_IN_LIST: AtomicU64 = AtomicU64::new(0);
/// Of those, the ones at a nonzero sample offset: the mid-block split step 6 deletes.
pub static TRANSPORT_MID_BLOCK: AtomicU64 = AtomicU64::new(0);
/// The largest such offset seen.
pub static TRANSPORT_MAX_OFFSET: AtomicU64 = AtomicU64::new(0);

/// Counts one transport event from the host's list, at `offset` samples into its block.
pub fn transport(offset: u32) {
    TRANSPORT_IN_LIST.fetch_add(1, Relaxed);
    if offset > 0 {
        TRANSPORT_MID_BLOCK.fetch_add(1, Relaxed);
        TRANSPORT_MAX_OFFSET.fetch_max(u64::from(offset), Relaxed);
    }
}

/// The transport counts, for a log line.
pub fn transport_counts() -> String {
    format!(
        "transport in-list={} mid-block={} max-offset={}",
        TRANSPORT_IN_LIST.load(Relaxed),
        TRANSPORT_MID_BLOCK.load(Relaxed),
        TRANSPORT_MAX_OFFSET.load(Relaxed),
    )
}

/// Appends one timestamped line. Main thread only.
pub fn line(plugin: &str, text: &str) {
    let Some(home) = std::env::var_os("HOME") else { return };
    let dir = std::path::Path::new(&home).join("Library/Logs/Harmonigraph");
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join(format!("bitwig-probe-{}.txt", std::process::id()));
    let ms = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis());
    if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(file, "{ms} {plugin}: {text}");
    }
}
