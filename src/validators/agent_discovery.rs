//! Shared recursive Markdown-file discovery across repository-relative roots.
//!
//! One collector feeds every agent rule: private `.claude/agents/`, the plugin
//! default `agents/`, and manifest-declared plugin agent roots. Discovery is
//! recursive because Claude Code scans agent directories recursively and an
//! agent's identity comes only from its `name` frontmatter field, not from the
//! subdirectory path. Traversal reuses [`crate::traversal::recursive_files`], so
//! it never follows symlinked directories and skips repository metadata,
//! dependency, and build trees.
//!
//! The discovery, dedup, and symlink-safety behavior is generic `.md`-across-roots
//! collection, so plugin-shipped output-style discovery
//! ([`super::claude_config::validate_plugin_output_styles`]) reuses the same
//! [`collect`] entry point rather than duplicating the security-sensitive
//! symlink-root handling.
//!
//! The returned [`AgentFileInventory`] carries two deterministic, normalized,
//! repository-relative, path-deduplicated vectors: `all_files` (every discovered
//! agent file before lint exclusion) and `lint_files` (the subset not matched by
//! [`ExcludeSet`]). Per-agent validators, A030, and override accounting consume
//! `lint_files`. `all_files` supports present-but-empty root accounting (A004)
//! and the reusable identity index that #344 / S065 consume; the collector never
//! applies `ExcludeSet` before populating `all_files`.

use crate::config::ExcludeSet;
use crate::frontmatter::{self, LeadingFrontmatterState};
use crate::traversal;
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

/// Two-vector inventory of discovered agent files.
///
/// Both vectors are sorted, normalized, repository-relative, and deduplicated by
/// path. `lint_files` is always a subset of `all_files`.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct AgentFileInventory {
    /// Every discovered `*.md` agent file, before `ExcludeSet` is applied.
    pub all_files: Vec<String>,
    /// The subset of `all_files` not matched by `ExcludeSet`.
    pub lint_files: Vec<String>,
}

/// The runtime agent roots and files visible to a skill.  Basic mode sees only
/// private agents; Plugin mode sees that private tree alongside the default and
/// manifest-declared plugin roots.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct RuntimeAgentInventory {
    /// Deduplicated roots in stable runtime search order.
    pub roots: Vec<String>,
    /// Files collected from those roots.
    pub files: AgentFileInventory,
    /// Canonical declared names and filename stems from `files.all_files`.
    pub identities: BTreeSet<String>,
}

/// One discovered agent root: its normalized repository-relative path, whether
/// it exists on disk, and the agent files found beneath it.
#[derive(Debug, Clone)]
pub(crate) struct AgentRoot {
    /// The root path exactly as supplied by the caller (already normalized).
    pub path: String,
    /// Whether the root exists on disk as a directory or file.
    pub exists: bool,
    /// Files discovered under this single root (before and after exclusion).
    pub inventory: AgentFileInventory,
}

/// Recursively discover `*.md` agent files under one repository-relative root.
///
/// A directory is walked recursively. A direct `*.md` file is itself one agent.
/// Any other existing entry (for example a non-Markdown file) exists but yields
/// no agents, which lets A004 distinguish present-but-empty intent from absence.
/// A missing path yields an absent, empty root. `all_files` omits the
/// `ExcludeSet`; `lint_files` applies it.
pub(crate) fn discover_root(root: &str, exclude: &ExcludeSet) -> AgentRoot {
    let base = Path::new(root);
    // A root must resolve strictly inside the repository. `WalkDir` would
    // otherwise descend through a symlinked root — or through a symlinked
    // ancestor of a declared multi-segment root, which the OS dereferences — and
    // pull files from outside the repository. The shared containment primitive
    // checks every path component (final and intermediate) for symlinks plus
    // canonical containment, so an escaping root is treated as absent: an
    // unusable in-repository root.
    let (exists, all_files) = if !crate::repo_path::is_repo_contained(base) {
        (false, Vec::new())
    } else if base.is_dir() {
        let files = traversal::recursive_files(base, Path::new("."), None)
            .entries
            .into_iter()
            .map(|entry| entry.display)
            .filter(|display| is_agent_markdown(display))
            .collect();
        (true, files)
    } else if base.is_file() {
        // A manifest-declared path may point directly at a single agent file.
        // A present non-Markdown file exists but contributes no agent.
        let files = if is_agent_markdown(root) {
            vec![root.to_string()]
        } else {
            Vec::new()
        };
        (true, files)
    } else {
        (false, Vec::new())
    };

    let lint_files = all_files
        .iter()
        .filter(|display| !exclude.is_excluded(display))
        .cloned()
        .collect();

    AgentRoot {
        path: root.to_string(),
        exists,
        inventory: AgentFileInventory {
            all_files,
            lint_files,
        },
    }
}

