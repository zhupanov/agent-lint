use crate::config::{ExcludeSet, LintConfig};
use crate::context::LintContext;
use crate::context::LintMode;
use crate::fence::CodeFenceTracker;
use crate::frontmatter;
use crate::hook_commands::extract_hook_command_paths;
use crate::platforms::ValidationTargets;
use crate::pwd_hygiene::replace_bundled_asset_prefixes;
use crate::rules::{FixKind, LintRule};
use crate::script_paths::Invocation;
use crate::traversal;
use crate::validators::skill_content::security::flagged_http_offsets;
use crate::validators::skill_content::{
    RE_BACKSLASH_PATH, S043_PROSE_FIELDS, contains_backslash_path, is_named_tex_escape_pair,
};
use crate::validators::skills::{collect_plugin_skill_files, collect_skills};
use regex::Regex;
use std::fs;
use std::io::Write;
use std::path::Path;
use std::sync::LazyLock;

// Reuse the same regexes validators use.
// This replacement-scoped pattern consumes every segment in a detected path
// run, so an autofix cannot leave mixed separators behind.
static RE_BACKSLASH_PATH_RUN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"[A-Za-z]:(?:\\[A-Za-z0-9_.-]+)+|\\\\[A-Za-z][A-Za-z0-9_-]*(?:\\[A-Za-z][A-Za-z0-9_-]*)+|(?:\\[A-Za-z][A-Za-z0-9_-]*){2,}",
    )
    .unwrap()
});
static RE_NAME_INVALID: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[^a-z0-9-]").unwrap());

/// Attempt to fix all instances of a given auto-fixable rule.
/// Returns `true` if at least one file was modified.
pub fn apply_fix(
    rule: LintRule,
    mode: LintMode,
    targets: ValidationTargets,
    exclude: &ExcludeSet,
    config: &LintConfig,
) -> bool {
    let Some(kind) = rule.fix_kind() else {
        return false;
    };
    match kind {
        FixKind::HookExecutable => fix_executability_hooks(mode, config),
        FixKind::ScriptExecutable => fix_executability_scripts(mode, exclude, config),
        FixKind::FrontmatterName => fix_frontmatter_name_mismatch(mode, targets, exclude, config),
        FixKind::EmptyFrontmatterField => fix_frontmatter_field_empty(mode, exclude, config),
        FixKind::DescriptionXml => fix_desc_has_xml(mode, exclude, config),
        FixKind::ConsecutiveBash => fix_consecutive_bash(mode, exclude, config),
        FixKind::BodyBackslashPath => fix_backslash_path(mode, exclude, config),
        FixKind::NonHttpsUrl => fix_non_https_url(mode, exclude, config),
        FixKind::FrontmatterBackslashPath => fix_frontmatter_backslash(mode, exclude, config),
        FixKind::PwdAssetPrefix => fix_pwd_in_skill(exclude, config),
    }
}

fn is_suppressed(config: &LintConfig, rule: LintRule, path: impl AsRef<Path>) -> bool {
    config.is_suppressed_at(rule, path.as_ref())
}

fn log_fix(rule: LintRule, msg: &str) {
    let msg = crate::diagnostic::sanitize_for_terminal(msg);
    let _ = writeln!(
        std::io::stderr(),
        "fixed[{}/{}]: {msg}",
        rule.code(),
        rule.name()
    );
}

// ── H005: chmod +x on hook scripts ──────────────────────────────────────

#[cfg(unix)]
fn fix_executability_hooks(mode: LintMode, config: &LintConfig) -> bool {
    use crate::context::{LintContext, ManifestState};

    let ctx = LintContext::new(Path::new("."), mode);
    let mut fixed = false;

    if mode == LintMode::Plugin
        && let ManifestState::Parsed(value) = &ctx.hooks_json
    {
        fixed |= fix_hook_config_executability(value, config);
    }
    if let ManifestState::Parsed(value) = &ctx.settings_json {
        fixed |= fix_hook_config_executability(value, config);
    }
    if let ManifestState::Parsed(value) = &ctx.settings_local_json {
        fixed |= fix_hook_config_executability(value, config);
    }
    for hook_config in &ctx.declared_hook_configs {
        if let ManifestState::Parsed(value) = &hook_config.state {
            fixed |= fix_hook_config_executability(value, config);
        }
    }
    fixed
}

#[cfg(unix)]
fn fix_hook_config_executability(value: &serde_json::Value, config: &LintConfig) -> bool {
    let mut fixed = false;
    for reference in extract_hook_command_paths(value, None) {
        if reference.invocation != Invocation::Direct || reference.path.as_os_str().is_empty() {
            continue;
        }
        if is_suppressed(config, LintRule::HookNotExecutable, &reference.path) {
            continue;
        }
        if make_hook_executable(&reference.path) {
            log_fix(
                LintRule::HookNotExecutable,
                &format!("made executable: {}", reference.path.display()),
            );
            fixed = true;
        }
    }
    fixed
}

#[cfg(unix)]
fn make_hook_executable(path: &Path) -> bool {
    let Ok(repo_root) = std::env::current_dir().and_then(std::fs::canonicalize) else {
        return false;
    };
    let Ok(resolved) = std::fs::canonicalize(path) else {
        return false;
    };
    if !resolved.starts_with(repo_root) {
        return false;
    }
    make_executable(resolved.to_str().unwrap_or(""))
}

#[cfg(not(unix))]
fn fix_executability_hooks(_mode: LintMode, _config: &LintConfig) -> bool {
    false
}

// ── G003: chmod +x on script files ──────────────────────────────────────

#[cfg(unix)]
fn fix_executability_scripts(mode: LintMode, exclude: &ExcludeSet, config: &LintConfig) -> bool {
    let ctx = LintContext::new(Path::new("."), mode);
    let mut fixed = false;
    for path in crate::validators::hygiene::scripts::direct_script_paths(&ctx, exclude) {
        let display = path.display().to_string();
        if is_suppressed(config, LintRule::ScriptNotExecutable, &path) {
            continue;
        }
        if make_executable(path.to_str().unwrap_or("")) {
            log_fix(
                LintRule::ScriptNotExecutable,
                &format!("made executable: {display}"),
            );
            fixed = true;
        }
    }
    fixed
}

