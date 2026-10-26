//! Mirrors `packages/coding-agent/test/file-mutation-queue.test.ts`: mutations
//! to one file run in invocation order even when the first caller's key
//! lookup is slower, and symlink aliases of missing targets share one queue.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
#[cfg(unix)]
use ice_tools::get_mutation_queue_key;
use ice_tools::{
    with_file_mutation_queue, ExecutionEnv, FileContent, FileError, FileInfo, FileSystem,
    InMemoryExecutionEnv, MutatingEnv, MutationQueueRegistry, Shell, ShellExecOptions, ShellOutput,
};
use tokio_util::sync::CancellationToken;

/// Delegates to an in-memory env, but the first `canonical_path` call is
/// slow, so an unordered queue would let the second caller win the race.
struct SlowFirstLookupEnv {
    inner: Arc<InMemoryExecutionEnv>,
    first_lookup_pending: AtomicBool,
    registry: MutationQueueRegistry,
}

impl MutatingEnv for SlowFirstLookupEnv {
    fn as_env(&self) -> &dyn ExecutionEnv {
        self
    }
    fn mutation_registry(&self) -> &MutationQueueRegistry {
        &self.registry
    }
}

#[async_trait]
impl FileSystem for SlowFirstLookupEnv {
    fn cwd(&self) -> &Path {
        self.inner.cwd()
    }
    async fn absolute_path(
        &self,
        path: &str,
        cancel: Option<&CancellationToken>,
    ) -> Result<PathBuf, FileError> {
        self.inner.absolute_path(path, cancel).await
    }
    async fn join_path(
        &self,
        parts: &[&str],
        cancel: Option<&CancellationToken>,
    ) -> Result<PathBuf, FileError> {
        self.inner.join_path(parts, cancel).await
    }
    async fn read_text_file(
        &self,
        path: &str,
        cancel: Option<&CancellationToken>,
    ) -> Result<String, FileError> {
        self.inner.read_text_file(path, cancel).await
    }
    async fn read_text_lines(
        &self,
        path: &str,
        max_lines: Option<usize>,
        cancel: Option<&CancellationToken>,
    ) -> Result<Vec<String>, FileError> {
        self.inner.read_text_lines(path, max_lines, cancel).await
    }
    async fn read_binary_file(
        &self,
        path: &str,
        cancel: Option<&CancellationToken>,
    ) -> Result<Vec<u8>, FileError> {
        self.inner.read_binary_file(path, cancel).await
    }
    async fn write_file(
        &self,
        path: &str,
        content: FileContent,
        cancel: Option<&CancellationToken>,
    ) -> Result<(), FileError> {
        self.inner.write_file(path, content, cancel).await
    }
    async fn append_file(
        &self,
        path: &str,
        content: FileContent,
        cancel: Option<&CancellationToken>,
    ) -> Result<(), FileError> {
        self.inner.append_file(path, content, cancel).await
    }
    async fn rename_file(
        &self,
        source: &str,
        dest: &str,
        cancel: Option<&CancellationToken>,
    ) -> Result<(), FileError> {
        self.inner.rename_file(source, dest, cancel).await
    }
    async fn file_info(
        &self,
        path: &str,
        cancel: Option<&CancellationToken>,
    ) -> Result<FileInfo, FileError> {
        self.inner.file_info(path, cancel).await
    }
    async fn list_dir(
        &self,
        path: &str,
        cancel: Option<&CancellationToken>,
    ) -> Result<Vec<FileInfo>, FileError> {
        self.inner.list_dir(path, cancel).await
    }
    async fn canonical_path(
        &self,
        path: &str,
        cancel: Option<&CancellationToken>,
    ) -> Result<PathBuf, FileError> {
        if self.first_lookup_pending.swap(false, Ordering::SeqCst) {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        self.inner.canonical_path(path, cancel).await
    }
    async fn exists(
        &self,
        path: &str,
        cancel: Option<&CancellationToken>,
    ) -> Result<bool, FileError> {
        self.inner.exists(path, cancel).await
    }
    async fn create_dir(
        &self,
        path: &str,
        recursive: bool,
        cancel: Option<&CancellationToken>,
    ) -> Result<(), FileError> {
        self.inner.create_dir(path, recursive, cancel).await
    }
    async fn remove(
        &self,
        path: &str,
        recursive: bool,
        force: bool,
        cancel: Option<&CancellationToken>,
    ) -> Result<(), FileError> {
        self.inner.remove(path, recursive, force, cancel).await
    }
    async fn create_temp_dir(
        &self,
        prefix: Option<&str>,
        cancel: Option<&CancellationToken>,
    ) -> Result<PathBuf, FileError> {
        self.inner.create_temp_dir(prefix, cancel).await
    }
    async fn create_temp_file(
        &self,
        prefix: &str,
        suffix: &str,
        cancel: Option<&CancellationToken>,
    ) -> Result<PathBuf, FileError> {
        self.inner.create_temp_file(prefix, suffix, cancel).await
    }
    async fn cleanup(&self) {
        let inner: &dyn FileSystem = &*self.inner;
        inner.cleanup().await;
    }
}

#[async_trait]
impl Shell for SlowFirstLookupEnv {
    async fn exec<'a>(
        &'a self,
        command: &str,
        options: ShellExecOptions<'a>,
    ) -> Result<ShellOutput, ice_tools::ExecutionError> {
        self.inner.exec(command, options).await
    }
    async fn cleanup(&self) {
        let inner: &dyn Shell = &*self.inner;
        inner.cleanup().await;
    }
}

