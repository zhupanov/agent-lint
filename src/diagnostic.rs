/// Collects lint diagnostics without aborting.
/// Mirrors the bash script's `fail()` + `ERROR_COUNT` collect-then-report pattern.
pub struct DiagnosticCollector {
    errors: Vec<String>,
}

impl DiagnosticCollector {
    pub fn new() -> Self {
        Self { errors: Vec::new() }
    }

    /// Record an error. Does not abort.
    pub fn fail(&mut self, msg: &str) {
        eprintln!("LINT ERROR: {msg}");
        self.errors.push(msg.to_string());
    }

    pub fn error_count(&self) -> usize {
        self.errors.len()
    }
}