/// Merge several roots into one deduplicated, sorted inventory.
pub(crate) fn merge(roots: &[AgentRoot]) -> AgentFileInventory {
    let mut all_files = Vec::new();
    let mut lint_files = Vec::new();
    for root in roots {
        all_files.extend(root.inventory.all_files.iter().cloned());
        lint_files.extend(root.inventory.lint_files.iter().cloned());
    }
    dedupe_sorted(&mut all_files);
    dedupe_sorted(&mut lint_files);
    AgentFileInventory {
        all_files,
        lint_files,
    }
}

/// Discover and merge a set of repository-relative roots into one inventory.
///
/// Roots are deduplicated by path so an overlapping root is walked once; files
/// reached through more than one root still appear once in each vector.
pub(crate) fn collect(roots: &[&str], exclude: &ExcludeSet) -> AgentFileInventory {
    let mut seen = BTreeSet::new();
    let discovered: Vec<AgentRoot> = roots
        .iter()
        .filter(|root| seen.insert(**root))
        .map(|root| discover_root(root, exclude))
        .collect();
    merge(&discovered)
}

/// Collect the complete repository-local runtime namespace for a skill.
///
/// This is the one root resolver for S065: callers supply only the
/// manifest-validated plugin roots from `manifest::declared_agent_roots`.
/// Excluded files intentionally remain available through `all_files`, because
/// lint exclusion does not make an installed agent unavailable at runtime.
pub(crate) fn runtime_inventory(
    plugin_mode: bool,
    declared_roots: &[String],
    exclude: &ExcludeSet,
) -> RuntimeAgentInventory {
    let mut roots = Vec::new();
    if plugin_mode {
        roots.push("agents".to_string());
    }
    roots.push(".claude/agents".to_string());
    if plugin_mode {
        roots.extend(declared_roots.iter().cloned());
    }
    let mut seen = BTreeSet::new();
    roots.retain(|root| seen.insert(root.clone()));
    let root_refs = roots.iter().map(String::as_str).collect::<Vec<_>>();
    let files = collect(&root_refs, exclude);
    let identities = identities(&files);
    RuntimeAgentInventory {
        roots,
        files,
        identities,
    }
}

/// Return every canonical runtime identity represented by an inventory.
///
/// A readable agent contributes its filename stem even when its frontmatter is
/// missing or malformed, plus a canonical string `name` when available. Agent
/// validators own defects in that frontmatter; S065 only answers whether a
/// runtime reference can resolve.
pub(crate) fn identities(inventory: &AgentFileInventory) -> BTreeSet<String> {
    let mut identities = BTreeSet::new();
    for path in &inventory.all_files {
        if let Some(stem) = Path::new(path).file_stem().and_then(|stem| stem.to_str()) {
            identities.insert(stem.to_string());
        }
        let Ok(content) = fs::read_to_string(path) else {
            continue;
        };
        let LeadingFrontmatterState::Complete(block) = frontmatter::leading_frontmatter(&content)
        else {
            continue;
        };
        let lines = block.yaml.lines().map(str::to_owned).collect::<Vec<_>>();
        let Ok(value) = frontmatter::parse_yaml_strict(&lines) else {
            continue;
        };
        if let Some(name) = value
            .as_mapping()
            .and_then(|mapping| mapping.get("name"))
            .and_then(|value| value.as_str())
        {
            identities.insert(name.to_string());
        }
    }
    identities
}

fn is_agent_markdown(display: &str) -> bool {
    display.ends_with(".md")
}