#[cfg(not(unix))]
fn fix_executability_scripts(_mode: LintMode, _exclude: &ExcludeSet, _config: &LintConfig) -> bool {
    false
}

#[cfg(unix)]
fn make_executable(path: &str) -> bool {
    use std::os::unix::fs::PermissionsExt;
    let p = Path::new(path);
    if !p.is_file() {
        return false;
    }
    let meta = match p.metadata() {
        Ok(m) => m,
        Err(_) => return false,
    };
    let mode = meta.permissions().mode();
    if mode & 0o111 != 0 {
        return false; // Already executable
    }
    let new_mode = mode | ((mode & 0o444) >> 2);
    fs::set_permissions(p, std::os::unix::fs::PermissionsExt::from_mode(new_mode)).is_ok()
}

// ── S006: frontmatter name mismatch ─────────────────────────────────────

fn fix_frontmatter_name_mismatch(
    mode: LintMode,
    targets: ValidationTargets,
    exclude: &ExcludeSet,
    config: &LintConfig,
) -> bool {
    let ctx = LintContext::new(Path::new("."), mode);
    let mut fixed = false;
    let discovery = crate::validators::skill_discovery::SkillDiscovery::from_context(&ctx, exclude);
    let mut paths = discovery.private_skill_files;
    if mode == LintMode::Plugin {
        paths.extend(discovery.exported_skill_files);
    }
    if targets.agent_skills {
        paths.extend(
            crate::platforms::agent_skill_candidates(exclude)
                .into_iter()
                .map(|entry| entry.path),
        );
    }
    paths.sort();
    paths.dedup();

    for skill_md in paths {
        let Some(path) = skill_md.parent() else {
            continue;
        };
        let dir_name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };
        // Root fallback has no directory-name contract, and the discovery
        // inventory has already excluded exported documentation trees.
        if path == Path::new(".") {
            continue;
        }
        let skill_path = skill_md.to_string_lossy().replace('\\', "/");
        if is_suppressed(config, LintRule::FrontmatterNameMismatch, &skill_path) {
            continue;
        }

        // Validate dir_name against naming rules before using it.
        if RE_NAME_INVALID.is_match(&dir_name)
            || dir_name.starts_with('-')
            || dir_name.ends_with('-')
            || dir_name.contains("--")
        {
            continue;
        }

        let content = match fs::read_to_string(&skill_md) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let fm_lines = match frontmatter::extract_frontmatter(&content) {
            Some(lines) => lines,
            None => continue,
        };
        let parsed_frontmatter = match frontmatter::parse_yaml_strict(&fm_lines) {
            Ok(yaml) => yaml,
            Err(_) => continue,
        };
        let name = match frontmatter::canonical_nonempty_string_field(&parsed_frontmatter, "name") {
            Some(name) => name,
            None => continue,
        };
        if name == dir_name {
            continue;
        }

        let Some(raw_name_index) = single_line_frontmatter_field_index(&fm_lines, "name") else {
            continue;
        };
        let raw_name_line = &fm_lines[raw_name_index];
        if raw_name_line["name:".len()..].trim().is_empty() {
            continue;
        }
        let new_line = format!("name: {dir_name}");
        if let Some(new_content) = replace_in_frontmatter(&content, raw_name_line, &new_line) {
            if fs::write(&skill_md, new_content).is_ok() {
                log_fix(
                    LintRule::FrontmatterNameMismatch,
                    &format!("{skill_path}: renamed '{name}' to '{dir_name}'"),
                );
                fixed = true;
            }
        }
    }
    fixed
}

// ── S007: empty frontmatter field ───────────────────────────────────────

fn fix_frontmatter_field_empty(mode: LintMode, exclude: &ExcludeSet, config: &LintConfig) -> bool {
    let ctx = LintContext::new(Path::new("."), mode);
    let mut fixed = false;
    let discovery = crate::validators::skill_discovery::SkillDiscovery::from_context(&ctx, exclude);
    let mut paths = discovery.private_skill_files;
    if mode == LintMode::Plugin {
        paths.extend(discovery.exported_skill_files);
    }
    paths.sort();
    paths.dedup();
    for skill_path in paths {
        let skill_md = skill_path.to_string_lossy().replace('\\', "/");
        if is_suppressed(config, LintRule::FrontmatterFieldEmpty, &skill_md) {
            continue;
        }
        let content = match fs::read_to_string(&skill_path) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let Some(fm_lines) = frontmatter::extract_frontmatter(&content) else {
            continue;
        };
        let body = frontmatter::extract_body(&content);

        for field in crate::validators::skill_content::OPTIONAL_NONEMPTY_SCALAR_FIELDS {
            let parsed_frontmatter = frontmatter::parse_yaml_strict(&fm_lines).ok();
            if !frontmatter::optional_field_is_present(
                &fm_lines,
                parsed_frontmatter.as_ref(),
                field,
            ) {
                continue;
            }
            if !frontmatter::optional_field_is_empty(&fm_lines, parsed_frontmatter.as_ref(), field)
            {
                continue; // Not empty
            }

            // FINDING_8: skip removing argument-hint if body uses $ARGUMENTS
            if *field == "argument-hint" && body.contains("$ARGUMENTS") {
                continue;
            }

            // A bare field line with no indented continuation is the only
            // unambiguous removal. In particular, never orphan a YAML
            // continuation or child block.
            let Some(index) = single_line_frontmatter_field_index(&fm_lines, field) else {
                continue;
            };
            let prefix = format!("{field}:");
            if fm_lines[index].trim_end() != prefix {
                continue;
            }

            if let Some(new_content) = remove_frontmatter_line(&content, &prefix) {
                if fs::write(&skill_path, &new_content).is_ok() {
                    log_fix(
                        LintRule::FrontmatterFieldEmpty,
                        &format!("{skill_md}: removed empty '{field}'"),
                    );
                    fixed = true;
                    break; // One fix per file, re-validate
                }
            }
        }
    }
    fixed
}

