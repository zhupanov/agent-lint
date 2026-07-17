use crate::rules::{ALL_RULES, LintRule};
use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
use serde::Deserialize;
use std::collections::{BTreeMap, HashSet};
use std::path::Path;

/// CLI strictness mode. Applied as a one-shot transformation to LintConfig
/// before creating DiagnosticCollector. Not configurable via TOML.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CliMode {
    #[default]
    Normal,
    /// Promotes warn-listed and default-warning rules to errors (except
    /// too-long rules). Respects suppress list. Default-suppressed rules
    /// stay suppressed.
    Pedantic,
    /// All rules fire as errors. Ignores all TOML severity config.
    All,
}

/// Raw TOML structure for deserialization.
#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct RawConfig {
    lint: Option<RawLintSection>,
    platforms: Option<RawPlatformsSection>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct RawPlatformsSection {
    cursor: Option<bool>,
    codex: Option<bool>,
}

/// Optional per-platform activation overrides from `[platforms]`.
/// `None` means activate only when a unique platform surface is discovered.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PlatformOverrides {
    pub cursor: Option<bool>,
    pub codex: Option<bool>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawLintSection {
    #[serde(default)]
    suppress: Vec<String>,
    #[serde(default)]
    error: Vec<String>,
    #[serde(default)]
    warn: Vec<String>,
    #[serde(default)]
    exclude: Vec<String>,
    #[serde(
        default = "default_desc_truncated_max_chars",
        rename = "desc-truncated-max-chars"
    )]
    desc_truncated_max_chars: usize,
    #[serde(default, rename = "skill-closure-max-lines")]
    skill_closure_max_lines: Option<usize>,
    #[serde(default, rename = "claude-import-max-lines")]
    claude_import_max_lines: Option<usize>,
    #[serde(default, rename = "claude-import-total-max-lines")]
    claude_import_total_max_lines: Option<usize>,
    #[serde(default, rename = "claude-import-path-budgets")]
    claude_import_path_budgets: BTreeMap<String, usize>,
    #[serde(default, rename = "prompt-source-budgets")]
    prompt_source_budgets: Vec<RawPromptSourceBudget>,
    #[serde(default = "default_instruction_files", rename = "instruction-files")]
    instruction_files: Vec<String>,
    #[serde(
        default = "default_inline_path_prefixes",
        rename = "inline-path-prefixes"
    )]
    inline_path_prefixes: Vec<String>,
    #[serde(default, rename = "script-inventory")]
    script_inventory: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPromptSourceBudget {
    name: String,
    roots: Vec<String>,
    #[serde(default, rename = "conditional-sources")]
    conditional_sources: Vec<String>,
    #[serde(default, rename = "root-max-lines")]
    root_max_lines: Option<usize>,
    #[serde(default, rename = "root-max-tokens")]
    root_max_tokens: Option<usize>,
    #[serde(default, rename = "root-max-content-tokens")]
    root_max_content_tokens: Option<usize>,
    #[serde(default, rename = "closure-max-lines")]
    closure_max_lines: Option<usize>,
    #[serde(default, rename = "closure-max-tokens")]
    closure_max_tokens: Option<usize>,
    #[serde(default, rename = "closure-max-content-tokens")]
    closure_max_content_tokens: Option<usize>,
    #[serde(default, rename = "conditional-max-lines")]
    conditional_max_lines: Option<usize>,
    #[serde(default, rename = "conditional-max-tokens")]
    conditional_max_tokens: Option<usize>,
    #[serde(default, rename = "conditional-max-content-tokens")]
    conditional_max_content_tokens: Option<usize>,
}

/// Optional caps for the three stable prompt-source size metrics.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PromptMetricCaps {
    pub lines: Option<usize>,
    pub estimated_tokens: Option<usize>,
    pub content_tokens: Option<usize>,
}

/// One repository-neutral prompt-source group configured for S062.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptSourceBudget {
    pub name: String,
    pub roots: Vec<String>,
    pub conditional_sources: Vec<String>,
    pub root_caps: PromptMetricCaps,
    pub closure_caps: PromptMetricCaps,
    pub conditional_caps: PromptMetricCaps,
}

const fn default_desc_truncated_max_chars() -> usize {
    250
}

fn default_instruction_files() -> Vec<String> {
    vec!["AGENTS.md".into(), "SECURITY.md".into(), "CLAUDE.md".into()]
}

fn default_inline_path_prefixes() -> Vec<String> {
    [
        "src/",
        "skills/",
        "scripts/",
        "docs/",
        "hooks/",
        "agents/",
        ".claude/",
        ".claude-plugin/",
        ".github/",
    ]
    .into_iter()
    .map(str::to_string)
    .collect()
}

impl Default for RawLintSection {
    fn default() -> Self {
        Self {
            suppress: Vec::new(),
            error: Vec::new(),
            warn: Vec::new(),
            exclude: Vec::new(),
            desc_truncated_max_chars: default_desc_truncated_max_chars(),
            skill_closure_max_lines: None,
            claude_import_max_lines: None,
            claude_import_total_max_lines: None,
            claude_import_path_budgets: BTreeMap::new(),
            prompt_source_budgets: Vec::new(),
            instruction_files: default_instruction_files(),
            inline_path_prefixes: default_inline_path_prefixes(),
            script_inventory: None,
        }
    }
}

/// Resolved lint configuration. Rules in `suppress` are completely suppressed.
/// Rules in `error` are promoted to errors (overriding default severity).
/// Rules in `warn` are downgraded to warnings. Priority: suppress > error > warn.
/// Rules not in any set fall back to `LintRule::default_severity()`.
#[derive(Debug, Clone)]
pub struct LintConfig {
    pub suppress: HashSet<LintRule>,
    pub error: HashSet<LintRule>,
    pub warn: HashSet<LintRule>,
    pub exclude: Vec<String>,
    pub desc_truncated_max_chars: usize,
    pub skill_closure_max_lines: Option<usize>,
    pub claude_import_max_lines: Option<usize>,
    pub claude_import_total_max_lines: Option<usize>,
    pub claude_import_path_budgets: BTreeMap<String, usize>,
    pub prompt_source_budgets: Vec<PromptSourceBudget>,
    pub instruction_files: Vec<String>,
    pub inline_path_prefixes: Vec<String>,
    /// Explicit repository-relative script paths used by G009-G011. `None`
    /// selects conventional script discovery instead.
    pub script_inventory: Option<Vec<String>>,
    pub platforms: PlatformOverrides,
}

impl Default for LintConfig {
    fn default() -> Self {
        Self {
            suppress: HashSet::new(),
            error: HashSet::new(),
            warn: HashSet::new(),
            exclude: Vec::new(),
            desc_truncated_max_chars: default_desc_truncated_max_chars(),
            skill_closure_max_lines: None,
            claude_import_max_lines: None,
            claude_import_total_max_lines: None,
            claude_import_path_budgets: BTreeMap::new(),
            prompt_source_budgets: Vec::new(),
            instruction_files: default_instruction_files(),
            inline_path_prefixes: default_inline_path_prefixes(),
            script_inventory: None,
            platforms: PlatformOverrides::default(),
        }
    }
}

