//! Single-owner bounded stdout writer for the native RPC transport.
//!
//! Mirrors the raw-output half of ice's `core/output-guard.ts`: writes stay
//! ordered on one sink, an optional UTF-8 byte budget is reserved
//! synchronously before a write is queued, transient sink pressure is
//! retried every 10 ms, and `flush` is an ordered barrier.
//!
//! Differences from ice, on purpose:
//! - the queue is also bounded by message count (64), so producers wait for
//!   the sink instead of buffering without limit;
//! - a retry resends only the bytes the sink has not accepted yet, so a
//!   partial write is never duplicated;
//! - ice redirects incidental `process.stdout.write` calls to stderr. Rust
//!   has no such hook: host code must use this writer (or stderr) while it
//!   owns stdout, and a native plugin writing to fd 1 directly is not caught.
use std::io::{ErrorKind, Write};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Pause before retrying a write the sink refused for now. Mirrors ice's
/// 10 ms `ENOBUFS`/`EAGAIN` retry.
const TRANSIENT_RETRY: Duration = Duration::from_millis(10);

/// Why a write was refused before it was queued. Mirrors
/// `RawStdoutLimitError` and its `code`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RawOutputLimitError {
    /// The write alone is larger than the budget.
    TooLarge { bytes: usize, limit: usize },
    /// The write does not fit beside the bytes already queued.
    Overflow {
        bytes: usize,
        limit: usize,
        queued: usize,
    },
}

impl RawOutputLimitError {
    pub(crate) fn code(&self) -> &'static str {
        match self {
            RawOutputLimitError::TooLarge { .. } => "output_too_large",
            RawOutputLimitError::Overflow { .. } => "output_overflow",
        }
    }
}

impl std::fmt::Display for RawOutputLimitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RawOutputLimitError::TooLarge { bytes, limit } => write!(
                f,
                "stdout write of {bytes} bytes exceeds the {limit}-byte limit"
            ),
            RawOutputLimitError::Overflow {
                bytes,
                limit,
                queued,
            } => write!(
                f,
                "stdout write of {bytes} bytes overflows the {limit}-byte limit \
                 ({queued} bytes already queued)"
            ),
        }
    }
}

impl std::error::Error for RawOutputLimitError {}

#[derive(Default)]
struct Budget {
    limit: Mutex<Option<usize>>,
    queued: AtomicUsize,
}

/// Bytes held against the budget until the sink is done with them. Dropping
/// releases them, whether the write succeeded, failed or was skipped because
/// the sink had already failed.
struct Reservation {
    bytes: Vec<u8>,
    budget: Arc<Budget>,
}

impl Drop for Reservation {
    fn drop(&mut self) {
        self.budget
            .queued
            .fetch_sub(self.bytes.len(), Ordering::SeqCst);
    }
}

enum Message {
    Write(Reservation),
    Flush(tokio::sync::oneshot::Sender<()>),
}

#[derive(Clone)]
pub(crate) struct RpcWriter {
    tx: tokio::sync::mpsc::Sender<Message>,
    budget: Arc<Budget>,
}

impl RpcWriter {
    /// Set the byte budget for queued plus in-flight writes; `None` removes
    /// it. Bytes already reserved stay reserved. Mirrors
    /// `configureRawStdoutLimit`.
    #[allow(dead_code)] // host policy hook; RPC runs unlimited today, like ice
    pub(crate) fn configure_limit(&self, limit: Option<usize>) {
        *self.budget.limit.lock().unwrap_or_else(|e| e.into_inner()) = limit;
    }

    /// Bytes reserved by queued and in-flight writes. Mirrors
    /// `getRawStdoutQueuedBytes`.
    #[allow(dead_code)]
    pub(crate) fn queued_bytes(&self) -> usize {
        self.budget.queued.load(Ordering::SeqCst)
    }

    /// Reserve `bytes` against the budget, then queue them, waiting while
    /// the queue is full. An empty write is ignored; a refused write reserves
    /// nothing. Returns `Ok` without writing once the sink has failed (the
    /// writer task reports that failure). Mirrors `writeRawStdout`.
    pub(crate) async fn write(&self, bytes: Vec<u8>) -> Result<(), RawOutputLimitError> {
        if bytes.is_empty() {
            return Ok(());
        }
        let reservation = self.reserve(bytes)?;
        let _ = self.tx.send(Message::Write(reservation)).await;
        Ok(())
    }

