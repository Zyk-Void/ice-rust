//! Mirrors `packages/agent/src/harness/tools/file-mutation-queue.ts` — serializes
//! file-mutating operations per canonical path so concurrent edits to the same
//! path (or a symlink + its target) don't interleave.
//!
//! The TS uses a `WeakMap<ExecutionEnv, MutationQueueState>` keyed by env
//! instance. Rust has no per-instance map keyed by trait object, so each env
//! impl carries an `Arc<MutationQueueRegistry>` field and exposes it via the
//! [`MutatingEnv`] accessor trait. The mutation-dependent tools (write, edit)
//! hold `Arc<dyn MutatingEnv>`; read and bash don't need it.
//!
//! **Critical invariant (plan §5.4): abort does NOT unblock the queue.** An
//! aborted caller still waits for its turn, then runs `fn` (which may observe
//! the abort and return `Err(Aborted)`), then releases. The lock guard is held
//! across `fn().await`; a `CancellationToken` only causes `fn` to error out —
//! it does NOT preempt mutex acquisition.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use crate::env::{ExecutionEnv, FileKind};
use crate::error::{FileError, FileErrorCode};

/// Per-env registry of canonical-path → mutex. Stored as a field on each env
/// impl (behind `Arc` so clones share the same map).
///
/// Admission is ordered: a fair `admission` lock is held while a caller
/// resolves its key and joins that key's queue, so a slow canonicalization
/// cannot let a later call overtake an earlier one. Different files still
/// run concurrently because only admission, not execution, is serialized.
#[derive(Default)]
pub struct MutationQueueRegistry {
    admission: Mutex<()>,
    queues: std::sync::Mutex<HashMap<String, Arc<Mutex<()>>>>,
}

impl MutationQueueRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Get (or lazily create) the mutex for `key`. The key is a canonical path
    /// string (from [`get_mutation_queue_key`]).
    fn mutex_for(&self, key: &str) -> Arc<Mutex<()>> {
        let mut map = self.queues.lock().unwrap_or_else(|e| e.into_inner());
        map.entry(key.to_string())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    }

    /// Drop `key`'s entry once nobody holds or waits on it. Callers clone the
    /// Arc under the same map lock, so a live owner always keeps it.
    fn release(&self, key: &str) {
        let mut map = self.queues.lock().unwrap_or_else(|e| e.into_inner());
        if map.get(key).is_some_and(|m| Arc::strong_count(m) == 1) {
            map.remove(key);
        }
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.queues.lock().unwrap().len()
    }
}

struct HeldQueueGuard<'a> {
    guard: Option<tokio::sync::OwnedMutexGuard<()>>,
    registry: &'a MutationQueueRegistry,
    key: String,
}

impl Drop for HeldQueueGuard<'_> {
    fn drop(&mut self) {
        // Release the mutex before checking the registry's Arc count. This
        // also runs during unwinding if a mutation closure panics.
        self.guard.take();
        self.registry.release(&self.key);
    }
}

/// Accessor used by [`with_file_mutation_queue`] to reach the env's registry
/// AND its `ExecutionEnv` methods. Concrete env impls (`OsExecutionEnv`,
/// `InMemoryExecutionEnv`) implement this; the write/edit tools hold
/// `Arc<dyn MutatingEnv>`.
///
/// The `as_env` indirection avoids needing trait-upcasting (`dyn MutatingEnv`
/// → `dyn ExecutionEnv`, unstable before Rust 1.86), so we expose the env
/// explicitly.
pub trait MutatingEnv: Send + Sync {
    /// The underlying `ExecutionEnv` (for `FileSystem`/`Shell` calls).
    fn as_env(&self) -> &dyn ExecutionEnv;

    /// The registry holding per-canonical-path mutexes.
    fn mutation_registry(&self) -> &MutationQueueRegistry;
}

/// Maximum symlink hops followed for a dangling link.
const MAX_LINK_HOPS: usize = 40;

/// Compute the canonical-path queue key for `path`. Mirrors the coding-agent
/// `getMutationQueueKey`:
/// 1. `absolutePath(path)` → propagate err.
/// 2. `canonicalPath(absolute)` → the canonical string when it exists.
/// 3. A missing path that is a dangling symlink resolves through its target.
/// 4. Otherwise canonicalize the nearest existing ancestor and append the
///    missing components, so `link/new.txt` and `target/new.txt` share a key.
/// 5. `not_supported` anywhere falls back to the absolute path; other errors
///    propagate.
pub async fn get_mutation_queue_key(
    env: &dyn ExecutionEnv,
    path: &str,
    cancel: Option<&CancellationToken>,
) -> Result<String, FileError> {
    let absolute = env.absolute_path(path, cancel).await?;
    let key = canonical_key(env, absolute.clone(), cancel, 0).await?;
    Ok(key.unwrap_or(absolute).to_string_lossy().into_owned())
}

