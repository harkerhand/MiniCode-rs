use std::sync::atomic::{AtomicBool, Ordering};

static MCP_LOG_ENABLED: AtomicBool = AtomicBool::new(true);

pub fn set_mcp_logging_enabled(enabled: bool) {
    MCP_LOG_ENABLED.store(enabled, Ordering::Relaxed);
}

pub(crate) fn mcp_log(message: impl AsRef<str>) {
    if !MCP_LOG_ENABLED.load(Ordering::Relaxed) {
        return;
    }
    eprintln!("\x1b[32m[mcp]\x1b[0m {}", message.as_ref());
}