    fn reserve(&self, bytes: Vec<u8>) -> Result<Reservation, RawOutputLimitError> {
        let limit = *self.budget.limit.lock().unwrap_or_else(|e| e.into_inner());
        let len = bytes.len();
        if let Some(limit) = limit {
            if len > limit {
                return Err(RawOutputLimitError::TooLarge { bytes: len, limit });
            }
            // Reserve atomically so concurrent producers cannot both fit.
            let reserved =
                self.budget
                    .queued
                    .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |queued| {
                        (queued + len <= limit).then_some(queued + len)
                    });
            if let Err(queued) = reserved {
                return Err(RawOutputLimitError::Overflow {
                    bytes: len,
                    limit,
                    queued,
                });
            }
        } else {
            self.budget.queued.fetch_add(len, Ordering::SeqCst);
        }
        Ok(Reservation {
            bytes,
            budget: self.budget.clone(),
        })
    }

    /// Wait until everything queued before this call reached the sink and
    /// was flushed. Returns at once when the sink has failed. Mirrors
    /// `flushRawStdout`.
    #[allow(dead_code)]
    pub(crate) async fn flush(&self) {
        let (done, wait) = tokio::sync::oneshot::channel();
        if self.tx.send(Message::Flush(done)).await.is_ok() {
            let _ = wait.await;
        }
    }
}

pub(crate) fn start_rpc_writer<W: Write + Send + 'static>(
    mut output: W,
) -> (
    RpcWriter,
    tokio::task::JoinHandle<std::io::Result<()>>,
    tokio_util::sync::CancellationToken,
) {
    let (tx, mut rx) = tokio::sync::mpsc::channel::<Message>(64);
    let failed = tokio_util::sync::CancellationToken::new();
    let failure_signal = failed.clone();
    let task = tokio::task::spawn_blocking(move || {
        let result = (|| {
            while let Some(message) = rx.blocking_recv() {
                match message {
                    Message::Write(reservation) => {
                        write_all_retrying(&mut output, &reservation.bytes)?;
                        flush_retrying(&mut output)?;
                    }
                    Message::Flush(done) => {
                        flush_retrying(&mut output)?;
                        let _ = done.send(());
                    }
                }
            }
            flush_retrying(&mut output)
        })();
        if result.is_err() {
            failure_signal.cancel();
            // Later writes are skipped; closing the queue releases their
            // reservations and wakes any producer waiting for room.
            rx.close();
            while rx.try_recv().is_ok() {}
        }
        result
    });
    (
        RpcWriter {
            tx,
            budget: Arc::default(),
        },
        task,
        failed,
    )
}

/// True for sink errors that mean "try again shortly": `WouldBlock`
/// (`EAGAIN`/`EWOULDBLOCK`) and, on Unix, `ENOBUFS`.
fn is_transient(error: &std::io::Error) -> bool {
    if error.kind() == ErrorKind::WouldBlock || error.kind() == ErrorKind::Interrupted {
        return true;
    }
    #[cfg(unix)]
    if error.raw_os_error() == Some(nix::errno::Errno::ENOBUFS as i32) {
        return true;
    }
    false
}

