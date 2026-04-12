/// Test helper: RAII guard that restores the working directory on drop.
/// Ensures CWD is restored even if a test panics mid-execution.
pub struct CwdGuard(std::path::PathBuf);

impl CwdGuard {
    pub fn new() -> Self {
        Self(std::env::current_dir().unwrap())
    }
}

impl Drop for CwdGuard {
    fn drop(&mut self) {
        let _ = std::env::set_current_dir(&self.0);
    }
}