/// Return a top-level frontmatter field line only when no indented YAML lines
/// belong to that field before the next top-level entry.
fn single_line_frontmatter_field_index(fm_lines: &[String], field: &str) -> Option<usize> {
    let index = frontmatter::simple_top_level_key_index(fm_lines, field)?;
    let has_continuation = fm_lines[index + 1..]
        .iter()
        .take_while(|line| line.is_empty() || line.starts_with(' ') || line.starts_with('\t'))
        .any(|line| line.starts_with(' ') || line.starts_with('\t'));
    (!has_continuation).then_some(index)
}

// ── S018: XML tags in description ───────────────────────────────────────

fn fix_desc_has_xml(mode: LintMode, exclude: &ExcludeSet, config: &LintConfig) -> bool {
    let mut fixed = false;
    let ctx = LintContext::new(Path::new("."), mode);
    let discovery = crate::validators::skill_discovery::SkillDiscovery::from_context(&ctx, exclude);
    let mut paths = discovery.private_skill_files;
    if mode == LintMode::Plugin {
        paths.extend(discovery.exported_skill_files);
    }
    for info in collect_plugin_skill_files(paths, exclude) {
        let display = info.path.clone();
        if is_suppressed(config, LintRule::DescHasXml, &display) {
            continue;
        }
        let value = match frontmatter::get_strict_string_field(&info.fm_lines, "description") {
            Some(v) => v,
            None => continue,
        };
        if !crate::validators::skill_content::description_contains_xml_tags(&value) {
            continue;
        }
        let new_value = crate::validators::skill_content::strip_description_xml_tags(&value);
        let new_value = new_value.trim().to_string();
        if new_value == value || new_value.is_empty() {
            continue;
        }

        let skill_path = Path::new(&display);
        let content = match fs::read_to_string(skill_path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        // A multiline or quoted scalar cannot be rewritten safely with a
        // line replacement. Only rewrite a single raw value that is
        // exactly the canonical parsed scalar.
        let prefix = "description:";
        let Some(raw_index) = info
            .fm_lines
            .iter()
            .position(|line| line.starts_with(prefix))
        else {
            continue;
        };
        let raw_line = &info.fm_lines[raw_index];
        let Some(raw_value) = raw_line.strip_prefix(prefix) else {
            continue;
        };
        let has_continuation = info.fm_lines[raw_index + 1..]
            .first()
            .is_some_and(|line| line.is_empty() || line.starts_with(char::is_whitespace));
        if has_continuation || raw_value.trim_start() != value {
            continue;
        }

        let new_line = format!("description: {new_value}");
        if let Some(new_content) = replace_in_frontmatter(&content, raw_line, &new_line) {
            if fs::write(skill_path, new_content).is_ok() {
                log_fix(
                    LintRule::DescHasXml,
                    &format!("{display}: stripped XML tags from description"),
                );
                fixed = true;
            }
        }
    }
    fixed
}

// ── S021: consecutive bash code blocks ──────────────────────────────────

fn fix_consecutive_bash(mode: LintMode, exclude: &ExcludeSet, config: &LintConfig) -> bool {
    let base_dirs: &[&str] = match mode {
        LintMode::Plugin => &["skills", ".claude/skills"],
        LintMode::Basic => &[".claude/skills"],
    };
    for base_dir in base_dirs {
        let skills = collect_skills(base_dir, exclude);
        for info in &skills {
            let skill_path = Path::new(base_dir).join(&info.dir_name).join("SKILL.md");
            if is_suppressed(config, LintRule::ConsecutiveBash, &skill_path) {
                continue;
            }
            let content = match fs::read_to_string(&skill_path) {
                Ok(c) => c,
                Err(_) => continue,
            };
            // The S021 body validator scans `info.body`; merge in that same
            // coordinate space so autofix touches exactly its flagged pairs,
            // then splice the rewritten body back onto the original preamble.
            let Some(preamble) = content.strip_suffix(info.body.as_str()) else {
                continue;
            };
            let Some(new_body) = merge_first_consecutive_bash(&info.body) else {
                continue;
            };
            let new_content = format!("{preamble}{new_body}");
            if fs::write(&skill_path, &new_content).is_ok() {
                log_fix(
                    LintRule::ConsecutiveBash,
                    &format!(
                        "{base_dir}/{}/SKILL.md: merged consecutive bash blocks",
                        info.dir_name
                    ),
                );
                return true; // One merged pair per apply_fix call; driver re-validates
            }
        }
    }
    fix_reference_consecutive_bash(mode, exclude, config)
}

/// Merge reference-file (`references/*.md`) S021 pairs, using the same scope as
/// `validate_reference_consecutive_bash`. One merged pair per call.
fn fix_reference_consecutive_bash(
    mode: LintMode,
    exclude: &ExcludeSet,
    config: &LintConfig,
) -> bool {
    let include_public = mode == LintMode::Plugin;
    for path in crate::validators::contracts::reference_bash_markdown_paths(include_public, exclude)
    {
        // Mirror the validator's `read_text` guard, then honor per-file
        // suppression before mutating (I-Fix-1).
        if path.is_symlink() || exclude.is_excluded(&path.to_string_lossy()) {
            continue;
        }
        if is_suppressed(config, LintRule::ConsecutiveBash, &path) {
            continue;
        }
        let content = match fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let Some(new_content) = merge_first_consecutive_bash(&content) else {
            continue;
        };
        if fs::write(&path, &new_content).is_ok() {
            log_fix(
                LintRule::ConsecutiveBash,
                &format!("{}: merged consecutive bash blocks", path.display()),
            );
            return true; // One merged pair per apply_fix call; driver re-validates
        }
    }
    false
}

/// Merge the first flagged consecutive-bash pair whose gap is only blank lines,
/// returning the rewritten content. Candidate pairs come from the shared policy
/// host `fence::consecutive_bash_pairs`, so waived, WRONG/CORRECT, design-driver,
/// and non-`bash` (`sh`/`shell`/bare) pairs the validator never flags are never
/// touched. A flagged pair whose gap holds prose breadcrumbs or HTML comments is
/// left for a human (deleting gap content is not a purely mechanical fix), so the
/// diagnostic survives and the autofix loop can terminate.
///
/// `content` is the exact string the validator scans — a SKILL.md body or a whole
/// reference file — and the result is in that same coordinate space.
fn merge_first_consecutive_bash(content: &str) -> Option<String> {
    let pairs = crate::fence::consecutive_bash_pairs(content);
    if pairs.is_empty() {
        return None;
    }
    let fences = crate::fence::markdown_fences(content);
    // Terminator-preserving segments keep each line's own `\n`/`\r\n`, so a merge
    // rewrites neither line endings nor the trailing-newline state. `split_inclusive`
    // is index-aligned with the 1-based `str::lines()` numbering the fence policy
    // uses, so line `N` is `segments[N - 1]`.
    let segments: Vec<&str> = content.split_inclusive('\n').collect();
    for (first_start, second_start) in pairs {
        // `consecutive_bash_pairs` yields (first opener, second opener) line
        // numbers; recover the first fence to find where its closer sits.
        let Some(first) = fences.iter().find(|fence| fence.start_line == first_start) else {
            continue;
        };
        // Gap between the first closer and the second opener, matching the slice
        // `consecutive_bash_pairs` itself inspects.
        let gap = &segments[first.end_line..second_start - 1];
        if !gap.iter().all(|line| line.trim().is_empty()) {
            continue; // Prose/HTML-comment gap: leave the diagnostic for a human.
        }
        // Drop the first block's closing fence, the blank gap, and the second
        // block's opening fence (1-based lines -> 0-based inclusive indices).
        let remove_start = first.end_line - 1;
        let remove_end = second_start - 1;
        let mut result = String::with_capacity(content.len());
        for (index, segment) in segments.iter().enumerate() {
            if index < remove_start || index > remove_end {
                result.push_str(segment);
            }
        }
        return Some(result);
    }
    None
}

// ── S022: backslash paths in body ───────────────────────────────────────

fn fix_backslash_path(mode: LintMode, exclude: &ExcludeSet, config: &LintConfig) -> bool {
    let mut fixed = false;
    let base_dirs: &[&str] = match mode {
        LintMode::Plugin => &["skills", ".claude/skills"],
        LintMode::Basic => &[".claude/skills"],
    };
    for base_dir in base_dirs {
        let skills = collect_skills(base_dir, exclude);
        for info in &skills {
            let skill_path = Path::new(base_dir).join(&info.dir_name).join("SKILL.md");
            if is_suppressed(config, LintRule::BackslashPath, &skill_path) {
                continue;
            }
            // Check outside code fences (matching validator)
            let has_backslash =
                crate::fence::lines_outside_fences(&info.body).any(contains_backslash_path);
            if !has_backslash {
                continue;
            }

            let content = match fs::read_to_string(&skill_path) {
                Ok(c) => c,
                Err(_) => continue,
            };

            // Replace backslash paths only in lines outside code fences
            let body = frontmatter::extract_body(&content);
            let fm_end = content.len() - body.len();
            let preamble = &content[..fm_end];

            let mut new_body = String::new();
            let mut tracker = CodeFenceTracker::new();
            for line in body.lines() {
                let class = tracker.process_line(line);
                if class == crate::fence::LineClass::Outside && contains_backslash_path(line) {
                    // Only replace backslashes within matched path patterns, not all backslashes
                    new_body.push_str(&replace_backslash_paths(line));
                } else {
                    new_body.push_str(line);
                }
                new_body.push('\n');
            }
            // Fix trailing newline
            if !body.ends_with('\n') && new_body.ends_with('\n') {
                new_body.pop();
            }

            let new_content = format!("{preamble}{new_body}");
            if new_content != content && fs::write(&skill_path, &new_content).is_ok() {
                log_fix(
                    LintRule::BackslashPath,
                    &format!(
                        "{base_dir}/{}/SKILL.md: replaced backslash paths",
                        info.dir_name
                    ),
                );
                fixed = true;
            }
        }
    }
    fixed
}

// ── S031: non-HTTPS URLs ────────────────────────────────────────────────

fn fix_non_https_url(mode: LintMode, exclude: &ExcludeSet, config: &LintConfig) -> bool {
    let mut fixed = false;
    let base_dirs: &[&str] = match mode {
        LintMode::Plugin => &["skills", ".claude/skills"],
        LintMode::Basic => &[".claude/skills"],
    };
    for base_dir in base_dirs {
        let skills = collect_skills(base_dir, exclude);
        for info in &skills {
            let skill_path = Path::new(base_dir).join(&info.dir_name).join("SKILL.md");
            if is_suppressed(config, LintRule::NonHttpsUrl, &skill_path) {
                continue;
            }
            if info.body.trim().is_empty() {
                continue;
            }
            // Only rewrite if the shared classifier would flag a match, so the
            // autofix and the S031 checker agree by construction (issue #353).
            if flagged_http_offsets(&info.body).next().is_none() {
                continue;
            }

            let content = match fs::read_to_string(&skill_path) {
                Ok(c) => c,
                Err(_) => continue,
            };

            // Replace http:// with https:// only in the body (matching validator scope)
            let body = frontmatter::extract_body(&content);
            let fm_end = content.len() - body.len();
            let preamble = &content[..fm_end];
            let new_body = replace_http_urls(body);
            let new_content = format!("{preamble}{new_body}");
            if new_content != content && fs::write(&skill_path, &new_content).is_ok() {
                log_fix(
                    LintRule::NonHttpsUrl,
                    &format!(
                        "{base_dir}/{}/SKILL.md: replaced http:// with https://",
                        info.dir_name
                    ),
                );
                fixed = true;
            }
        }
    }
    fixed
}

fn replace_http_urls(content: &str) -> String {
    let mut result = content.to_string();
    // The shared classifier decides which matches are flagged; exempt
    // identifier/reserved-host matches are left byte-identical. Rewrite in
    // reverse so earlier offsets stay valid as later ones grow by one byte.
    let offsets: Vec<usize> = flagged_http_offsets(content).collect();
    for start in offsets.into_iter().rev() {
        result = format!(
            "{}https://{}",
            &result[..start],
            &result[start + "http://".len()..]
        );
    }
    result
}

// ── S043: backslash paths in frontmatter ────────────────────────────────

fn fix_frontmatter_backslash(mode: LintMode, exclude: &ExcludeSet, config: &LintConfig) -> bool {
    let mut fixed = false;
    let base_dirs: &[&str] = match mode {
        LintMode::Plugin => &["skills", ".claude/skills"],
        LintMode::Basic => &[".claude/skills"],
    };
    for base_dir in base_dirs {
        let skills = collect_skills(base_dir, exclude);
        for info in &skills {
            let skill_path = Path::new(base_dir).join(&info.dir_name).join("SKILL.md");
            if is_suppressed(config, LintRule::FrontmatterBackslash, &skill_path) {
                continue;
            }
            // Read canonical values so prose and metadata are never rewritten
            // and quoting cannot corrupt the value. Invalid YAML is owned by
            // X001 and has nothing safe to rewrite here.
            let Some(map) = info.frontmatter_mapping() else {
                continue;
            };

            let content = match fs::read_to_string(&skill_path) {
                Ok(c) => c,
                Err(_) => continue,
            };

            // The autofix only rewrites a single-line scalar whose raw value is
            // exactly the canonical scalar; sequences, multiline, and quoted
            // values leave the diagnostic standing (item 9 / #309 / #318).
            let mut new_content = content.clone();
            let mut changed = false;
            for (key, value) in map.iter() {
                if S043_PROSE_FIELDS.contains(&key.as_str()) {
                    continue;
                }
                let Some(scalar) = value.as_str() else {
                    continue;
                };
                if !contains_backslash_path(scalar) {
                    continue;
                }
                let Some(raw_line) = single_line_scalar_raw_line(&info.fm_lines, key, scalar)
                else {
                    continue;
                };
                let new_line = replace_backslash_paths(&raw_line);
                if new_line == raw_line {
                    continue;
                }
                if let Some(updated) = replace_in_frontmatter(&new_content, &raw_line, &new_line) {
                    new_content = updated;
                    changed = true;
                }
            }

            if changed && fs::write(&skill_path, &new_content).is_ok() {
                log_fix(
                    LintRule::FrontmatterBackslash,
                    &format!(
                        "{base_dir}/{}/SKILL.md: replaced backslash paths in frontmatter",
                        info.dir_name
                    ),
                );
                fixed = true;
            }
        }
    }
    fixed
}

/// The raw frontmatter line for a simple top-level scalar `field` when it is
/// single-line-safe to rewrite: the key is unindented, has no continuation, and
/// its raw value equals the canonical scalar. Returns the full raw line.
fn single_line_scalar_raw_line(
    fm_lines: &[String],
    field: &str,
    canonical: &str,
) -> Option<String> {
    let index = single_line_frontmatter_field_index(fm_lines, field)?;
    let raw_line = &fm_lines[index];
    let raw_value = raw_line.strip_prefix(&format!("{field}:"))?;
    (raw_value.trim_start() == canonical).then(|| raw_line.clone())
}

// ── G001: $PWD in skill content ─────────────────────────────────────────

fn fix_pwd_in_skill(exclude: &ExcludeSet, config: &LintConfig) -> bool {
    let skills_dir = Path::new("skills");
    if !skills_dir.is_dir() {
        return false;
    }
    let mut fixed = false;
    for entry in traversal::shallow_directories(skills_dir, Path::new("."), None).entries {
        let path = entry.path;
        let name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };
        if name == "shared" {
            continue;
        }
        let skill_path = format!("skills/{name}/SKILL.md");
        if exclude.is_excluded(&skill_path)
            || is_suppressed(config, LintRule::PwdInSkill, &skill_path)
        {
            continue;
        }
        let skill_md = path.join("SKILL.md");
        if !skill_md.is_file() {
            continue;
        }
        let content = match fs::read_to_string(&skill_md) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let Some(new_content) = replace_bundled_asset_prefixes(&content) else {
            continue;
        };

        if new_content != content && fs::write(&skill_md, &new_content).is_ok() {
            log_fix(
                LintRule::PwdInSkill,
                &format!(
                    "{skill_path}: replaced bundled-asset PWD prefix with ${{CLAUDE_PLUGIN_ROOT}}/"
                ),
            );
            fixed = true;
        }
    }
    fixed
}

