//! The `pi` binary entry point. Mirrors the TS
//! `packages/coding-agent/src/cli.ts` (the `#!/usr/bin/env node` shebang file
//! whose `bin` entry in `package.json` resolves to `dist/cli.js`, which just
//! calls `main(process.argv.slice(2))`).
//!
//! Rust's analogue: a thin wrapper that boots the multi-thread runtime the
//! harness needs (the harness's `StreamFn` bridge uses `block_in_place`, which
//! requires the multi-thread flavor) and hands control to
//! [`ice_cli::main::run`], exiting with its returned code.

use std::process::ExitCode;
use std::time::Duration;

/// How long leftover blocking work may delay exit after `run` has returned.
/// All cleanup (session shutdown, observe finish, shell disposal) is already
/// done by then; what remains is e.g. Tokio's blocking stdin read, which never
/// completes while a host keeps the pipe open after `stop` or a signal.
const BLOCKING_SHUTDOWN_GRACE: Duration = Duration::from_millis(500);

fn main() -> ExitCode {
    // `run` does everything else: arg parse, model resolution, harness build,
    // mode dispatch.
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("failed to build the Tokio runtime");
    let code = runtime.block_on(ice_cli::app::run());
    runtime.shutdown_timeout(BLOCKING_SHUTDOWN_GRACE);
    ExitCode::from(code as u8)
}
