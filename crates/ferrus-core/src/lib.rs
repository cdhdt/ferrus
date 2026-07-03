//! Ferrus core engine.
//!
//! Platform-abstracted engine for creating bootable USB media. This crate holds
//! all the logic Ferrus needs regardless of front-end (CLI or GUI) and,
//! eventually, regardless of host OS. In Phase 0 the modules are documented
//! stubs: signatures and doc-comments are in place, bodies are `todo!()` with a
//! `// TODO(phaseN):` marker.
//!
//! # Safety
//!
//! Ferrus erases block devices. The guard rails against writing to the wrong
//! device are part of the API surface (see [`device`]) rather than an
//! afterthought. Read `CLAUDE.md` at the repo root before implementing any
//! destructive path.
//!
//! # Module map
//!
//! - [`error`]     — error types.
//! - [`device`]    — device enumeration and safe target selection.
//! - [`source`]    — source image (`RawImage`) + Windows-media detection.
//! - [`partition`] — partition scheme (GPT/MBR, UEFI/Legacy).
//! - [`format`]    — filesystem creation wrappers.
//! - [`copy`]      — ISO content extraction/copy.
//! - [`boot`]      — bootloader installation (UEFI:NTFS).
//! - [`windows`]   — `autounattend.xml` + LabConfig generation (the
//!   differentiator).
//! - [`progress`]  — progress reporting.
//! - [`platform`]  — OS abstraction.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod boot;
pub mod copy;
pub mod device;
pub mod error;
pub mod format;
pub mod partition;
pub mod platform;
pub mod progress;
pub mod source;
pub mod windows;

pub use error::{Error, Result};