// ── String helpers ──────────────────────────────────────────────────────

/// Replace only backslash path patterns on a line, leaving other backslashes intact.
fn replace_backslash_paths(line: &str) -> String {
    if !RE_BACKSLASH_PATH.is_match(line) || !contains_backslash_path(line) {
        return line.to_string();
    }

    RE_BACKSLASH_PATH_RUN
        .replace_all(line, |caps: &regex::Captures| {
            if is_named_tex_escape_pair(&caps[0]) {
                caps[0].to_string()
            } else {
                caps[0].replace('\\', "/")
            }
        })
        .to_string()
}

// ── Frontmatter helpers ─────────────────────────────────────────────────

/// Replace an exact line in the frontmatter section of a file.
fn replace_in_frontmatter(content: &str, old_line: &str, new_line: &str) -> Option<String> {
    let mut result = String::new();
    let mut in_fm = false;
    let mut fm_delim_count = 0;
    let mut replaced = false;

    for line in content.lines() {
        if line == "---" {
            fm_delim_count += 1;
            if fm_delim_count == 1 {
                in_fm = true;
            } else if fm_delim_count == 2 {
                in_fm = false;
            }
            result.push_str(line);
        } else if in_fm && !replaced && line.trim() == old_line.trim() {
            result.push_str(new_line);
            replaced = true;
        } else {
            result.push_str(line);
        }
        result.push('\n');
    }
    if !content.ends_with('\n') && result.ends_with('\n') {
        result.pop();
    }
    if replaced { Some(result) } else { None }
}

