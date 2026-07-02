//! Progress reporting.
//!
//! Long operations (copy, format) report progress through a [`ProgressSink`] so
//! the engine stays UI-agnostic: the CLI can render a bar, the GUI can update a
//! widget, and tests can assert on the events.

/// A stage of the overall operation, for coarse progress labeling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Stage {
    /// Inspecting the source image.
    Inspecting,
    /// Partitioning the target device.
    Partitioning,
    /// Creating filesystems.
    Formatting,
    /// Copying data.
    Copying,
    /// Installing bootloaders.
    InstallingBoot,
    /// Writing Windows tweak files.
    WritingTweaks,
    /// Flushing buffers to the device.
    Finalizing,
}

/// Receives progress updates from the engine.
///
/// Implementations must be cheap and non-blocking; the engine may call them
/// frequently from within tight copy loops.
pub trait ProgressSink {
    /// Called when the operation enters a new [`Stage`].
    fn stage(&mut self, stage: Stage);

    /// Called periodically within a stage with `done`/`total` bytes (or items).
    /// `total` is `None` when the size is unknown.
    fn advance(&mut self, done: u64, total: Option<u64>);

    /// Called with a free-form human-readable status line.
    fn message(&mut self, text: &str);
}

/// A [`ProgressSink`] that discards everything. Useful for tests and dry-runs
/// where progress is irrelevant.
#[derive(Debug, Default, Clone, Copy)]
pub struct NullProgress;

impl ProgressSink for NullProgress {
    fn stage(&mut self, _stage: Stage) {}
    fn advance(&mut self, _done: u64, _total: Option<u64>) {}
    fn message(&mut self, _text: &str) {}
}