impl ExecutionEnv for SlowFirstLookupEnv {}

#[tokio::test]
async fn a_slow_key_lookup_cannot_be_overtaken_by_a_later_call() {
    let env = Arc::new(SlowFirstLookupEnv {
        inner: Arc::new(InMemoryExecutionEnv::with_cwd("/w".into())),
        first_lookup_pending: AtomicBool::new(true),
        registry: MutationQueueRegistry::new(),
    });
    let order = Arc::new(std::sync::Mutex::new(Vec::new()));

    let run = |label: &'static str| {
        let env = env.clone();
        let order = order.clone();
        async move {
            let token = CancellationToken::new();
            with_file_mutation_queue(&*env, "same.txt", &token, |_| {
                Box::pin(async move {
                    order.lock().unwrap().push(label);
                    Ok(())
                })
            })
            .await
            .unwrap();
        }
    };
    let first = tokio::spawn(run("first"));
    // Let the first call start its (slow) key lookup.
    tokio::time::sleep(Duration::from_millis(10)).await;
    let second = tokio::spawn(run("second"));
    first.await.unwrap();
    second.await.unwrap();
    assert_eq!(*order.lock().unwrap(), vec!["first", "second"]);
}

#[tokio::test]
async fn different_files_still_run_concurrently() {
    let env = Arc::new(InMemoryExecutionEnv::with_cwd("/w".into()));
    let (held_tx, held_rx) = tokio::sync::oneshot::channel::<()>();
    let (release_tx, release_rx) = tokio::sync::oneshot::channel::<()>();
    let blocker = {
        let env = env.clone();
        tokio::spawn(async move {
            let token = CancellationToken::new();
            with_file_mutation_queue(&*env, "a.txt", &token, |_| {
                Box::pin(async move {
                    let _ = held_tx.send(());
                    let _ = release_rx.await;
                    Ok(())
                })
            })
            .await
            .unwrap();
        })
    };
    held_rx.await.unwrap();
    let token = CancellationToken::new();
    // b.txt is not blocked by the in-flight a.txt mutation.
    tokio::time::timeout(
        Duration::from_secs(2),
        with_file_mutation_queue(&*env, "b.txt", &token, |_| Box::pin(async { Ok(()) })),
    )
    .await
    .expect("b.txt mutation was blocked by a.txt")
    .unwrap();
    let _ = release_tx.send(());
    blocker.await.unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn symlinked_parent_and_missing_target_share_a_key() {
    let dir = tempfile::tempdir().unwrap();
    let real = dir.path().join("real");
    std::fs::create_dir(&real).unwrap();
    std::os::unix::fs::symlink(&real, dir.path().join("alias")).unwrap();
    let env = ice_tools::OsExecutionEnv::with_cwd(dir.path().to_path_buf());

    // Neither file exists yet; both spellings must queue together.
    let via_alias = get_mutation_queue_key(&env, "alias/new/file.txt", None)
        .await
        .unwrap();
    let via_real = get_mutation_queue_key(&env, "real/new/file.txt", None)
        .await
        .unwrap();
    assert_eq!(via_alias, via_real);
    let canonical_real = std::fs::canonicalize(&real).unwrap();
    assert_eq!(
        PathBuf::from(&via_real),
        canonical_real.join("new").join("file.txt")
    );
}

#[cfg(unix)]
#[tokio::test]
async fn dangling_symlink_keys_by_its_target() {
    let dir = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink("target.txt", dir.path().join("link.txt")).unwrap();
    let env = ice_tools::OsExecutionEnv::with_cwd(dir.path().to_path_buf());
    let via_link = get_mutation_queue_key(&env, "link.txt", None)
        .await
        .unwrap();
    let via_target = get_mutation_queue_key(&env, "target.txt", None)
        .await
        .unwrap();
    assert_eq!(via_link, via_target);
}

#[cfg(unix)]
#[tokio::test]
async fn dangling_symlink_target_preserves_symlink_aware_parent_segments() {
    let dir = tempfile::tempdir().unwrap();
    let actual = dir.path().join("actual");
    std::fs::create_dir_all(actual.join("sub")).unwrap();
    std::os::unix::fs::symlink(actual.join("sub"), dir.path().join("alias")).unwrap();
    std::os::unix::fs::symlink("alias/../target.txt", dir.path().join("dangling.txt")).unwrap();
    let env = ice_tools::OsExecutionEnv::with_cwd(dir.path().to_path_buf());

    let through_link = get_mutation_queue_key(&env, "dangling.txt", None)
        .await
        .unwrap();
    let through_target = get_mutation_queue_key(&env, "actual/target.txt", None)
        .await
        .unwrap();
    assert_eq!(through_link, through_target);
}