fn dedupe_sorted(items: &mut Vec<String>) {
    items.sort();
    items.dedup();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    #[serial_test::serial]
    fn discovers_nested_markdown_recursively_and_ignores_non_markdown() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        fs::create_dir_all(".claude/agents/review/deep").unwrap();
        fs::write(".claude/agents/top.md", "").unwrap();
        fs::write(".claude/agents/review/mid.md", "").unwrap();
        fs::write(".claude/agents/review/deep/leaf.md", "").unwrap();
        fs::write(".claude/agents/review/notes.txt", "").unwrap();

        let root = discover_root(".claude/agents", &ExcludeSet::default());
        assert!(root.exists);
        assert_eq!(
            root.inventory.all_files,
            vec![
                ".claude/agents/review/deep/leaf.md".to_string(),
                ".claude/agents/review/mid.md".to_string(),
                ".claude/agents/top.md".to_string(),
            ]
        );
        assert_eq!(root.inventory.lint_files, root.inventory.all_files);
    }

    #[test]
    #[serial_test::serial]
    fn all_files_precedes_exclusion_and_lint_files_follows_it() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        fs::create_dir_all(".claude/agents/review").unwrap();
        fs::write(".claude/agents/included.md", "").unwrap();
        fs::write(".claude/agents/review/excluded.md", "").unwrap();

        let exclude = ExcludeSet::new(&[".claude/agents/review/**".to_string()]).unwrap();
        let root = discover_root(".claude/agents", &exclude);

        // The reusable identity vector keeps the excluded file; lint accounting drops it.
        assert_eq!(
            root.inventory.all_files,
            vec![
                ".claude/agents/included.md".to_string(),
                ".claude/agents/review/excluded.md".to_string(),
            ]
        );
        assert_eq!(
            root.inventory.lint_files,
            vec![".claude/agents/included.md".to_string()]
        );
    }

    #[test]
    #[serial_test::serial]
    fn direct_markdown_file_root_is_one_agent_and_non_markdown_is_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        fs::create_dir_all("custom").unwrap();
        fs::write("custom/agent.md", "").unwrap();
        fs::write("custom/notes.txt", "").unwrap();

        let md = discover_root("custom/agent.md", &ExcludeSet::default());
        assert!(md.exists);
        assert_eq!(md.inventory.all_files, vec!["custom/agent.md".to_string()]);

        let non_md = discover_root("custom/notes.txt", &ExcludeSet::default());
        assert!(non_md.exists, "a present non-Markdown file exists");
        assert!(
            non_md.inventory.all_files.is_empty(),
            "but contributes no agent"
        );
    }

    #[test]
    #[serial_test::serial]
    fn missing_root_is_absent_and_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        let root = discover_root("agents", &ExcludeSet::default());
        assert!(!root.exists);
        assert!(root.inventory.all_files.is_empty());
        assert!(root.inventory.lint_files.is_empty());
    }

    #[cfg(unix)]
    #[test]
    #[serial_test::serial]
    fn a_symlinked_root_never_escapes_the_repository() {
        use std::os::unix::fs::symlink;

        let outside = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(outside.path().join("secret")).unwrap();
        std::fs::write(outside.path().join("secret/leak.md"), "").unwrap();

        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        // A declared root that is a symlink pointing outside the repository.
        symlink(
            outside.path().join("secret"),
            tmp.path().join("custom-agents"),
        )
        .unwrap();

        let root = discover_root("custom-agents", &ExcludeSet::default());
        assert!(
            root.inventory.all_files.is_empty(),
            "outside-repository files must never be discovered through a symlinked root: {:?}",
            root.inventory.all_files
        );
    }

    #[cfg(unix)]
    #[test]
    #[serial_test::serial]
    fn a_symlinked_ancestor_of_a_declared_root_never_escapes_the_repository() {
        use std::os::unix::fs::symlink;

        let outside = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(outside.path().join("agents")).unwrap();
        std::fs::write(outside.path().join("agents/leak.md"), "").unwrap();

        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        // `via` is an intermediate symlink to an outside directory; the declared
        // root is `via/agents`. The OS dereferences `via`, so a guard that only
        // inspects the final component would walk outside the repository.
        symlink(outside.path(), tmp.path().join("via")).unwrap();

        let root = discover_root("via/agents", &ExcludeSet::default());
        assert!(
            root.inventory.all_files.is_empty(),
            "a symlinked ancestor must never pull in outside-repository files: {:?}",
            root.inventory.all_files
        );
    }

    #[test]
    #[serial_test::serial]
    fn collect_deduplicates_overlapping_roots_and_files() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        fs::create_dir_all("agents/review").unwrap();
        fs::write("agents/top.md", "").unwrap();
        fs::write("agents/review/nested.md", "").unwrap();

        // "agents" and "agents/review" overlap; "agents" repeats verbatim.
        let inventory = collect(
            &["agents", "agents/review", "agents"],
            &ExcludeSet::default(),
        );
        assert_eq!(
            inventory.all_files,
            vec![
                "agents/review/nested.md".to_string(),
                "agents/top.md".to_string(),
            ],
            "overlapping roots contribute each physical file exactly once"
        );
    }
}
