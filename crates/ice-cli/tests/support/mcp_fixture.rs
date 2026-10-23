pub fn server_executable() -> std::path::PathBuf {
    let exe = std::env::current_exe().unwrap();
    let deps = exe.parent().unwrap();
    let suffix = std::env::consts::EXE_SUFFIX;
    let mut candidates: Vec<_> = std::fs::read_dir(deps)
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            let name = path.file_name().unwrap().to_string_lossy();
            let stem = name.strip_suffix(suffix).unwrap_or(&name);
            let Some(hash) = stem.strip_prefix("mcp_test_server-") else {
                return false;
            };
            !hash.is_empty() && hash.bytes().all(|byte| byte.is_ascii_hexdigit()) && path.is_file()
        })
        .collect();
    candidates.sort_by_key(|path| {
        std::fs::metadata(path)
            .and_then(|meta| meta.modified())
            .ok()
    });
    candidates
        .pop()
        .expect("build the mcp_test_server test target before running MCP tests")
}