/// `Ok(None)` means "use the absolute path" (canonicalization unsupported).
fn canonical_key<'a>(
    env: &'a dyn ExecutionEnv,
    absolute: PathBuf,
    cancel: Option<&'a CancellationToken>,
    hops: usize,
) -> futures::future::BoxFuture<'a, Result<Option<PathBuf>, FileError>> {
    Box::pin(async move {
        let abs_str = absolute.to_string_lossy().into_owned();
        match env.canonical_path(&abs_str, cancel).await {
            Ok(canonical) => return Ok(Some(canonical)),
            Err(e) if is_missing(&e) => {}
            Err(e) if e.code == FileErrorCode::NotSupported => return Ok(None),
            Err(e) => return Err(e),
        }
        // Dangling symlink: key by its (resolved) target.
        if hops < MAX_LINK_HOPS {
            if let Ok(info) = env.file_info(&abs_str, cancel).await {
                if info.kind == FileKind::Symlink {
                    if let Ok(target) = env.read_link(&abs_str, cancel).await {
                        let target = match absolute.parent() {
                            Some(parent) if target.is_relative() => parent.join(target),
                            _ => target,
                        };
                        // Keep `..` until canonicalization has resolved any
                        // symlink components before it. Lexically collapsing
                        // `alias/..` is not equivalent when `alias` is itself
                        // a symlink.
                        return canonical_key(env, target, cancel, hops + 1).await;
                    }
                }
            }
        }
        // Walk up to the nearest existing ancestor.
        let mut missing: Vec<std::ffi::OsString> = Vec::new();
        let mut current = absolute.clone();
        loop {
            match (current.parent(), current.file_name()) {
                (Some(parent), Some(name)) => {
                    missing.push(name.to_os_string());
                    current = parent.to_path_buf();
                }
                _ => return Ok(None),
            }
            let cur_str = current.to_string_lossy().into_owned();
            match env.canonical_path(&cur_str, cancel).await {
                Ok(mut canonical) => {
                    for name in missing.iter().rev() {
                        canonical.push(name);
                    }
                    return Ok(Some(canonical));
                }
                Err(e) if is_missing(&e) => continue,
                Err(e) if e.code == FileErrorCode::NotSupported => return Ok(None),
                Err(e) => return Err(e),
            }
        }
    })
}

fn is_missing(e: &FileError) -> bool {
    matches!(
        e.code,
        FileErrorCode::NotFound | FileErrorCode::NotDirectory
    )
}

/// Serialize `fn` per canonical-path key for `env`. The closure `f` runs while
/// holding the per-key `tokio::Mutex` guard; abort/cancel during the wait does
/// NOT skip the queue — `f` itself must observe the token and return
/// `Err(FileError::aborted())`.
///
/// Mirrors `withFileMutationQueue`. Calls are admitted in invocation order:
/// key resolution happens under the registry's fair admission lock (without
/// the cancel token, as in the TS harness), and the caller joins the key's
/// FIFO queue before admission is released. The `&CancellationToken` is
/// passed to `f` so it can perform the same pre/post-execution abort checks
/// the tools do (the write tool checks before AND after the write).
pub async fn with_file_mutation_queue<F, R>(
    env: &dyn MutatingEnv,
    path: &str,
    cancel: &CancellationToken,
    f: F,
) -> Result<R, FileError>
where
    F: for<'a> FnOnce(
        &'a CancellationToken,
    ) -> futures::future::BoxFuture<'a, Result<R, FileError>>,
    R: Send + 'static,
{
    let registry = env.mutation_registry();
    let (key, mut acquire, ready) = {
        let _admission = registry.admission.lock().await;
        let key = get_mutation_queue_key(env.as_env(), path, None).await?;
        let mutex = registry.mutex_for(&key);
        let mut acquire = Box::pin(mutex.lock_owned());
        // Poll once so this caller is queued (tokio's mutex is FIFO) before
        // the next caller is admitted.
        let ready = futures::poll!(&mut acquire);
        (key, acquire, ready)
    };
    let guard = match ready {
        std::task::Poll::Ready(guard) => guard,
        std::task::Poll::Pending => acquire.as_mut().await,
    };
    drop(acquire);
    let _guard = HeldQueueGuard {
        guard: Some(guard),
        registry: &registry,
        key,
    };
    // Now that we hold the lock, run `f`. It may observe cancellation and
    // return Err(Aborted); the guard drops on return, releasing the next
    // waiter and removing the idle registry entry. It also cleans up during
    // unwinding if `f` panics.
    f(cancel).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn registry_creates_and_reuses_mutex() {
        let reg = MutationQueueRegistry::new();
        let m1 = reg.mutex_for("/a/b");
        let m2 = reg.mutex_for("/a/b");
        assert!(Arc::ptr_eq(&m1, &m2));
        let m3 = reg.mutex_for("/c");
        assert!(!Arc::ptr_eq(&m1, &m3));
    }

    #[tokio::test]
    async fn idle_entries_are_removed_after_use() {
        let env = crate::InMemoryExecutionEnv::with_cwd("/w".into());
        let token = CancellationToken::new();
        with_file_mutation_queue(&env, "a.txt", &token, |_| Box::pin(async { Ok(()) }))
            .await
            .unwrap();
        assert_eq!(env.mutation_registry().len(), 0);
    }

    #[tokio::test]
    async fn panicking_mutation_releases_its_idle_registry_entry() {
        use futures::FutureExt;

        let env = crate::InMemoryExecutionEnv::with_cwd("/w".into());
        let token = CancellationToken::new();
        let result = std::panic::AssertUnwindSafe(with_file_mutation_queue::<_, ()>(
            &env,
            "a.txt",
            &token,
            |_| Box::pin(async { panic!("simulated mutation panic") }),
        ))
        .catch_unwind()
        .await;

        assert!(result.is_err());
        assert_eq!(env.mutation_registry().len(), 0);
    }
}