/// Compiled glob set for file exclusion. Wraps `globset::GlobSet` and provides
/// path normalization. Use `ExcludeSet::default()` for an empty set that matches
/// nothing.
pub struct ExcludeSet {
    globs: GlobSet,
}

impl Default for ExcludeSet {
    fn default() -> Self {
        Self {
            globs: GlobSet::empty(),
        }
    }
}

impl ExcludeSet {
    /// Build an `ExcludeSet` from raw glob pattern strings.
    /// Returns `Err` if any pattern is invalid.
    pub fn new(patterns: &[String]) -> Result<Self, String> {
        if patterns.is_empty() {
            return Ok(Self::default());
        }
        let mut builder = GlobSetBuilder::new();
        for pattern in patterns {
            let glob = GlobBuilder::new(pattern)
                .literal_separator(true)
                .build()
                .map_err(|e| format!("invalid exclude glob pattern '{pattern}': {e}"))?;
            builder.add(glob);
        }
        let globs = builder
            .build()
            .map_err(|e| format!("failed to compile exclude patterns: {e}"))?;
        Ok(Self { globs })
    }

    /// Check whether a path should be excluded from linting.
    /// Normalizes the path before matching: strips leading `./` and
    /// converts backslashes to forward slashes.
    pub fn is_excluded(&self, path: &str) -> bool {
        let normalized = normalize_path(path);
        self.globs.is_match(&normalized)
    }
}

/// Normalize a path for consistent glob matching: strip leading `./`,
/// convert `\` to `/`.
pub fn normalize_path(path: &str) -> String {
    let s = path.replace('\\', "/");
    s.strip_prefix("./").unwrap_or(&s).to_string()
}

impl LintConfig {
    /// Load configuration from `agent-lint.toml` in the given repo root.
    ///
    /// - Missing file → default (empty) config.
    /// - Malformed TOML or unknown rule code/name → `Err(msg)`.
    pub fn load(repo_root: &str) -> Result<Self, String> {
        let path = Path::new(repo_root).join("agent-lint.toml");
        if !path.is_file() {
            let legacy = Path::new(repo_root).join("claude-lint.toml");
            if legacy.is_file() {
                eprintln!(
                    "warning: found 'claude-lint.toml' which is no longer read; rename it to 'agent-lint.toml'"
                );
            }
            return Ok(Self::default());
        }

        let content = std::fs::read_to_string(&path)
            .map_err(|e| format!("cannot read {}: {e}", path.display()))?;

        let raw: RawConfig =
            toml::from_str(&content).map_err(|e| format!("{}: {e}", path.display()))?;

        let section = raw.lint.unwrap_or_default();
        let platforms = raw.platforms.unwrap_or_default();

        if section.desc_truncated_max_chars == 0 {
            return Err(format!(
                "{}: desc-truncated-max-chars must be greater than zero",
                path.display()
            ));
        }
        for (name, value) in [
            ("skill-closure-max-lines", section.skill_closure_max_lines),
            ("claude-import-max-lines", section.claude_import_max_lines),
            (
                "claude-import-total-max-lines",
                section.claude_import_total_max_lines,
            ),
        ] {
            if value == Some(0) {
                return Err(format!(
                    "{}: {name} must be greater than zero",
                    path.display()
                ));
            }
        }
        validate_relative_paths(&section.instruction_files, "instruction-files", false)
            .map_err(|message| format!("{}: {message}", path.display()))?;
        validate_relative_paths(&section.inline_path_prefixes, "inline-path-prefixes", true)
            .map_err(|message| format!("{}: {message}", path.display()))?;
        let claude_import_path_budgets =
            load_import_path_budgets(Path::new(repo_root), section.claude_import_path_budgets)
                .map_err(|message| format!("{}: {message}", path.display()))?;
        let prompt_source_budgets =
            load_prompt_source_budgets(Path::new(repo_root), section.prompt_source_budgets)
                .map_err(|message| format!("{}: {message}", path.display()))?;
        let script_inventory = section
            .script_inventory
            .as_deref()
            .map(|inventory| load_script_inventory(Path::new(repo_root), inventory))
            .transpose()
            .map_err(|message| format!("{}: {message}", path.display()))?;

        // Parse error list first (user-explicit error promotions).
        let mut error = HashSet::new();
        for entry in &section.error {
            let rule = LintRule::from_code_or_name(entry).ok_or_else(|| {
                format!(
                    "{}: unknown rule in error list: '{entry}'. Use a valid code (e.g. M001) or name (e.g. plugin-json-missing).",
                    path.display()
                )
            })?;
            error.insert(rule);
        }

        // Parse warn list. error wins over warn.
        let mut warn = HashSet::new();
        for entry in &section.warn {
            let rule = LintRule::from_code_or_name(entry).ok_or_else(|| {
                format!(
                    "{}: unknown rule in warn list: '{entry}'. Use a valid code (e.g. M001) or name (e.g. plugin-json-missing).",
                    path.display()
                )
            })?;
            if !error.contains(&rule) {
                warn.insert(rule);
            }
        }

        // Parse suppress list. suppress wins over error and warn.
        let mut suppress = HashSet::new();
        for entry in &section.suppress {
            let rule = LintRule::from_code_or_name(entry).ok_or_else(|| {
                format!(
                    "{}: unknown rule in suppress list: '{entry}'. Use a valid code (e.g. M001) or name (e.g. plugin-json-missing).",
                    path.display()
                )
            })?;
            error.remove(&rule);
            warn.remove(&rule);
            suppress.insert(rule);
        }

        // Validate exclude patterns at load time (compile a throwaway GlobSet).
        ExcludeSet::new(&section.exclude).map_err(|e| format!("{}: {e}", path.display()))?;

        Ok(Self {
            suppress,
            error,
            warn,
            exclude: section.exclude,
            desc_truncated_max_chars: section.desc_truncated_max_chars,
            skill_closure_max_lines: section.skill_closure_max_lines,
            claude_import_max_lines: section.claude_import_max_lines,
            claude_import_total_max_lines: section.claude_import_total_max_lines,
            claude_import_path_budgets,
            prompt_source_budgets,
            instruction_files: section.instruction_files,
            inline_path_prefixes: section.inline_path_prefixes,
            script_inventory,
            platforms: PlatformOverrides {
                cursor: platforms.cursor,
                codex: platforms.codex,
            },
        })
    }