/// Remove the first line matching a prefix from the frontmatter section.
fn remove_frontmatter_line(content: &str, line_prefix: &str) -> Option<String> {
    let mut result = String::new();
    let mut in_fm = false;
    let mut fm_delim_count = 0;
    let mut removed = false;

    for line in content.lines() {
        if line == "---" {
            fm_delim_count += 1;
            if fm_delim_count == 1 {
                in_fm = true;
            } else if fm_delim_count == 2 {
                in_fm = false;
            }
            result.push_str(line);
            result.push('\n');
        } else if in_fm && !removed && line.starts_with(line_prefix) {
            removed = true;
            // Don't add this line
        } else {
            result.push_str(line);
            result.push('\n');
        }
    }
    if !content.ends_with('\n') && result.ends_with('\n') {
        result.pop();
    }
    if removed { Some(result) } else { None }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    #[serial_test::serial]
    fn fix_executability_hooks_includes_settings_local() {
        use std::os::unix::fs::PermissionsExt;

        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all(".claude").unwrap();
        std::fs::create_dir_all("scripts").unwrap();
        std::fs::write("scripts/local-hook.sh", "#!/usr/bin/env bash\n").unwrap();
        std::fs::set_permissions(
            "scripts/local-hook.sh",
            std::fs::Permissions::from_mode(0o644),
        )
        .unwrap();
        std::fs::write(
            ".claude/settings.local.json",
            r#"{"hooks":[{"command":"${CLAUDE_PLUGIN_ROOT}/scripts/local-hook.sh"}]}"#,
        )
        .unwrap();

        assert!(fix_executability_hooks(
            LintMode::Plugin,
            &LintConfig::default()
        ));
        assert_ne!(
            std::fs::metadata("scripts/local-hook.sh")
                .unwrap()
                .permissions()
                .mode()
                & 0o111,
            0
        );
    }

    #[cfg(unix)]
    #[test]
    #[serial_test::serial]
    fn fix_executability_hooks_in_basic_mode() {
        use std::os::unix::fs::PermissionsExt;

        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all(".claude/hooks").unwrap();
        std::fs::write(".claude/hooks/check.py", "#!/usr/bin/env python3\n").unwrap();
        std::fs::set_permissions(
            ".claude/hooks/check.py",
            std::fs::Permissions::from_mode(0o644),
        )
        .unwrap();
        std::fs::write(
            ".claude/settings.json",
            r#"{"hooks":[{"command":"\"${CLAUDE_PROJECT_DIR}\"/.claude/hooks/check.py"}]}"#,
        )
        .unwrap();

        assert!(fix_executability_hooks(
            LintMode::Basic,
            &LintConfig::default()
        ));
        assert!(!fix_executability_hooks(
            LintMode::Basic,
            &LintConfig::default()
        ));
        assert_ne!(
            std::fs::metadata(".claude/hooks/check.py")
                .unwrap()
                .permissions()
                .mode()
                & 0o111,
            0
        );
    }

    #[cfg(unix)]
    #[test]
    #[serial_test::serial]
    fn fix_executability_hooks_does_not_follow_external_symlink() {
        use std::os::unix::fs::{PermissionsExt, symlink};

        let tmp = tempfile::tempdir().unwrap();
        let external = tempfile::NamedTempFile::new().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all(".claude").unwrap();
        std::fs::set_permissions(external.path(), std::fs::Permissions::from_mode(0o644)).unwrap();
        symlink(external.path(), ".claude/external-hook").unwrap();
        std::fs::write(
            ".claude/settings.json",
            r#"{"hooks":[{"command":"$PWD/.claude/external-hook"}]}"#,
        )
        .unwrap();

        assert!(!fix_executability_hooks(
            LintMode::Basic,
            &LintConfig::default()
        ));
        assert_eq!(
            std::fs::metadata(external.path())
                .unwrap()
                .permissions()
                .mode()
                & 0o111,
            0
        );
    }

    #[cfg(unix)]
    #[test]
    #[serial_test::serial]
    fn fix_executability_hooks_includes_manifest_declared_config() {
        use std::os::unix::fs::PermissionsExt;

        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all(".claude-plugin").unwrap();
        std::fs::create_dir_all("config").unwrap();
        std::fs::create_dir_all("scripts").unwrap();
        std::fs::write("scripts/declared-hook.sh", "#!/usr/bin/env bash\n").unwrap();
        std::fs::set_permissions(
            "scripts/declared-hook.sh",
            std::fs::Permissions::from_mode(0o644),
        )
        .unwrap();
        std::fs::write(
            ".claude-plugin/plugin.json",
            r#"{"name":"declared-hooks","hooks":"./config/hooks.json"}"#,
        )
        .unwrap();
        std::fs::write(
            "config/hooks.json",
            r#"{"hooks":{"PreToolUse":[{"hooks":[{"type":"command","command":"${CLAUDE_PLUGIN_ROOT}/scripts/declared-hook.sh"}]}]}}"#,
        )
        .unwrap();

        assert!(fix_executability_hooks(
            LintMode::Plugin,
            &LintConfig::default()
        ));
        assert_ne!(
            std::fs::metadata("scripts/declared-hook.sh")
                .unwrap()
                .permissions()
                .mode()
                & 0o111,
            0
        );
    }

    #[test]
    fn replace_in_frontmatter_basic() {
        let content = "---\nname: old\ndescription: test\n---\nbody\n";
        let result = replace_in_frontmatter(content, "name: old", "name: new").unwrap();
        assert!(result.contains("name: new"));
        assert!(!result.contains("name: old"));
    }

    #[test]
    fn replace_in_frontmatter_no_match() {
        let content = "---\nname: foo\n---\nbody\n";
        assert!(replace_in_frontmatter(content, "name: bar", "name: baz").is_none());
    }

    #[test]
    fn remove_frontmatter_line_basic() {
        let content = "---\nname: foo\nargument-hint:\n---\nbody\n";
        let result = remove_frontmatter_line(content, "argument-hint:").unwrap();
        assert!(!result.contains("argument-hint"));
        assert!(result.contains("name: foo"));
    }

    #[test]
    #[serial_test::serial]
    fn fix_frontmatter_field_empty_removes_a_bare_field_via_invalid_yaml_fallback() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all(".claude/skills/empty").unwrap();
        // A duplicate `name` key makes the frontmatter genuinely invalid YAML,
        // so S007's autofix must use its line-oriented fallback to find and
        // remove the bare `argument-hint:` field. (A bare trailing null key is
        // valid YAML and is now handled by the canonical path instead.)
        std::fs::write(
            ".claude/skills/empty/SKILL.md",
            "---\nname: empty\ndescription: A valid description\nargument-hint:\nname: duplicate\n---\nBody\n",
        )
        .unwrap();

        let skills = collect_skills(".claude/skills", &ExcludeSet::default());
        assert_eq!(skills.len(), 1);
        assert!(frontmatter::parse_yaml_strict(&skills[0].fm_lines).is_err());
        assert_eq!(
            single_line_frontmatter_field_index(&skills[0].fm_lines, "argument-hint"),
            Some(2)
        );

        assert!(fix_frontmatter_field_empty(
            LintMode::Basic,
            &ExcludeSet::default(),
            &LintConfig::default()
        ));
        assert!(
            !std::fs::read_to_string(".claude/skills/empty/SKILL.md")
                .unwrap()
                .contains("argument-hint:")
        );
    }

    // ── S043 autofix: single-line-safe, prose-exempt, idempotent ─────

    #[test]
    #[serial_test::serial]
    fn fix_frontmatter_backslash_rewrites_single_line_scalar_and_is_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/demo").unwrap();
        std::fs::write(
            "skills/demo/SKILL.md",
            "---\nname: demo\ndescription: A valid skill description here\nargument-hint: C:\\Users\\me\\file\n---\nBody\n",
        )
        .unwrap();

        assert!(fix_frontmatter_backslash(
            LintMode::Plugin,
            &ExcludeSet::default(),
            &LintConfig::default()
        ));
        let after = std::fs::read_to_string("skills/demo/SKILL.md").unwrap();
        assert!(
            after.contains("argument-hint: C:/Users/me/file"),
            "got: {after}"
        );
        assert!(!after.contains('\\'));

        // Second pass makes no change.
        assert!(!fix_frontmatter_backslash(
            LintMode::Plugin,
            &ExcludeSet::default(),
            &LintConfig::default()
        ));
        assert_eq!(
            std::fs::read_to_string("skills/demo/SKILL.md").unwrap(),
            after
        );
    }

    #[test]
    #[serial_test::serial]
    fn fix_frontmatter_backslash_leaves_sequence_and_prose_byte_identical() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/demo").unwrap();
        // A prose description (exempt) and a sequence item (fires S043 but is not
        // single-line-scalar) both carry a backslash path. Neither is rewritten.
        let original = "---\nname: demo\ndescription: See C:\\Users\\me\\notes\npaths:\n  - C:\\Users\\a\n---\nBody\n";
        std::fs::write("skills/demo/SKILL.md", original).unwrap();

        assert!(!fix_frontmatter_backslash(
            LintMode::Plugin,
            &ExcludeSet::default(),
            &LintConfig::default()
        ));
        assert_eq!(
            std::fs::read_to_string("skills/demo/SKILL.md").unwrap(),
            original
        );
    }

    #[test]
    #[serial_test::serial]
    fn fix_frontmatter_backslash_leaves_quoted_scalar_standing() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/demo").unwrap();
        // A double-quoted value's raw text differs from its canonical scalar, so
        // the single-line-safety rule declines to rewrite it.
        let original = "---\nname: demo\ndescription: A valid skill description here\nargument-hint: \"C:\\\\Users\\\\x\"\n---\nBody\n";
        std::fs::write("skills/demo/SKILL.md", original).unwrap();

        assert!(!fix_frontmatter_backslash(
            LintMode::Plugin,
            &ExcludeSet::default(),
            &LintConfig::default()
        ));
        assert_eq!(
            std::fs::read_to_string("skills/demo/SKILL.md").unwrap(),
            original
        );
    }

    #[test]
    fn merge_consecutive_bash_basic() {
        let content = "---\nname: test\n---\n```bash\necho a\n```\n\n```bash\necho b\n```\n";
        let result = merge_first_consecutive_bash(content).unwrap();
        // Should have only one bash block
        assert_eq!(result.matches("```bash").count(), 1);
        assert!(result.contains("echo a"));
        assert!(result.contains("echo b"));
    }

    #[test]
    fn merge_consecutive_bash_no_consecutive() {
        let content =
            "---\nname: test\n---\n```bash\necho a\n```\nsome text\n```bash\necho b\n```\n";
        assert!(merge_first_consecutive_bash(content).is_none());
    }

    #[test]
    fn merge_bash_skips_pairs_the_shared_policy_never_flags() {
        // Reason-bearing waiver in the first fence: deliberate tool boundary.
        let waived = "```bash\n# lint-consecutive-bash: ok separate tool boundary needed\necho one\n```\n\n```bash\necho two\n```\n";
        assert!(crate::fence::consecutive_bash_pairs(waived).is_empty());
        assert!(merge_first_consecutive_bash(waived).is_none());

        // Non-`bash` info strings are never S021, even with a blank-only gap.
        for info in ["sh", "shell", ""] {
            let content = format!("```{info}\necho one\n```\n\n```{info}\necho two\n```\n");
            assert!(merge_first_consecutive_bash(&content).is_none(), "{info:?}");
        }

        // WRONG/CORRECT example pair (blank gap) is carved out by context.
        let example = "WRONG then CORRECT:\n```bash\necho one\n```\n\n```bash\necho two\n```\n";
        assert!(crate::fence::consecutive_bash_pairs(example).is_empty());
        assert!(merge_first_consecutive_bash(example).is_none());

        // Design-driver pause/resume pair (blank gap) is carved out.
        let design = "```bash\npython3 python/cli.py design driver --action pause\n```\n\n```bash\npython3 python/cli.py design driver --action resume\n```\n";
        assert!(crate::fence::consecutive_bash_pairs(design).is_empty());
        assert!(merge_first_consecutive_bash(design).is_none());
    }

    #[test]
    fn merge_bash_leaves_flagged_breadcrumb_gap_for_a_human() {
        // A short breadcrumb does not create a tool boundary, so the pair is
        // flagged — but the gap is not blank, so a mechanical merge would delete
        // author content. Autofix must decline and leave the diagnostic.
        let breadcrumb = "```bash\necho one\n```\nThen continue:\n```bash\necho two\n```\n";
        assert_eq!(crate::fence::consecutive_bash_pairs(breadcrumb), [(1, 5)]);
        assert!(merge_first_consecutive_bash(breadcrumb).is_none());
    }

    #[test]
    fn merge_bash_merges_genuine_blank_gap_pair_and_is_idempotent() {
        let genuine = "```bash\necho one\n```\n\n```bash\necho two\n```\n";
        let merged = merge_first_consecutive_bash(genuine).unwrap();
        assert_eq!(merged, "```bash\necho one\necho two\n```\n");
        // The rewritten content re-lints clean and a second pass is a no-op.
        assert!(crate::fence::consecutive_bash_pairs(&merged).is_empty());
        assert!(merge_first_consecutive_bash(&merged).is_none());

        // Directly adjacent fences (empty gap) merge too.
        let adjacent = "```bash\necho one\n```\n```bash\necho two\n```\n";
        assert_eq!(
            merge_first_consecutive_bash(adjacent).unwrap(),
            "```bash\necho one\necho two\n```\n"
        );
    }

    #[test]
    fn merge_bash_preserves_absent_trailing_newline() {
        let no_newline = "```bash\necho one\n```\n\n```bash\necho two\n```";
        let merged = merge_first_consecutive_bash(no_newline).unwrap();
        assert_eq!(merged, "```bash\necho one\necho two\n```");
        assert!(!merged.ends_with('\n'));
    }

    #[test]
    fn merge_bash_preserves_crlf_line_endings() {
        // Every retained line keeps its own terminator, so a CRLF file stays
        // CRLF instead of degrading to mixed or LF-only endings.
        let crlf = "```bash\r\necho one\r\n```\r\n\r\n```bash\r\necho two\r\n```\r\n";
        let merged = merge_first_consecutive_bash(crlf).unwrap();
        assert_eq!(merged, "```bash\r\necho one\r\necho two\r\n```\r\n");
        assert!(!merged.contains("\r\r") && !merged.contains("\n\n"));
    }

    #[test]
    #[serial_test::serial]
    fn fix_consecutive_bash_merges_reference_file_blank_gap_pair() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/demo/references").unwrap();
        std::fs::write(
            "skills/demo/references/guide.md",
            "# Guide\n\n```bash\necho one\n```\n\n```bash\necho two\n```\n",
        )
        .unwrap();

        assert!(fix_consecutive_bash(
            LintMode::Plugin,
            &ExcludeSet::default(),
            &LintConfig::default()
        ));
        assert_eq!(
            std::fs::read_to_string("skills/demo/references/guide.md").unwrap(),
            "# Guide\n\n```bash\necho one\necho two\n```\n"
        );
        // No flagged pair remains, so a second pass makes no change.
        assert!(!fix_consecutive_bash(
            LintMode::Plugin,
            &ExcludeSet::default(),
            &LintConfig::default()
        ));
    }

    #[test]
    #[serial_test::serial]
    fn fix_consecutive_bash_honors_reference_file_suppression() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all("skills/demo/references").unwrap();
        let original = "# Guide\n\n```bash\necho one\n```\n\n```bash\necho two\n```\n";
        std::fs::write("skills/demo/references/guide.md", original).unwrap();

        std::fs::write(
            "agent-lint.toml",
            "[lint]\n[[lint.overrides]]\nfiles = [\"skills/demo/references/guide.md\"]\nsuppress = [\"S021\"]\n",
        )
        .unwrap();
        let config = LintConfig::load(Path::new(".")).unwrap();
        assert!(!fix_consecutive_bash(
            LintMode::Plugin,
            &ExcludeSet::default(),
            &config
        ));
        assert_eq!(
            std::fs::read_to_string("skills/demo/references/guide.md").unwrap(),
            original
        );
    }

    #[test]
    fn replace_http_urls_basic() {
        let content = "Visit http://api.foo.dev for details";
        let result = replace_http_urls(content);
        assert_eq!(result, "Visit https://api.foo.dev for details");
    }

    #[test]
    fn replace_http_urls_excludes_localhost() {
        let content = "Use http://localhost:3000 for dev";
        let result = replace_http_urls(content);
        assert_eq!(result, content); // No change
    }

    #[test]
    fn replace_http_urls_excludes_example_com() {
        let content = "See http://example.com/docs for reference";
        let result = replace_http_urls(content);
        assert_eq!(result, content);
    }

    #[test]
    fn s022_imports_the_validator_detection_regex() {
        assert!(std::ptr::eq(
            &*RE_BACKSLASH_PATH,
            &*crate::validators::skill_content::RE_BACKSLASH_PATH,
        ));
    }

    #[test]
    fn replace_backslash_paths_converts_full_runs_only() {
        assert_eq!(
            replace_backslash_paths(r"Open C:\Users\name and \dir\file\last; keep \n."),
            "Open C:/Users/name and /dir/file/last; keep \\n."
        );
        assert_eq!(
            replace_backslash_paths(r"Use \alpha\beta and \\server\share."),
            r"Use \alpha\beta and //server/share."
        );
    }

    #[test]
    fn replace_http_urls_keeps_xml_identifier_but_rewrites_link() {
        // Evidence 1 (issue #353): the XML namespace identifier is untouched
        // while a genuine plain-HTTP link on another line is upgraded.
        let content = "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 24 24\">\nDownload from http://api.foo.dev/asset\n";
        let once = replace_http_urls(content);
        assert!(
            once.contains("xmlns=\"http://www.w3.org/2000/svg\""),
            "identifier changed: {once}"
        );
        assert!(
            once.contains("https://api.foo.dev/asset"),
            "link not upgraded: {once}"
        );
        // Idempotent: a second pass changes nothing.
        assert_eq!(replace_http_urls(&once), once);
    }

    #[test]
    fn replace_http_urls_excludes_reserved_name_hosts() {
        for content in [
            "See http://www.example.com/guide for the walkthrough",
            "Try http://foo.test/x locally",
            "Placeholder http://demo.invalid/ endpoint",
        ] {
            assert_eq!(replace_http_urls(content), content, "content: {content}");
        }
    }
}
