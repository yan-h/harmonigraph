//! How far a video render has got, and the export queue around it.
//!
//! The export queue's vocabulary — [`RenderProgress`], [`ExportJob`],
//! [`ExportStatus`] and the archive copy's name — never enters a take and has
//! no serde. Progress is counted off the renderer subprocess's stderr and lives
//! only as long as that process. It sits here because `harmonigraph-record`
//! drives that subprocess and reports back through it, and must not link the
//! editor to do so. Moving it to the UI would put the GUI stack back on the
//! record path, which is what #176 removed.
//!
//! The Video pane's SETTINGS (`RenderConfig` and its parts) are not here: a
//! take carries them only inside the opaque `appearance` document in its header
//! (#797), so they live in `harmonigraph-ui` beside that document.

/// How far a video render running in the background has got.
///
/// Frames rather than a fraction, because frames are what the renderer counts;
/// the Video pane divides them for its percentage and keeps both counts in its
/// hover text.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RenderProgress {
    /// Frames finished so far: encoded, for a video, which trails the frames
    /// drawn by the encoder's backlog and reaches `total` only once the video
    /// is written — and its archive copy, when the Video pane's Archive copy
    /// asks for one, whose frames count after the video's.
    pub done: u64,
    /// Frames the render is aiming for — 0 until the renderer has said, which
    /// is a moment into the run (it has a take to read and an encoder to
    /// start first). Twice the video's with an archive copy, which encodes
    /// every frame again.
    pub total: u64,
}

impl RenderProgress {
    /// Reserved stderr prefix for the renderer's progress records.
    pub const PREFIX: &str = "progress: ";

    /// Read one complete record. Human diagnostics, including frame counts,
    /// are not part of this protocol. The percentage is presentation only.
    pub fn parse_record(record: &str) -> Option<Self> {
        let (counts, percent) = record.strip_prefix(Self::PREFIX)?.split_once(" frames (")?;
        let percent = percent.strip_suffix("%)")?;
        let (done, total) = counts.split_once('/')?;
        let number = |value: &str| {
            (!value.is_empty() && value.bytes().all(|b| b.is_ascii_digit()))
                .then(|| value.parse::<u64>().ok())
                .flatten()
        };
        (number(percent)? <= 100).then_some(Self { done: number(done)?, total: number(total)? })
    }

    /// The share done, in `0..=1`, or `None` while the total is unknown —
    /// which is not the same as zero, and must not draw as it.
    pub fn fraction(self) -> Option<f32> {
        (self.total > 0).then(|| (self.done as f32 / self.total as f32).clamp(0.0, 1.0))
    }
}

impl std::fmt::Display for RenderProgress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let percent = if self.total == 0 {
            0
        } else {
            (u128::from(self.done) * 100 / u128::from(self.total)).min(100)
        };
        write!(f, "{}{}/{} frames ({percent}%)", Self::PREFIX, self.done, self.total)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_records_round_trip_and_require_the_whole_record() {
        for (done, total) in [(0, 0), (0, 300), (120, 300), (u64::MAX, u64::MAX)] {
            let progress = RenderProgress { done, total };
            assert_eq!(RenderProgress::parse_record(&progress.to_string()), Some(progress));
        }
        for text in [
            "warning: only 30 frames remain",
            "30/300 frames (10%)",
            "progress: 30/300 frames (10%) trailing",
            "progress: 30/300 frames",
            "progress: +30/300 frames (10%)",
            "progress: 30/300 frames (101%)",
            "progress: 18446744073709551616/300 frames (10%)",
        ] {
            assert_eq!(RenderProgress::parse_record(text), None, "{text}");
        }
    }
}

/// Where a video's archive copy goes, when the Video pane's Archive copy
/// (`RenderConfig::archive` in `harmonigraph-ui`) asks for one: beside it, as
/// `<stem>-archive.mp4`. One function because two processes name it — the
/// offline renderer writes it, and the export queue publishes it beside the
/// video it published.
pub fn archive_path(video: &std::path::Path) -> std::path::PathBuf {
    video.with_file_name(format!(
        "{}-archive.mp4",
        video.file_stem().unwrap_or_default().to_string_lossy()
    ))
}

/// Session-only export state; never persisted in a project or take.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExportStatus {
    Pending,
    Running,
    Cancelling,
    Completed,
    Failed,
    Cancelled,
}
impl ExportStatus {
    pub fn is_finished(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }
}
#[derive(Clone, Debug)]
pub struct ExportJob {
    pub id: u64,
    pub take: std::path::PathBuf,
    pub output: std::path::PathBuf,
    /// What the Exports list shows for the job, and nothing the render reads:
    /// the renderer derives the real size from the appearance it is given.
    /// `None` for a recorded take's automatic export: its appearance rides in
    /// the take's header, and this crate never parses an appearance.
    pub size: Option<[u32; 2]>,
    pub state: ExportStatus,
    pub progress: RenderProgress,
    pub detail: String,
}