    /// Apply CLI strictness mode. Transforms the suppress/error/warn sets
    /// so that `DiagnosticCollector::report()` needs no changes.
    ///
    /// - `Pedantic`: moves warn entries and default-warning rules to error
    ///   (except too-long rules). Respects suppress list. Default-suppressed
    ///   rules stay suppressed.
    /// - `All`: clears suppress/warn, fills error with all rules. Overrides
    ///   all TOML severity config. File exclusions (`exclude`) are not
    ///   affected — `--all` changes rule severity, not file selection.
    pub fn apply_cli_mode(&mut self, mode: CliMode) {
        use crate::rules::DefaultSeverity;
        match mode {
            CliMode::Normal => {}
            CliMode::Pedantic => {
                // Promote user-warn rules to error (except too-long).
                let to_promote: Vec<_> = self
                    .warn
                    .iter()
                    .filter(|r| !r.is_too_long())
                    .copied()
                    .collect();
                for r in to_promote {
                    self.warn.remove(&r);
                    self.error.insert(r);
                }
                // Promote default-warning rules to error (except too-long
                // and already-suppressed).
                for r in ALL_RULES {
                    if r.default_severity() == DefaultSeverity::Warning
                        && !r.is_too_long()
                        && !self.suppress.contains(r)
                    {
                        self.error.insert(*r);
                    }
                }
            }
            CliMode::All => {
                self.suppress.clear();
                self.warn.clear();
                self.error.clear();
                for r in ALL_RULES {
                    self.error.insert(*r);
                }
            }
        }
    }

    /// Build a compiled `ExcludeSet` from this config's exclude patterns.
    /// This should be called once after loading and passed through to validators.
    pub fn build_exclude_set(&self) -> ExcludeSet {
        // Patterns were already validated in load(), so unwrap is safe.
        ExcludeSet::new(&self.exclude).expect("exclude patterns were validated at load time")
    }
}

fn load_script_inventory(root: &Path, inventory: &str) -> Result<Vec<String>, String> {
    validate_relative_paths(&[inventory.to_string()], "script-inventory", false)?;
    let canonical_root = root
        .canonicalize()
        .map_err(|error| format!("cannot resolve repository root: {error}"))?;
    let inventory_path = canonical_root.join(inventory);
    validate_inventory_file(
        &canonical_root,
        &inventory_path,
        &format!("script-inventory '{inventory}'"),
    )?;
    let content = std::fs::read_to_string(&inventory_path).map_err(|error| {
        format!("script-inventory '{inventory}' cannot be read as UTF-8: {error}")
    })?;
    let mut paths = Vec::new();
    for (index, raw) in content.lines().enumerate() {
        let value = raw.trim();
        if value.is_empty() || value.starts_with('#') {
            continue;
        }
        validate_relative_paths(&[value.to_string()], "script-inventory", false)
            .map_err(|message| format!("{message} on line {}", index + 1))?;
        if !is_supported_script_path(value) {
            return Err(format!(
                "script-inventory entry '{value}' on line {} must end in .sh, .inc.bash, or .awk",
                index + 1
            ));
        }
        let script_path = canonical_root.join(value);
        validate_inventory_file(
            &canonical_root,
            &script_path,
            &format!("script-inventory entry '{value}' on line {}", index + 1),
        )?;
        paths.push(normalize_path(value));
    }
    paths.sort();
    paths.dedup();
    Ok(paths)
}

fn load_import_path_budgets(
    root: &Path,
    raw: BTreeMap<String, usize>,
) -> Result<BTreeMap<String, usize>, String> {
    let mut budgets = BTreeMap::new();
    for (path, cap) in raw {
        if cap == 0 {
            return Err(format!(
                "claude-import-path-budgets entry '{path}' must be greater than zero"
            ));
        }
        let normalized = normalize_config_path(&path, "claude-import-path-budgets")?;
        validate_source_file(root, &normalized, "claude-import-path-budgets")?;
        if budgets.insert(normalized.clone(), cap).is_some() {
            return Err(format!(
                "claude-import-path-budgets contains duplicate normalized path '{normalized}'"
            ));
        }
    }
    Ok(budgets)
}

fn load_prompt_source_budgets(
    root: &Path,
    raw: Vec<RawPromptSourceBudget>,
) -> Result<Vec<PromptSourceBudget>, String> {
    let mut names = HashSet::new();
    let mut budgets = Vec::new();
    for budget in raw {
        if budget.name.is_empty()
            || !budget
                .name
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
        {
            return Err(format!(
                "prompt-source-budgets name '{}' must use only ASCII letters, digits, '.', '_', or '-'",
                budget.name
            ));
        }
        if !names.insert(budget.name.clone()) {
            return Err(format!(
                "prompt-source-budgets contains duplicate name '{}'",
                budget.name
            ));
        }
        if budget.roots.is_empty() {
            return Err(format!(
                "prompt-source-budgets '{}' must configure at least one root",
                budget.name
            ));
        }

        let root_caps = PromptMetricCaps {
            lines: budget.root_max_lines,
            estimated_tokens: budget.root_max_tokens,
            content_tokens: budget.root_max_content_tokens,
        };
        let closure_caps = PromptMetricCaps {
            lines: budget.closure_max_lines,
            estimated_tokens: budget.closure_max_tokens,
            content_tokens: budget.closure_max_content_tokens,
        };
        let conditional_caps = PromptMetricCaps {
            lines: budget.conditional_max_lines,
            estimated_tokens: budget.conditional_max_tokens,
            content_tokens: budget.conditional_max_content_tokens,
        };
        validate_metric_caps(&budget.name, "root", root_caps)?;
        validate_metric_caps(&budget.name, "closure", closure_caps)?;
        validate_metric_caps(&budget.name, "conditional", conditional_caps)?;
        if [root_caps, closure_caps, conditional_caps]
            .iter()
            .all(|caps| {
                caps.lines.is_none()
                    && caps.estimated_tokens.is_none()
                    && caps.content_tokens.is_none()
            })
        {
            return Err(format!(
                "prompt-source-budgets '{}' must configure at least one maximum",
                budget.name
            ));
        }
        if budget.conditional_sources.is_empty()
            && (conditional_caps.lines.is_some()
                || conditional_caps.estimated_tokens.is_some()
                || conditional_caps.content_tokens.is_some())
        {
            return Err(format!(
                "prompt-source-budgets '{}' configures conditional maxima without conditional-sources",
                budget.name
            ));
        }

        let mut paths = HashSet::new();
        let roots = normalize_budget_paths(root, &budget.name, "roots", budget.roots, &mut paths)?;
        let conditional_sources = normalize_budget_paths(
            root,
            &budget.name,
            "conditional-sources",
            budget.conditional_sources,
            &mut paths,
        )?;
        budgets.push(PromptSourceBudget {
            name: budget.name,
            roots,
            conditional_sources,
            root_caps,
            closure_caps,
            conditional_caps,
        });
    }
    budgets.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(budgets)
}

fn validate_metric_caps(group: &str, scope: &str, caps: PromptMetricCaps) -> Result<(), String> {
    if [caps.lines, caps.estimated_tokens, caps.content_tokens]
        .into_iter()
        .flatten()
        .any(|cap| cap == 0)
    {
        return Err(format!(
            "prompt-source-budgets '{group}' {scope} maxima must be greater than zero"
        ));
    }
    Ok(())
}