/// Write every byte, retrying transient refusals indefinitely and resuming
/// after the bytes the sink already accepted.
fn write_all_retrying<W: Write>(output: &mut W, mut bytes: &[u8]) -> std::io::Result<()> {
    while !bytes.is_empty() {
        match output.write(bytes) {
            Ok(0) => return Err(ErrorKind::WriteZero.into()),
            Ok(n) => bytes = &bytes[n..],
            Err(error) if is_transient(&error) => std::thread::sleep(TRANSIENT_RETRY),
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

fn flush_retrying<W: Write>(output: &mut W) -> std::io::Result<()> {
    loop {
        match output.flush() {
            Err(error) if is_transient(&error) => std::thread::sleep(TRANSIENT_RETRY),
            result => return result,
        }
    }
}

/// Queue one JSON line. With no budget configured (the RPC default) this
/// never fails; a refused line is reported on stderr and dropped.
pub(crate) async fn write_line(out: &RpcWriter, value: &serde_json::Value) {
    let mut line = value.to_string().into_bytes();
    line.push(b'\n');
    if let Err(error) = out.write(line).await {
        eprintln!("[icers] rpc output refused ({}): {error}", error.code());
    }
}

/// Write `text` to a print-mode sink, retrying transient pressure, and
/// report a permanent failure instead of discarding it.
pub(crate) fn write_print_output<W: Write>(output: &mut W, text: &str) -> std::io::Result<()> {
    write_all_retrying(output, text.as_bytes())?;
    if !text.ends_with('\n') {
        write_all_retrying(output, b"\n")?;
    }
    flush_retrying(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Default)]
    struct Sink {
        data: Arc<Mutex<Vec<u8>>>,
        /// Refusals to return before accepting, per write call.
        would_block: Arc<AtomicUsize>,
        /// Accept at most this many bytes per write call (0 = unlimited).
        chunk: usize,
        fail: bool,
    }

    impl Write for Sink {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            if self.fail {
                return Err(ErrorKind::BrokenPipe.into());
            }
            if self
                .would_block
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
                .is_ok()
            {
                return Err(ErrorKind::WouldBlock.into());
            }
            let n = if self.chunk == 0 {
                buf.len()
            } else {
                buf.len().min(self.chunk)
            };
            self.data.lock().unwrap().extend_from_slice(&buf[..n]);
            Ok(n)
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn budget_counts_utf8_bytes_and_rejects_without_reserving() {
        let (writer, task, _) = start_rpc_writer(Sink::default());
        // "é" is two bytes: three of them are exactly six.
        writer.configure_limit(Some(6));
        let error = writer.write("ééé!".as_bytes().to_vec()).await.unwrap_err();
        assert_eq!(error.code(), "output_too_large");
        assert_eq!(writer.queued_bytes(), 0, "a refused write reserves nothing");
        // Overflow: hold a reservation directly, as an in-flight write would.
        let held = writer.reserve("éé".as_bytes().to_vec()).unwrap();
        let error = writer.write("éé".as_bytes().to_vec()).await.unwrap_err();
        assert_eq!(
            error,
            RawOutputLimitError::Overflow {
                bytes: 4,
                limit: 6,
                queued: 4
            }
        );
        assert_eq!(error.code(), "output_overflow");
        // Reservations survive reconfiguration and are released on drop.
        writer.configure_limit(None);
        writer.configure_limit(Some(6));
        assert_eq!(writer.queued_bytes(), 4);
        drop(held);
        writer.write("ééé".as_bytes().to_vec()).await.unwrap();
        writer.write(Vec::new()).await.unwrap();
        writer.flush().await;
        assert_eq!(writer.queued_bytes(), 0);
        drop(writer);
        task.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn transient_refusals_retry_without_duplicating_partial_writes() {
        let sink = Sink {
            would_block: Arc::new(AtomicUsize::new(3)),
            chunk: 2,
            ..Default::default()
        };
        let data = sink.data.clone();
        let refusals = sink.would_block.clone();
        let (writer, task, _) = start_rpc_writer(sink);
        writer.write(b"hello ".to_vec()).await.unwrap();
        refusals.store(2, Ordering::SeqCst);
        writer.write(b"world\n".to_vec()).await.unwrap();
        writer.flush().await;
        assert_eq!(&*data.lock().unwrap(), b"hello world\n");
        drop(writer);
        task.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn a_permanent_failure_poisons_the_queue_and_releases_reservations() {
        let sink = Sink {
            fail: true,
            ..Default::default()
        };
        let (writer, task, failed) = start_rpc_writer(sink);
        writer.write(b"a\n".to_vec()).await.unwrap();
        failed.cancelled().await;
        // Later writes are skipped and the flush barrier does not hang.
        writer.write(b"b\n".to_vec()).await.unwrap();
        writer.flush().await;
        assert_eq!(writer.queued_bytes(), 0);
        drop(writer);
        assert_eq!(
            task.await.unwrap().unwrap_err().kind(),
            ErrorKind::BrokenPipe
        );
    }

    #[test]
    fn print_output_reports_sink_failures_and_adds_a_newline() {
        let mut ok = Sink::default();
        write_print_output(&mut ok, "done").unwrap();
        assert_eq!(&*ok.data.lock().unwrap(), b"done\n");
        let mut broken = Sink {
            fail: true,
            ..Default::default()
        };
        assert!(write_print_output(&mut broken, "done").is_err());
    }
}
