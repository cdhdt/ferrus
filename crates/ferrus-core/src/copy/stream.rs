//! The pure block-copy loop.
//!
//! Written against [`WriteSink`] and [`Read`] rather than a concrete device, so
//! it is fully testable with an in-memory sink (see `../tests.rs`).

use std::io::Read;

use crate::Result;
use crate::platform::WriteSink;
use crate::progress::{ProgressSink, Stage};

/// Block size for the copy loop. 4 MiB balances syscall overhead against memory
/// and gives smooth progress on typical USB write speeds.
pub(crate) const BLOCK_SIZE: usize = 4 * 1024 * 1024;

/// Copy every byte of `reader` into `sink`, reporting cumulative progress
/// against `total`. Returns the number of bytes written.
///
/// Does **not** sync — the caller is responsible for the final
/// [`WriteSink::sync`] before reporting success (SPEC-0002 invariant 7).
///
/// # Errors
///
/// Returns an error on the first read or write failure.
pub(crate) fn copy_stream(
    reader: &mut dyn Read,
    sink: &mut dyn WriteSink,
    total: u64,
    progress: &mut dyn ProgressSink,
) -> Result<u64> {
    let mut buf = vec![0u8; BLOCK_SIZE];
    let mut written: u64 = 0;

    progress.stage(Stage::Copying);
    progress.advance(0, Some(total));

    loop {
        let n = fill(reader, &mut buf)?;
        if n == 0 {
            break;
        }
        sink.write_chunk(&buf[..n])?;
        written += n as u64;
        progress.advance(written, Some(total));
    }

    Ok(written)
}

/// Read until `buf` is full or EOF, coping with short reads. Returns the number
/// of bytes read (0 only at EOF).
fn fill(reader: &mut dyn Read, buf: &mut [u8]) -> Result<usize> {
    let mut filled = 0;
    while filled < buf.len() {
        match reader.read(&mut buf[filled..]) {
            Ok(0) => break,
            Ok(n) => filled += n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e.into()),
        }
    }
    Ok(filled)
}