fn normalize_budget_paths(
    root: &Path,
    group: &str,
    field: &str,
    raw: Vec<String>,
    seen: &mut HashSet<String>,
) -> Result<Vec<String>, String> {
    let mut paths = Vec::new();
    for path in raw {
        let normalized = normalize_config_path(&path, "prompt-source-budgets")?;
        validate_source_file(root, &normalized, "prompt-source-budgets")?;
        if !seen.insert(normalized.clone()) {
            return Err(format!(
                "prompt-source-budgets '{group}' has duplicate normalized source '{normalized}'"
            ));
        }
        paths.push(normalized);
    }
    paths.sort();
    if field == "roots" && paths.is_empty() {
        return Err(format!(
            "prompt-source-budgets '{group}' must configure at least one root"
        ));
    }
    Ok(paths)
}

fn normalize_config_path(value: &str, name: &str) -> Result<String, String> {
    let normalized_separators = normalize_path(value);
    validate_relative_paths(std::slice::from_ref(&normalized_separators), name, false)?;
    let mut parts = Vec::new();
    for component in Path::new(&normalized_separators).components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::Normal(part) => parts.push(part.to_string_lossy()),
            _ => {
                return Err(format!(
                    "{name} entry '{value}' must be a safe repository-relative path"
                ));
            }
        }
    }
    let normalized = parts.join("/");
    if normalized.is_empty() {
        return Err(format!("{name} entry '{value}' must name a file"));
    }
    Ok(normalized)
}

fn validate_source_file(root: &Path, relative: &str, name: &str) -> Result<(), String> {
    let canonical_root = root
        .canonicalize()
        .map_err(|error| format!("cannot resolve repository root: {error}"))?;
    let path = canonical_root.join(relative);
    let metadata = std::fs::symlink_metadata(&path)
        .map_err(|error| format!("{name} source '{relative}' cannot be read: {error}"))?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(format!("{name} source '{relative}' must be a regular file"));
    }
    let canonical = path
        .canonicalize()
        .map_err(|error| format!("{name} source '{relative}' cannot be resolved: {error}"))?;
    if !canonical.starts_with(canonical_root) {
        return Err(format!(
            "{name} source '{relative}' resolves outside the repository root"
        ));
    }
    std::fs::read_to_string(&canonical)
        .map_err(|error| format!("{name} source '{relative}' cannot be read as UTF-8: {error}"))?;
    Ok(())
}

fn validate_inventory_file(root: &Path, path: &Path, label: &str) -> Result<(), String> {
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|error| format!("{label} cannot be read: {error}"))?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(format!("{label} must be a regular file"));
    }
    let canonical = path
        .canonicalize()
        .map_err(|error| format!("{label} cannot be resolved: {error}"))?;
    if !canonical.starts_with(root) {
        return Err(format!("{label} resolves outside the repository root"));
    }
    Ok(())
}

fn is_supported_script_path(path: &str) -> bool {
    path.ends_with(".sh") || path.ends_with(".inc.bash") || path.ends_with(".awk")
}

fn validate_relative_paths(
    values: &[String],
    name: &str,
    require_slash: bool,
) -> Result<(), String> {
    for value in values {
        let candidate = Path::new(value);
        if value.is_empty()
            || candidate.is_absolute()
            || candidate
                .components()
                .any(|part| matches!(part, std::path::Component::ParentDir))
            || (require_slash && !value.ends_with('/'))
        {
            return Err(format!(
                "{name} entry '{value}' must be a safe repository-relative {}",
                if require_slash {
                    "prefix ending in /"
                } else {
                    "path"
                }
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[serial_test::serial]
    fn missing_config_file_returns_default() {
        let tmp = tempfile::tempdir().unwrap();
        let config = LintConfig::load(tmp.path().to_str().unwrap()).unwrap();
        assert!(config.suppress.is_empty());
        assert!(config.error.is_empty());
        assert!(config.warn.is_empty());
    }

    #[test]
    #[serial_test::serial]
    fn valid_config_by_code() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("agent-lint.toml"),
            "[lint]\nsuppress = [\"M001\"]\nwarn = [\"G005\"]\n",
        )
        .unwrap();
        let config = LintConfig::load(tmp.path().to_str().unwrap()).unwrap();
        assert!(config.suppress.contains(&LintRule::PluginJsonMissing));
        assert!(config.warn.contains(&LintRule::SecurityMdMissing));
    }

    #[test]
    #[serial_test::serial]
    fn valid_config_by_name() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("agent-lint.toml"),
            "[lint]\nsuppress = [\"plugin-json-missing\"]\nwarn = [\"security-md-missing\"]\n",
        )
        .unwrap();
        let config = LintConfig::load(tmp.path().to_str().unwrap()).unwrap();
        assert!(config.suppress.contains(&LintRule::PluginJsonMissing));
        assert!(config.warn.contains(&LintRule::SecurityMdMissing));
    }

    #[test]
    #[serial_test::serial]
    fn suppress_wins_over_warn() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("agent-lint.toml"),
            "[lint]\nsuppress = [\"M001\"]\nwarn = [\"M001\"]\n",
        )
        .unwrap();
        let config = LintConfig::load(tmp.path().to_str().unwrap()).unwrap();
        assert!(config.suppress.contains(&LintRule::PluginJsonMissing));
        assert!(!config.warn.contains(&LintRule::PluginJsonMissing));
    }

    #[test]
    #[serial_test::serial]
    fn unknown_code_returns_error() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("agent-lint.toml"),
            "[lint]\nsuppress = [\"X999\"]\n",
        )
        .unwrap();
        let err = LintConfig::load(tmp.path().to_str().unwrap()).unwrap_err();
        assert!(
            err.contains("unknown rule"),
            "Expected unknown rule error, got: {err}"
        );
        assert!(err.contains("X999"));
    }

    #[test]
    #[serial_test::serial]
    fn malformed_toml_returns_error() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("agent-lint.toml"), "not valid toml {{{\n").unwrap();
        let err = LintConfig::load(tmp.path().to_str().unwrap()).unwrap_err();
        assert!(!err.is_empty());
    }

    #[test]
    #[serial_test::serial]
    fn empty_lint_section_is_valid() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("agent-lint.toml"), "[lint]\n").unwrap();
        let config = LintConfig::load(tmp.path().to_str().unwrap()).unwrap();
        assert!(config.suppress.is_empty());
        assert!(config.error.is_empty());
        assert!(config.warn.is_empty());
    }

    #[test]
    #[serial_test::serial]
    fn no_lint_section_is_valid() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("agent-lint.toml"), "# empty config\n").unwrap();
        let config = LintConfig::load(tmp.path().to_str().unwrap()).unwrap();
        assert!(config.suppress.is_empty());
        assert!(config.error.is_empty());
        assert!(config.warn.is_empty());
    }

    #[test]
    #[serial_test::serial]
    fn platform_overrides_are_parsed() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("agent-lint.toml"),
            "[platforms]\ncursor = true\ncodex = false\n",
        )
        .unwrap();
        let config = LintConfig::load(tmp.path().to_str().unwrap()).unwrap();
        assert_eq!(config.platforms.cursor, Some(true));
        assert_eq!(config.platforms.codex, Some(false));
    }

    #[test]
    #[serial_test::serial]
    fn unknown_platform_key_returns_error() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("agent-lint.toml"),
            "[platforms]\nother = true\n",
        )
        .unwrap();
        let err = LintConfig::load(tmp.path().to_str().unwrap()).unwrap_err();
        assert!(err.contains("unknown field"), "unexpected error: {err}");
    }

    #[test]
    #[serial_test::serial]
    fn typo_in_section_name_returns_error() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("agent-lint.toml"),
            "[lnt]\nsuppress = [\"M001\"]\n",
        )
        .unwrap();
        let err = LintConfig::load(tmp.path().to_str().unwrap()).unwrap_err();
        assert!(
            err.contains("unknown field"),
            "Expected unknown field error, got: {err}"
        );
    }

    #[test]
    #[serial_test::serial]
    fn typo_in_field_name_returns_error() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("agent-lint.toml"),
            "[lint]\nwran = [\"M001\"]\n",
        )
        .unwrap();
        let err = LintConfig::load(tmp.path().to_str().unwrap()).unwrap_err();
        assert!(
            err.contains("unknown field"),
            "Expected unknown field error, got: {err}"
        );
    }

    #[test]
    #[serial_test::serial]
    fn old_ignore_field_returns_error() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("agent-lint.toml"),
            "[lint]\nignore = [\"M001\"]\n",
        )
        .unwrap();
        let err = LintConfig::load(tmp.path().to_str().unwrap()).unwrap_err();
        assert!(
            err.contains("unknown field"),
            "Expected unknown field error for old 'ignore' syntax, got: {err}"
        );
    }

    // ── Error list ──────────────────────────────────────────────────

    #[test]
    #[serial_test::serial]
    fn error_list_parsed() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("agent-lint.toml"),
            "[lint]\nerror = [\"S033\", \"G005\"]\n",
        )
        .unwrap();
        let config = LintConfig::load(tmp.path().to_str().unwrap()).unwrap();
        assert!(config.error.contains(&LintRule::NameVague));
        assert!(config.error.contains(&LintRule::SecurityMdMissing));
    }

    #[test]
    #[serial_test::serial]
    fn error_list_by_name() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("agent-lint.toml"),
            "[lint]\nerror = [\"name-vague\"]\n",
        )
        .unwrap();
        let config = LintConfig::load(tmp.path().to_str().unwrap()).unwrap();
        assert!(config.error.contains(&LintRule::NameVague));
    }

    #[test]
    #[serial_test::serial]
    fn unknown_error_code_returns_error() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("agent-lint.toml"),
            "[lint]\nerror = [\"X999\"]\n",
        )
        .unwrap();
        let err = LintConfig::load(tmp.path().to_str().unwrap()).unwrap_err();
        assert!(
            err.contains("unknown rule"),
            "Expected unknown rule error, got: {err}"
        );
        assert!(err.contains("X999"));
    }

    #[test]
    #[serial_test::serial]
    fn error_wins_over_warn() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("agent-lint.toml"),
            "[lint]\nerror = [\"S033\"]\nwarn = [\"S033\"]\n",
        )
        .unwrap();
        let config = LintConfig::load(tmp.path().to_str().unwrap()).unwrap();
        assert!(config.error.contains(&LintRule::NameVague));
        assert!(!config.warn.contains(&LintRule::NameVague));
    }

    #[test]
    #[serial_test::serial]
    fn suppress_wins_over_error() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("agent-lint.toml"),
            "[lint]\nsuppress = [\"S033\"]\nerror = [\"S033\"]\n",
        )
        .unwrap();
        let config = LintConfig::load(tmp.path().to_str().unwrap()).unwrap();
        assert!(config.suppress.contains(&LintRule::NameVague));
        assert!(!config.error.contains(&LintRule::NameVague));
    }

    #[test]
    #[serial_test::serial]
    fn suppress_wins_over_error_and_warn() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("agent-lint.toml"),
            "[lint]\nsuppress = [\"S033\"]\nerror = [\"S033\"]\nwarn = [\"S033\"]\n",
        )
        .unwrap();
        let config = LintConfig::load(tmp.path().to_str().unwrap()).unwrap();
        assert!(config.suppress.contains(&LintRule::NameVague));
        assert!(!config.error.contains(&LintRule::NameVague));
        assert!(!config.warn.contains(&LintRule::NameVague));
    }

    #[test]
    #[serial_test::serial]
    fn missing_error_defaults_to_empty() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("agent-lint.toml"),
            "[lint]\nsuppress = [\"M001\"]\n",
        )
        .unwrap();
        let config = LintConfig::load(tmp.path().to_str().unwrap()).unwrap();
        assert!(config.error.is_empty());
    }

    // ── Exclude patterns ────────────────────────────────────────────

    #[test]
    #[serial_test::serial]
    fn exclude_parsed_from_config() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("agent-lint.toml"),
            "[lint]\nexclude = [\"docs/*.md\", \"skills/internal/**\"]\n",
        )
        .unwrap();
        let config = LintConfig::load(tmp.path().to_str().unwrap()).unwrap();
        assert_eq!(config.exclude.len(), 2);
        assert_eq!(config.exclude[0], "docs/*.md");
        assert_eq!(config.exclude[1], "skills/internal/**");
    }

    #[test]
    #[serial_test::serial]
    fn empty_exclude_is_valid() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("agent-lint.toml"), "[lint]\nexclude = []\n").unwrap();
        let config = LintConfig::load(tmp.path().to_str().unwrap()).unwrap();
        assert!(config.exclude.is_empty());
    }

    #[test]
    #[serial_test::serial]
    fn missing_exclude_defaults_to_empty() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("agent-lint.toml"),
            "[lint]\nsuppress = [\"M001\"]\n",
        )
        .unwrap();
        let config = LintConfig::load(tmp.path().to_str().unwrap()).unwrap();
        assert!(config.exclude.is_empty());
    }

    #[test]
    #[serial_test::serial]
    fn script_inventory_loads_supported_untracked_paths_deterministically() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir(tmp.path().join("scripts")).unwrap();
        for path in ["z.sh", "helper.inc.bash", "rules.awk"] {
            std::fs::write(tmp.path().join("scripts").join(path), "# fixture\n").unwrap();
        }
        std::fs::write(
            tmp.path().join("scripts/inventory.txt"),
            "# explicit scope\nscripts/z.sh\nscripts/rules.awk\nscripts/helper.inc.bash\nscripts/z.sh\n",
        )
        .unwrap();
        std::fs::write(
            tmp.path().join("agent-lint.toml"),
            "[lint]\nscript-inventory = \"scripts/inventory.txt\"\n",
        )
        .unwrap();

        let config = LintConfig::load(tmp.path().to_str().unwrap()).unwrap();
        assert_eq!(
            config.script_inventory.unwrap(),
            [
                "scripts/helper.inc.bash",
                "scripts/rules.awk",
                "scripts/z.sh"
            ]
        );
    }

    #[test]
    #[serial_test::serial]
    fn invalid_script_inventory_fails_closed() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("notes.md"), "notes\n").unwrap();
        std::fs::write(tmp.path().join("inventory.txt"), "../outside.sh\n").unwrap();
        std::fs::write(
            tmp.path().join("agent-lint.toml"),
            "[lint]\nscript-inventory = \"inventory.txt\"\n",
        )
        .unwrap();
        let traversal = LintConfig::load(tmp.path().to_str().unwrap()).unwrap_err();
        assert!(traversal.contains("safe repository-relative"));

        std::fs::write(tmp.path().join("inventory.txt"), "notes.md\n").unwrap();
        let extension = LintConfig::load(tmp.path().to_str().unwrap()).unwrap_err();
        assert!(extension.contains("must end in .sh, .inc.bash, or .awk"));

        std::fs::write(tmp.path().join("inventory.txt"), "missing.sh\n").unwrap();
        let missing = LintConfig::load(tmp.path().to_str().unwrap()).unwrap_err();
        assert!(missing.contains("cannot be read"));
    }

    #[cfg(unix)]
    #[test]
    #[serial_test::serial]
    fn script_inventory_rejects_parent_symlink_escape() {
        use std::os::unix::fs::symlink;

        let tmp = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("escaped.sh"), "printf ok\n").unwrap();
        symlink(outside.path(), tmp.path().join("linked")).unwrap();
        std::fs::write(tmp.path().join("inventory.txt"), "linked/escaped.sh\n").unwrap();
        std::fs::write(
            tmp.path().join("agent-lint.toml"),
            "[lint]\nscript-inventory = \"inventory.txt\"\n",
        )
        .unwrap();

        let error = LintConfig::load(tmp.path().to_str().unwrap()).unwrap_err();
        assert!(error.contains("resolves outside the repository root"));
    }

    #[test]
    #[serial_test::serial]
    fn invalid_exclude_pattern_returns_error() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("agent-lint.toml"),
            "[lint]\nexclude = [\"[invalid\"]\n",
        )
        .unwrap();
        let err = LintConfig::load(tmp.path().to_str().unwrap()).unwrap_err();
        assert!(
            err.contains("invalid exclude glob"),
            "Expected invalid glob error, got: {err}"
        );
    }

    #[test]
    #[serial_test::serial]
    fn exclude_not_array_returns_error() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("agent-lint.toml"),
            "[lint]\nexclude = \"not-an-array\"\n",
        )
        .unwrap();
        let err = LintConfig::load(tmp.path().to_str().unwrap()).unwrap_err();
        assert!(!err.is_empty());
    }

    #[test]
    #[serial_test::serial]
    fn contract_limits_and_path_scope_are_configurable() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("agent-lint.toml"),
            "[lint]\n\
             desc-truncated-max-chars = 200\n\
             skill-closure-max-lines = 700\n\
             claude-import-max-lines = 120\n\
             claude-import-total-max-lines = 400\n\
             instruction-files = [\"AGENTS.md\"]\n\
             inline-path-prefixes = [\"src/\", \"docs/\"]\n",
        )
        .unwrap();
        let config = LintConfig::load(tmp.path().to_str().unwrap()).unwrap();
        assert_eq!(config.desc_truncated_max_chars, 200);
        assert_eq!(config.skill_closure_max_lines, Some(700));
        assert_eq!(config.claude_import_max_lines, Some(120));
        assert_eq!(config.claude_import_total_max_lines, Some(400));
        assert_eq!(config.instruction_files, ["AGENTS.md"]);
        assert_eq!(config.inline_path_prefixes, ["src/", "docs/"]);
    }

    #[test]
    #[serial_test::serial]
    fn zero_contract_limit_is_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("agent-lint.toml"),
            "[lint]\nskill-closure-max-lines = 0\n",
        )
        .unwrap();
        let error = LintConfig::load(tmp.path().to_str().unwrap()).unwrap_err();
        assert!(error.contains("must be greater than zero"));
    }

    #[test]
    #[serial_test::serial]
    fn import_and_prompt_source_budgets_are_normalized_and_sorted() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("skills/design")).unwrap();
        for path in ["AGENTS.md", "BASH_AUTHORING.md"] {
            std::fs::write(tmp.path().join(path), "source\n").unwrap();
        }
        std::fs::write(tmp.path().join("skills/design/SKILL.md"), "root\n").unwrap();
        std::fs::write(
            tmp.path().join("agent-lint.toml"),
            r#"[lint.claude-import-path-budgets]
"./AGENTS.md" = 89
"BASH_AUTHORING.md" = 115

[[lint.prompt-source-budgets]]
name = "design"
roots = ["./skills/design/SKILL.md"]
conditional-sources = ["AGENTS.md"]
root-max-lines = 700
closure-max-tokens = 50000
conditional-max-content-tokens = 1000
"#,
        )
        .unwrap();

        let config = LintConfig::load(tmp.path().to_str().unwrap()).unwrap();

        assert_eq!(config.claude_import_path_budgets["AGENTS.md"], 89);
        assert_eq!(config.prompt_source_budgets[0].name, "design");
        assert_eq!(
            config.prompt_source_budgets[0].roots,
            ["skills/design/SKILL.md"]
        );
        assert_eq!(
            config.prompt_source_budgets[0]
                .conditional_caps
                .content_tokens,
            Some(1000)
        );
    }

    #[test]
    #[serial_test::serial]
    fn duplicate_normalized_import_budget_path_is_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("AGENTS.md"), "source\n").unwrap();
        std::fs::write(
            tmp.path().join("agent-lint.toml"),
            "[lint.claude-import-path-budgets]\n\"AGENTS.md\" = 10\n\"./AGENTS.md\" = 11\n",
        )
        .unwrap();

        let error = LintConfig::load(tmp.path().to_str().unwrap()).unwrap_err();

        assert!(error.contains("duplicate normalized path"));
    }

    #[test]
    #[serial_test::serial]
    fn malformed_prompt_source_budget_is_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("source.md"), "source\n").unwrap();
        std::fs::write(
            tmp.path().join("agent-lint.toml"),
            "[[lint.prompt-source-budgets]]\nname = \"demo\"\nroots = [\"source.md\"]\nclosure-max-lines = 10\nunknown-cap = 2\n",
        )
        .unwrap();

        let error = LintConfig::load(tmp.path().to_str().unwrap()).unwrap_err();

        assert!(error.contains("unknown field"));
    }

    #[test]
    #[serial_test::serial]
    fn prompt_budgets_support_three_skills_and_a_named_panel() {
        let tmp = tempfile::tempdir().unwrap();
        for name in ["design", "implement", "review"] {
            std::fs::create_dir_all(tmp.path().join(format!("skills/{name}"))).unwrap();
            std::fs::write(tmp.path().join(format!("skills/{name}/SKILL.md")), "root\n").unwrap();
        }
        std::fs::create_dir(tmp.path().join("agents")).unwrap();
        std::fs::write(tmp.path().join("agents/reviewer.md"), "panel\n").unwrap();
        std::fs::write(tmp.path().join("review-conditional.md"), "branch\n").unwrap();
        std::fs::write(
            tmp.path().join("agent-lint.toml"),
            r#"[[lint.prompt-source-budgets]]
name = "review"
roots = ["skills/review/SKILL.md"]
conditional-sources = ["review-conditional.md"]
closure-max-lines = 10
conditional-max-tokens = 10

[[lint.prompt-source-budgets]]
name = "implement"
roots = ["skills/implement/SKILL.md"]
root-max-content-tokens = 10

[[lint.prompt-source-budgets]]
name = "panel"
roots = ["agents/reviewer.md"]
closure-max-tokens = 10

[[lint.prompt-source-budgets]]
name = "design"
roots = ["skills/design/SKILL.md"]
root-max-lines = 10
"#,
        )
        .unwrap();

        let config = LintConfig::load(tmp.path().to_str().unwrap()).unwrap();

        assert_eq!(
            config
                .prompt_source_budgets
                .iter()
                .map(|budget| budget.name.as_str())
                .collect::<Vec<_>>(),
            ["design", "implement", "panel", "review"]
        );
        assert_eq!(
            config.prompt_source_budgets[3].conditional_sources,
            ["review-conditional.md"]
        );
    }

    #[test]
    #[serial_test::serial]
    fn unsafe_and_missing_budget_sources_are_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("agent-lint.toml"),
            "[lint.claude-import-path-budgets]\n\"../outside.md\" = 10\n",
        )
        .unwrap();
        let unsafe_error = LintConfig::load(tmp.path().to_str().unwrap()).unwrap_err();
        assert!(unsafe_error.contains("safe repository-relative path"));

        std::fs::write(
            tmp.path().join("agent-lint.toml"),
            "[lint.claude-import-path-budgets]\n\"missing.md\" = 10\n",
        )
        .unwrap();
        let missing_error = LintConfig::load(tmp.path().to_str().unwrap()).unwrap_err();
        assert!(missing_error.contains("cannot be read"));

        std::fs::write(tmp.path().join("bad.md"), [0xff, 0xfe]).unwrap();
        std::fs::write(
            tmp.path().join("agent-lint.toml"),
            "[lint.claude-import-path-budgets]\n\"bad.md\" = 10\n",
        )
        .unwrap();
        let utf8_error = LintConfig::load(tmp.path().to_str().unwrap()).unwrap_err();
        assert!(utf8_error.contains("UTF-8"));
    }

    #[test]
    #[serial_test::serial]
    fn escaping_instruction_path_is_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("agent-lint.toml"),
            "[lint]\ninstruction-files = [\"../AGENTS.md\"]\n",
        )
        .unwrap();
        let error = LintConfig::load(tmp.path().to_str().unwrap()).unwrap_err();
        assert!(error.contains("safe repository-relative path"));
    }

    // ── ExcludeSet ──────────────────────────────────────────────────

    #[test]
    fn exclude_set_empty_matches_nothing() {
        let set = ExcludeSet::default();
        assert!(!set.is_excluded("skills/foo/SKILL.md"));
        assert!(!set.is_excluded("anything"));
    }

    #[test]
    fn exclude_set_star_matches_single_level() {
        let set = ExcludeSet::new(&["docs/*.md".to_string()]).unwrap();
        assert!(set.is_excluded("docs/readme.md"));
        assert!(set.is_excluded("docs/architecture.md"));
        // * does NOT match across path separators
        assert!(!set.is_excluded("docs/sub/nested.md"));
    }

    #[test]
    fn exclude_set_double_star_matches_recursive() {
        let set = ExcludeSet::new(&["docs/**/*.md".to_string()]).unwrap();
        assert!(set.is_excluded("docs/readme.md"));
        assert!(set.is_excluded("docs/sub/nested.md"));
        assert!(set.is_excluded("docs/a/b/c.md"));
    }

    #[test]
    fn exclude_set_skill_pattern() {
        let set = ExcludeSet::new(&["skills/my-skill/**".to_string()]).unwrap();
        assert!(set.is_excluded("skills/my-skill/SKILL.md"));
        assert!(set.is_excluded("skills/my-skill/scripts/helper.sh"));
        assert!(!set.is_excluded("skills/other-skill/SKILL.md"));
    }

    #[test]
    fn exclude_set_normalizes_dot_slash() {
        let set = ExcludeSet::new(&["skills/*/SKILL.md".to_string()]).unwrap();
        // With leading ./
        assert!(set.is_excluded("./skills/my-skill/SKILL.md"));
        // Without leading ./
        assert!(set.is_excluded("skills/my-skill/SKILL.md"));
    }

    #[test]
    fn exclude_set_normalizes_backslashes() {
        let set = ExcludeSet::new(&["skills/*/SKILL.md".to_string()]).unwrap();
        assert!(set.is_excluded("skills\\my-skill\\SKILL.md"));
    }

    #[test]
    fn exclude_set_multiple_patterns() {
        let set = ExcludeSet::new(&[
            "agents/internal.md".to_string(),
            "skills/deprecated-*/**".to_string(),
        ])
        .unwrap();
        assert!(set.is_excluded("agents/internal.md"));
        assert!(set.is_excluded("skills/deprecated-old/SKILL.md"));
        assert!(!set.is_excluded("agents/general.md"));
        assert!(!set.is_excluded("skills/active/SKILL.md"));
    }

    #[test]
    fn exclude_set_exact_file() {
        let set = ExcludeSet::new(&["CLAUDE.md".to_string()]).unwrap();
        assert!(set.is_excluded("CLAUDE.md"));
        assert!(!set.is_excluded("README.md"));
    }

    #[test]
    fn exclude_set_invalid_pattern_error() {
        let result = ExcludeSet::new(&["[invalid".to_string()]);
        assert!(result.is_err());
    }

    // ── normalize_path ──────────────────────────────────────────────

    #[test]
    fn normalize_strips_dot_slash() {
        assert_eq!(
            normalize_path("./skills/foo/SKILL.md"),
            "skills/foo/SKILL.md"
        );
    }

    #[test]
    fn normalize_no_dot_slash_unchanged() {
        assert_eq!(normalize_path("skills/foo/SKILL.md"), "skills/foo/SKILL.md");
    }

    #[test]
    fn normalize_backslash_to_forward() {
        assert_eq!(
            normalize_path("skills\\foo\\SKILL.md"),
            "skills/foo/SKILL.md"
        );
    }

    #[test]
    fn normalize_mixed_separators() {
        assert_eq!(
            normalize_path(".\\skills/foo\\SKILL.md"),
            "skills/foo/SKILL.md"
        );
    }

    // ── apply_cli_mode ─────────────────────────────────────────────

    #[test]
    fn apply_normal_no_change() {
        let mut config = LintConfig {
            suppress: HashSet::from([LintRule::PluginJsonMissing]),
            error: HashSet::from([LintRule::NameVague]),
            warn: HashSet::from([LintRule::SecurityMdMissing]),
            exclude: vec![],
            ..LintConfig::default()
        };
        config.apply_cli_mode(CliMode::Normal);
        assert!(config.suppress.contains(&LintRule::PluginJsonMissing));
        assert!(config.error.contains(&LintRule::NameVague));
        assert!(config.warn.contains(&LintRule::SecurityMdMissing));
    }

    #[test]
    fn apply_pedantic_moves_warn_to_error() {
        let mut config = LintConfig {
            suppress: HashSet::new(),
            error: HashSet::new(),
            warn: HashSet::from([LintRule::SecurityMdMissing, LintRule::TodoInSkill]),
            exclude: vec![],
            ..LintConfig::default()
        };
        config.apply_cli_mode(CliMode::Pedantic);
        assert!(config.error.contains(&LintRule::SecurityMdMissing));
        assert!(config.error.contains(&LintRule::TodoInSkill));
        assert!(config.warn.is_empty());
    }

    #[test]
    fn apply_pedantic_skips_too_long() {
        let mut config = LintConfig {
            suppress: HashSet::new(),
            error: HashSet::new(),
            warn: HashSet::from([
                LintRule::SecurityMdMissing,
                LintRule::BodyTooLong,
                LintRule::CompatTooLong,
            ]),
            exclude: vec![],
            ..LintConfig::default()
        };
        config.apply_cli_mode(CliMode::Pedantic);
        // Non-too-long rule promoted to error
        assert!(config.error.contains(&LintRule::SecurityMdMissing));
        // Too-long rules remain in warn
        assert!(config.warn.contains(&LintRule::BodyTooLong));
        assert!(config.warn.contains(&LintRule::CompatTooLong));
        assert!(!config.error.contains(&LintRule::BodyTooLong));
        assert!(!config.error.contains(&LintRule::CompatTooLong));
    }

    #[test]
    fn apply_pedantic_leaves_suppress_intact() {
        let mut config = LintConfig {
            suppress: HashSet::from([LintRule::PluginJsonMissing]),
            error: HashSet::new(),
            warn: HashSet::from([LintRule::SecurityMdMissing]),
            exclude: vec![],
            ..LintConfig::default()
        };
        config.apply_cli_mode(CliMode::Pedantic);
        assert!(config.suppress.contains(&LintRule::PluginJsonMissing));
        assert!(config.error.contains(&LintRule::SecurityMdMissing));
    }

    #[test]
    fn apply_pedantic_default_error_stays_error() {
        let mut config = LintConfig {
            suppress: HashSet::new(),
            error: HashSet::new(),
            warn: HashSet::new(),
            exclude: vec![],
            ..LintConfig::default()
        };
        config.apply_cli_mode(CliMode::Pedantic);
        // Default-error rules like PluginJsonMissing aren't in the error set
        // from TOML, but fire as errors via default_severity() in report().
        // Pedantic adds default-warning rules to the error set (except too-long).
        assert!(!config.error.contains(&LintRule::PluginJsonMissing));
    }

    #[test]
    fn apply_pedantic_promotes_default_warning_to_error() {
        let mut config = LintConfig {
            suppress: HashSet::new(),
            error: HashSet::new(),
            warn: HashSet::new(),
            exclude: vec![],
            ..LintConfig::default()
        };
        config.apply_cli_mode(CliMode::Pedantic);
        // Default-warning rules are promoted to error by pedantic.
        assert!(config.error.contains(&LintRule::SecurityMdMissing));
        assert!(config.error.contains(&LintRule::TodoInSkill));
        assert!(config.error.contains(&LintRule::NameVague));
    }

    #[test]
    fn apply_pedantic_skips_default_warning_too_long() {
        let mut config = LintConfig {
            suppress: HashSet::new(),
            error: HashSet::new(),
            warn: HashSet::new(),
            exclude: vec![],
            ..LintConfig::default()
        };
        config.apply_cli_mode(CliMode::Pedantic);
        // CompatTooLong is a default-warning too-long rule; stays as warning.
        assert!(!config.error.contains(&LintRule::CompatTooLong));
        // BodyTooLong is default-suppressed, so pedantic never considers it.
        assert!(!config.error.contains(&LintRule::BodyTooLong));
    }

    #[test]
    fn apply_pedantic_respects_suppress_for_default_warning() {
        let mut config = LintConfig {
            suppress: HashSet::from([LintRule::NameVague]),
            error: HashSet::new(),
            warn: HashSet::new(),
            exclude: vec![],
            ..LintConfig::default()
        };
        config.apply_cli_mode(CliMode::Pedantic);
        // Suppressed default-warning rules are not promoted.
        assert!(!config.error.contains(&LintRule::NameVague));
        assert!(config.suppress.contains(&LintRule::NameVague));
    }

    #[test]
    fn apply_pedantic_leaves_suppressed_alone() {
        let mut config = LintConfig {
            suppress: HashSet::new(),
            error: HashSet::new(),
            warn: HashSet::new(),
            exclude: vec![],
            ..LintConfig::default()
        };
        config.apply_cli_mode(CliMode::Pedantic);
        // NameNotGerund is default-suppressed, should not be promoted.
        assert!(!config.error.contains(&LintRule::NameNotGerund));
    }

    #[test]
    fn apply_all_enables_everything() {
        let mut config = LintConfig {
            suppress: HashSet::from([LintRule::PluginJsonMissing]),
            error: HashSet::new(),
            warn: HashSet::from([LintRule::SecurityMdMissing]),
            exclude: vec!["docs/*.md".to_string()],
            ..LintConfig::default()
        };
        config.apply_cli_mode(CliMode::All);
        assert!(config.suppress.is_empty());
        assert!(config.warn.is_empty());
        assert_eq!(config.error.len(), 286);
        // Exclude is NOT cleared — it's about file paths, not rule severity
        assert_eq!(config.exclude.len(), 1);
    }

    #[test]
    fn apply_all_overrides_suppress() {
        let mut config = LintConfig {
            suppress: HashSet::from([LintRule::PluginJsonMissing, LintRule::NameVague]),
            error: HashSet::new(),
            warn: HashSet::new(),
            exclude: vec![],
            ..LintConfig::default()
        };
        config.apply_cli_mode(CliMode::All);
        assert!(config.suppress.is_empty());
        assert!(config.error.contains(&LintRule::PluginJsonMissing));
        assert!(config.error.contains(&LintRule::NameVague));
    }

    #[test]
    fn apply_all_includes_too_long_rules() {
        let mut config = LintConfig::default();
        config.apply_cli_mode(CliMode::All);
        assert!(config.error.contains(&LintRule::NameTooLong));
        assert!(config.error.contains(&LintRule::DescTooLong));
        assert!(config.error.contains(&LintRule::BodyTooLong));
        assert!(config.error.contains(&LintRule::CompatTooLong));
    }
}
