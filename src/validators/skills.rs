use crate::config::ExcludeSet;
use crate::context::LintContext;
use crate::diagnostic::{DiagnosticCollector, DiagnosticMetadata};
use crate::frontmatter;
use crate::live_instructions::{InstructionSurfaceKind, LiveInstructionDocument};
use crate::markdown::MarkdownDocument;
use crate::rules::LintRule;
use crate::traversal;
use crate::validators::shared_md_refs;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
const SKILL_DIR_SIZE_LIMIT: u64 = 8 * 1024 * 1024;

/// Pre-parsed data for a single SKILL.md file.
#[allow(dead_code)]
pub struct SkillInfo {
    /// Display path, e.g. "skills/my-skill/SKILL.md"
    pub path: String,
    /// Directory name, e.g. "my-skill"
    pub dir_name: String,
    /// Frontmatter lines (between the --- delimiters).
    pub fm_lines: Vec<String>,
    /// Canonical frontmatter parsed once with `parse_yaml_strict`. `None` when
    /// the frontmatter is not valid YAML. Field-type rules that read canonical
    /// values consume this instead of the legacy line-oriented helpers; invalid
    /// YAML is owned by X001 (and missing/non-string required fields by S005).
    pub(crate) parsed_frontmatter: Option<crate::yaml::Value>,
    /// Body content after the frontmatter closing delimiter.
    pub body: String,
    /// Shared Markdown facts for this file. Content validators must consume
    /// this rather than parse the body again.
    pub document: MarkdownDocument,
    /// Whether the skill directory contains a non-empty `scripts/` subdirectory.
    pub has_scripts_dir: bool,
}

impl SkillInfo {
    /// Canonical top-level frontmatter mapping, or `None` when the frontmatter
    /// is invalid YAML or its document is not a mapping. Field-type rules that
    /// read canonical values skip when this is `None`; those invalid states are
    /// owned by X001 (parse failure) and S004/S005 (structure/required fields).
    pub(crate) fn frontmatter_mapping(&self) -> Option<&crate::yaml::Mapping> {
        self.parsed_frontmatter.as_ref()?.as_mapping()
    }
}

/// Walk a skills directory and collect SkillInfo for each valid skill.
/// Skips `shared/` subdirectory and excluded paths. Returns empty vec if dir doesn't exist.
pub fn collect_skills(base_dir: &str, exclude: &ExcludeSet) -> Vec<SkillInfo> {
    let dir = Path::new(base_dir);
    let subdirs = traversal::shallow_directories(dir, Path::new("."), None)
        .entries
        .into_iter()
        .filter_map(|entry| {
            let name = entry.path.file_name()?.to_str()?.to_string();
            (name != "shared" && !exclude.is_excluded(&format!("{base_dir}/{name}/SKILL.md")))
                .then_some((entry.path, name))
        })
        .collect::<Vec<_>>();

    collect_skill_files(
        subdirs
            .into_iter()
            .map(|(path, dir_name)| traversal::WalkEntry {
                path: path.join("SKILL.md"),
                display: format!("{base_dir}/{dir_name}/SKILL.md"),
            }),
    )
}

/// Collect the shared Cursor runtime skill inventory. This is intentionally
/// separate from the Claude/plugin shallow layout: Cursor recognizes nested
/// `.cursor/skills/` and `.agents/skills/` roots throughout a repository.
pub(crate) fn collect_cursor_runtime_skills(exclude: &ExcludeSet) -> Vec<SkillInfo> {
    collect_skill_files(crate::platforms::cursor_runtime_skill_candidates(exclude))
}

/// Collect shared Agent Skills roots throughout a repository.
pub(crate) fn collect_agent_skills(exclude: &ExcludeSet) -> Vec<SkillInfo> {
    collect_skill_files(crate::platforms::agent_skill_candidates(exclude))
}

pub(crate) fn collect_plugin_skill_files(
    paths: Vec<PathBuf>,
    exclude: &ExcludeSet,
) -> Vec<SkillInfo> {
    let entries = paths
        .into_iter()
        .filter(|path| !exclude.is_excluded(&path.to_string_lossy()))
        .map(|path| traversal::WalkEntry {
            display: path.to_string_lossy().replace('\\', "/"),
            path,
        });
    collect_skill_files(entries)
}

pub(crate) fn collect_skills_including_shared(
    base_dir: &str,
    exclude: &ExcludeSet,
) -> Vec<SkillInfo> {
    let entries = traversal::shallow_directories(Path::new(base_dir), Path::new("."), None)
        .entries
        .into_iter()
        .map(|entry| traversal::WalkEntry {
            display: format!(
                "{base_dir}/{}/SKILL.md",
                entry.path.file_name().unwrap().to_string_lossy()
            ),
            path: entry.path.join("SKILL.md"),
        })
        .filter(|entry| !exclude.is_excluded(&entry.display));
    collect_skill_files(entries)
}

fn collect_skill_files(entries: impl IntoIterator<Item = traversal::WalkEntry>) -> Vec<SkillInfo> {
    let mut skills = Vec::new();
    for entry in entries {
        let dir_name = entry
            .path
            .parent()
            .and_then(|parent| parent.file_name())
            .and_then(|name| name.to_str())
            .map(str::to_owned)
            .unwrap_or_default();
        let content = match fs::read_to_string(&entry.path) {
            Ok(content) => content,
            Err(_) => continue,
        };
        let document = MarkdownDocument::parse(content);
        let Some(fm_lines) = document.frontmatter().map(|lines| lines.to_vec()) else {
            continue; // S004 fires from existing validators where applicable.
        };
        let root_skill = entry
            .path
            .parent()
            .is_some_and(|parent| parent.as_os_str().is_empty());
        // Parse the frontmatter once here so field-type validators read
        // canonical YAML values. Invalid YAML stays `None` (X001 owns it).
        let parsed_frontmatter = frontmatter::parse_yaml_strict(&fm_lines).ok();
        let scripts_dir = entry
            .path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or(Path::new("."))
            .join("scripts");
        let has_scripts_dir = !root_skill
            && !traversal::shallow_entries(&scripts_dir, Path::new("."), None)
                .entries
                .is_empty();
        skills.push(SkillInfo {
            path: entry.display,
            dir_name,
            fm_lines,
            parsed_frontmatter,
            body: document.body().to_string(),
            document,
            has_scripts_dir,
        });
    }
    skills
}

/// V5: Validate skills/* layout — every skills/*/ (except shared/) must contain SKILL.md.
#[cfg(test)]
pub fn validate_skills_layout(diag: &mut DiagnosticCollector, exclude: &ExcludeSet) {
    let skills_dir = Path::new("skills");
    if !skills_dir.is_dir() {
        return;
    }

    let mut skill_count = 0;
    let mut excluded_count = 0;
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
        if exclude.is_excluded(&skill_path) {
            excluded_count += 1;
            continue;
        }
        let skill_md = path.join("SKILL.md");
        if !skill_md.is_file() {
            diag.report_at(
                LintRule::SkillMdMissing,
                &skill_path,
                &format!("skills/{name}/ missing SKILL.md"),
            );
            continue;
        }
        skill_count += 1;
    }

    if skill_count == 0 && excluded_count == 0 {
        diag.report_at(
            LintRule::NoExportedSkills,
            skills_dir,
            "no plugin-exported skills found under skills/",
        );
    }
}

pub fn validate_discovered_skills_layout(
    ctx: &LintContext,
    diag: &mut DiagnosticCollector,
    exclude: &ExcludeSet,
) {
    let discovery = super::skill_discovery::SkillDiscovery::from_context(ctx, exclude);
    let mut excluded = 0;
    let mut dirs = vec![PathBuf::from("skills")];
    dirs.extend(discovery.declared_skill_dirs.clone());
    dirs.sort();
    dirs.dedup();
    for dir in dirs {
        let base = dir.to_string_lossy().replace('\\', "/");
        for entry in traversal::shallow_directories(&dir, Path::new("."), None).entries {
            let Some(name) = entry.path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            if name == "shared" {
                continue;
            }
            let subject = format!("{base}/{name}/SKILL.md");
            if exclude.is_excluded(&subject) {
                excluded += 1;
                continue;
            }
            if !entry.path.join("SKILL.md").is_file() {
                diag.report_at(
                    LintRule::SkillMdMissing,
                    &subject,
                    &format!("{base}/{name}/ missing SKILL.md"),
                );
            }
        }
    }
    if Path::new("skills").is_dir()
        && discovery.exported_skill_files.is_empty()
        && discovery
            .active_command_files
            .iter()
            .all(|path| path.starts_with(".claude"))
        && !discovery.has_excluded_plugin_command
        && excluded == 0
    {
        diag.report_at(
            LintRule::NoExportedSkills,
            "skills",
            "no plugin-exported skills found",
        );
    }
}

/// V6: Validate SKILL.md frontmatter for public skills (skills/*/SKILL.md).
pub fn validate_skill_frontmatter(diag: &mut DiagnosticCollector, exclude: &ExcludeSet) {
    validate_skill_frontmatter_in_dir("skills", true, false, diag, exclude, None);
}

pub fn validate_discovered_skill_frontmatter(
    ctx: &LintContext,
    diag: &mut DiagnosticCollector,
    exclude: &ExcludeSet,
) {
    let discovery = super::skill_discovery::SkillDiscovery::from_context(ctx, exclude);
    for dir in discovery.declared_skill_dirs {
        if let Some(dir) = dir.to_str() {
            validate_skill_frontmatter_in_dir(dir, true, false, diag, exclude, None);
        }
    }
    if discovery
        .exported_skill_files
        .iter()
        .any(|path| path == Path::new("SKILL.md"))
    {
        validate_root_skill_frontmatter(diag, exclude);
    }
}

fn validate_root_skill_frontmatter(diag: &mut DiagnosticCollector, exclude: &ExcludeSet) {
    if exclude.is_excluded("SKILL.md") {
        return;
    }
    let Ok(content) = fs::read_to_string("SKILL.md") else {
        return;
    };
    let bom_before_delimiter = has_utf8_bom_before_opening_delimiter(&content);
    let document = MarkdownDocument::parse(content);
    let Some(lines) = document.frontmatter() else {
        let mut message = "SKILL.md: malformed frontmatter (must start with '---' on line 1, must have closing '---')".to_string();
        if bom_before_delimiter {
            message.push_str(": file starts with a UTF-8 byte-order mark; remove it");
        }
        diag.report_at_with(
            LintRule::FrontmatterMalformed,
            "SKILL.md",
            &message,
            DiagnosticMetadata::at_line(1),
        );
        super::markdown_structure::check_markdown_document("SKILL.md", &document, diag);
        return;
    };
    let parsed = match frontmatter::parse_yaml_strict(lines) {
        Ok(value) => {
            if let Some(hooks) = value.get("hooks") {
                diag.with_subject_path("SKILL.md", |diag| {
                    super::hook_schema::validate_frontmatter_hooks(
                        hooks,
                        "SKILL.md frontmatter",
                        diag,
                    );
                });
            }
            Some(value)
        }
        Err(error) => {
            diag.report_at_with(
                LintRule::FrontmatterYamlInvalid,
                "SKILL.md",
                &format!(
                    "SKILL.md:{}: frontmatter is not valid YAML: {}",
                    error.file_line, error.message
                ),
                DiagnosticMetadata::at_line(error.file_line),
            );
            None
        }
    };
    super::markdown_structure::check_markdown_document("SKILL.md", &document, diag);
    for field in ["name", "description"] {
        if !parsed.as_ref().is_some_and(|value| {
            frontmatter::canonical_nonempty_string_field(value, field).is_some()
        }) {
            diag.report_at_with(
                LintRule::FrontmatterFieldMissing,
                "SKILL.md",
                &format!("SKILL.md: required frontmatter field '{field}' is missing or not a non-empty string"),
                s005_location(lines, parsed.as_ref(), field),
            );
        }
    }
    for field in super::skill_content::OPTIONAL_NONEMPTY_SCALAR_FIELDS {
        if frontmatter::optional_field_is_present(lines, parsed.as_ref(), field)
            && frontmatter::optional_field_is_empty(lines, parsed.as_ref(), field)
        {
            diag.report_at_with(
                LintRule::FrontmatterFieldEmpty,
                "SKILL.md",
                &format!("SKILL.md: optional field '{field}' is present but empty"),
                frontmatter::simple_top_level_key_line(lines, field)
                    .map_or_else(DiagnosticMetadata::default, DiagnosticMetadata::at_line),
            );
        }
    }
}

/// V6-adapted: Validate SKILL.md frontmatter for private skills (.claude/skills/*/SKILL.md).
pub fn validate_private_skill_frontmatter(diag: &mut DiagnosticCollector, exclude: &ExcludeSet) {
    validate_skill_frontmatter_in_dir(".claude/skills", true, false, diag, exclude, None);
}

/// Validate frontmatter and prompt content for cross-client skills in
/// `.agents/skills/`.
pub(crate) fn validate_agent_skill_frontmatter_with_prompt_pass(
    diag: &mut DiagnosticCollector,
    exclude: &ExcludeSet,
    prompt_pass: &mut super::prompt_content::PromptContentPass,
) {
    validate_skill_frontmatter_in_dir(
        ".agents/skills",
        true,
        true,
        diag,
        exclude,
        Some(prompt_pass),
    );
}

fn validate_skill_frontmatter_in_dir(
    base_dir: &str,
    check_name_match: bool,
    platform_neutral: bool,
    diag: &mut DiagnosticCollector,
    exclude: &ExcludeSet,
    mut prompt_pass: Option<&mut super::prompt_content::PromptContentPass>,
) {
    let skill_files = if platform_neutral && base_dir == ".agents/skills" {
        crate::platforms::agent_skill_candidates(exclude)
            .into_iter()
            .map(|entry| (entry.path, entry.display))
            .collect::<Vec<_>>()
    } else {
        let dir = Path::new(base_dir);
        if !dir.is_dir() {
            return;
        }
        traversal::shallow_directories(dir, Path::new("."), None)
            .entries
            .into_iter()
            .map(|entry| {
                let skill_md = entry.path.join("SKILL.md");
                let skill_path = format!(
                    "{base_dir}/{}/SKILL.md",
                    entry.path.file_name().unwrap().to_string_lossy()
                );
                (skill_md, skill_path)
            })
            .collect()
    };

    for (skill_md, skill_path) in skill_files {
        let Some(path) = skill_md.parent() else {
            continue;
        };
        let dir_name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };
        // `skills/shared` is plugin documentation rather than a runnable
        // skill. `.agents/skills/shared`, however, is a valid shared-agent
        // skill and must remain eligible for prompt analysis.
        if dir_name == "shared" && !platform_neutral && base_dir != ".claude/skills" {
            continue;
        }

        if !skill_md.is_file() {
            continue;
        }

        if exclude.is_excluded(&skill_path) {
            continue;
        }
        let content = match fs::read_to_string(&skill_md) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let bom_before_delimiter = has_utf8_bom_before_opening_delimiter(&content);
        let document = MarkdownDocument::parse(content);
        if let Some(prompt_pass) = prompt_pass.as_deref_mut()
            && let Some(prompt_markdown) =
                MarkdownDocument::parse_for_prompt_content(document.content())
        {
            let prompt_document = LiveInstructionDocument::new(
                Path::new(&skill_path),
                InstructionSurfaceKind::Skill,
                &prompt_markdown,
            );
            prompt_pass.validate(&prompt_document, diag);
        }

        let fm_lines = match document.frontmatter() {
            Some(lines) => lines,
            None => {
                let mut message = format!(
                    "{skill_path}: malformed frontmatter (must start with '---' on line 1, must have closing '---')"
                );
                if bom_before_delimiter {
                    message.push_str(": file starts with a UTF-8 byte-order mark; remove it");
                }
                diag.report_at_with(
                    LintRule::FrontmatterMalformed,
                    &skill_path,
                    &message,
                    DiagnosticMetadata::at_line(1),
                );
                // X002–X005 still apply to the markdown file when frontmatter is broken.
                super::markdown_structure::check_markdown_document(&skill_path, &document, diag);
                continue;
            }
        };

        // X001: strict YAML parse; CC-SK-010: hooks schema when present.
        let parsed_frontmatter = match frontmatter::parse_yaml_strict(fm_lines) {
            Ok(yaml) => {
                if !platform_neutral && let Some(hooks) = yaml.get("hooks") {
                    diag.with_subject_path(&skill_path, |diag| {
                        super::hook_schema::validate_frontmatter_hooks(
                            hooks,
                            &format!("{skill_path} frontmatter"),
                            diag,
                        );
                    });
                }
                Some(yaml)
            }
            Err(err) => {
                let metadata = match err.column {
                    Some(column) => DiagnosticMetadata::at_point(err.file_line, column),
                    None => DiagnosticMetadata::at_line(err.file_line),
                };
                diag.report_at_with(
                    LintRule::FrontmatterYamlInvalid,
                    &skill_path,
                    &format!(
                        "{skill_path}:{}: frontmatter is not valid YAML: {}",
                        err.file_line, err.message
                    ),
                    metadata,
                );
                None
            }
        };

        // X002–X005: fence / XML structure on the full file.
        super::markdown_structure::check_markdown_document(&skill_path, &document, diag);

        if !platform_neutral {
            // S072: skill directory size limit.
            check_skill_dir_size(path, &skill_path, diag);

            // S073: relative .md refs nested deeper than one level.
            check_skill_ref_depth(&skill_path, &document, diag);
        }

        let raw_name = frontmatter::get_field(fm_lines, "name");
        let raw_desc = frontmatter::get_field(fm_lines, "description");
        let canonical_name = parsed_frontmatter
            .as_ref()
            .and_then(|yaml| frontmatter::canonical_nonempty_string_field(yaml, "name"));
        let canonical_desc = parsed_frontmatter
            .as_ref()
            .and_then(|yaml| frontmatter::canonical_nonempty_string_field(yaml, "description"));
        // Invalid YAML is already reported by X001. For a valid document,
        // require canonical non-empty string scalars rather than treating YAML
        // syntax as a field value.
        let name_is_valid = parsed_frontmatter
            .as_ref()
            .map_or_else(|| raw_name.is_some(), |_| canonical_name.is_some());
        let desc_is_valid = parsed_frontmatter
            .as_ref()
            .map_or_else(|| raw_desc.is_some(), |_| canonical_desc.is_some());

        if !name_is_valid {
            let metadata = s005_location(fm_lines, parsed_frontmatter.as_ref(), "name");
            diag.report_at_with(
                LintRule::FrontmatterFieldMissing,
                &skill_path,
                &format!(
                    "{skill_path}: required frontmatter field 'name' is missing or not a non-empty string"
                ),
                metadata,
            );
        }
        if !desc_is_valid {
            let metadata = s005_location(fm_lines, parsed_frontmatter.as_ref(), "description");
            diag.report_at_with(
                LintRule::FrontmatterFieldMissing,
                &skill_path,
                &format!(
                    "{skill_path}: required frontmatter field 'description' is missing or not a non-empty string"
                ),
                metadata,
            );
        }

        if check_name_match {
            if let Some(n) = canonical_name {
                if n != dir_name {
                    let metadata = frontmatter::simple_top_level_key_line(fm_lines, "name")
                        .map_or_else(DiagnosticMetadata::default, DiagnosticMetadata::at_line);
                    diag.report_at_with(
                        LintRule::FrontmatterNameMismatch,
                        &skill_path,
                        &format!(
                            "{skill_path}: frontmatter name '{n}' does not match directory '{dir_name}'"
                        ),
                        metadata,
                    );
                }
            }
        }

        // Optional scalar fields: if present, must be non-empty.
        // List lives next to KNOWN_SKILL_FRONTMATTER_FIELDS in skill_content.
        if platform_neutral {
            continue;
        }
        for field in super::skill_content::OPTIONAL_NONEMPTY_SCALAR_FIELDS {
            let field_present = frontmatter::optional_field_is_present(
                fm_lines,
                parsed_frontmatter.as_ref(),
                field,
            );
            if field_present {
                let is_empty = frontmatter::optional_field_is_empty(
                    fm_lines,
                    parsed_frontmatter.as_ref(),
                    field,
                );
                if is_empty {
                    // For allowed-tools: a following YAML list means the field
                    // is a documented list form, not empty. Canonical parses
                    // already treat a sequence as non-empty; this guard keeps
                    // the invalid-YAML line-oriented fallback consistent.
                    if *field == "allowed-tools" {
                        let has_list_items = fm_lines
                            .iter()
                            .position(|l| l.starts_with("allowed-tools:"))
                            .is_some_and(|i| {
                                fm_lines[i + 1..]
                                    .iter()
                                    .take_while(|l| {
                                        l.is_empty()
                                            || l.starts_with(' ')
                                            || l.starts_with('\t')
                                            || l.starts_with("- ")
                                    })
                                    .any(|l| l.trim_start().starts_with("- "))
                            });
                        if has_list_items {
                            continue; // a list-form value is not empty
                        }
                    }
                    diag.report_at_with(
                        LintRule::FrontmatterFieldEmpty,
                        &skill_path,
                        &format!("{skill_path}: optional field '{field}' is present but empty"),
                        frontmatter::simple_top_level_key_line(fm_lines, field)
                            .map_or_else(DiagnosticMetadata::default, DiagnosticMetadata::at_line),
                    );
                }
            }
        }
    }
}

fn check_skill_dir_size(dir: &Path, skill_path: &str, diag: &mut DiagnosticCollector) {
    let total = traversal::directory_byte_size(dir);
    if total > SKILL_DIR_SIZE_LIMIT {
        diag.report_at(
            LintRule::SkillDirOversized,
            skill_path,
            &format!(
                "{skill_path}: skill directory exceeds 8MB platform upload limit ({total} bytes)"
            ),
        );
    }
}

fn check_skill_ref_depth(
    skill_path: &str,
    document: &MarkdownDocument,
    diag: &mut DiagnosticCollector,
) {
    for link in document.links() {
        let raw_target = link.destination.as_str();
        if raw_target.starts_with('/') || raw_target.contains("${CLAUDE_PLUGIN_ROOT}") {
            continue;
        }
        let target = strip_link_fragment_or_query(raw_target);
        if has_uri_scheme(target) {
            continue;
        }
        if !target.to_ascii_lowercase().ends_with(".md") {
            continue;
        }
        let depth = target
            .split('/')
            .filter(|p| !p.is_empty() && *p != ".")
            .count();
        // One nesting level = dir/file.md (2 components). Deeper is flagged.
        // `..` components count toward depth (a parent hop leaves the skill root).
        if depth > 2 {
            diag.report_at_with(
                LintRule::SkillRefNested,
                skill_path,
                &format!(
                    "{skill_path}: skill-relative .md link '{raw_target}' is nested deeper than one level"
                ),
                DiagnosticMetadata::at_line(link.line),
            );
        }
    }
}

/// Strip a trailing `#fragment` or `?query` (whichever appears first).
fn strip_link_fragment_or_query(target: &str) -> &str {
    match target.find(['#', '?']) {
        Some(idx) => &target[..idx],
        None => target,
    }
}

/// True when `target` begins with a URI scheme (`^[A-Za-z][A-Za-z0-9+.-]*:`).
fn has_uri_scheme(target: &str) -> bool {
    let Some(colon) = target.find(':') else {
        return false;
    };
    let scheme = &target[..colon];
    let mut chars = scheme.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() => {
            chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '.' | '-'))
        }
        _ => false,
    }
}

/// V15: Validate shared markdown reference integrity.
/// Every `$CLAUDE_PLUGIN_ROOT/skills/shared/**/*.md` path referenced from
/// `skills/*/SKILL.md` must exist on disk.
#[cfg(test)]
pub fn validate_shared_md_references(diag: &mut DiagnosticCollector, exclude: &ExcludeSet) {
    let skills_dir = Path::new("skills");
    if !skills_dir.is_dir() {
        return;
    }

    for entry in traversal::shallow_directories(skills_dir, Path::new("."), None).entries {
        let path = entry.path;
        let dir_name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };
        if dir_name == "shared" {
            continue;
        }

        let skill_md = path.join("SKILL.md");
        if !skill_md.is_file() {
            continue;
        }

        let skill_path = format!("skills/{dir_name}/SKILL.md");
        if exclude.is_excluded(&skill_path) {
            continue;
        }

        let content = match fs::read_to_string(&skill_md) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let mut seen = HashSet::new();
        for shared_ref in shared_md_refs::find_shared_md_refs(&content, "skills") {
            if !seen.insert(shared_ref.relative_path.clone()) {
                continue;
            }
            if !Path::new(&shared_ref.relative_path).is_file() {
                diag.report_at_with(
                    LintRule::SharedMdMissing,
                    &skill_path,
                    &format!(
                        "shared markdown reference missing on disk: {} (in {skill_path}, expected {})",
                        shared_ref.reference, shared_ref.relative_path
                    ),
                    DiagnosticMetadata::at_line(shared_ref.line),
                );
            }
        }
    }
}

pub fn validate_discovered_shared_md_references(
    ctx: &LintContext,
    diag: &mut DiagnosticCollector,
    exclude: &ExcludeSet,
) {
    for path in
        super::skill_discovery::SkillDiscovery::from_context(ctx, exclude).exported_skill_files
    {
        let display = path.to_string_lossy().replace('\\', "/");
        let Ok(content) = fs::read_to_string(&path) else {
            continue;
        };
        let mut seen = HashSet::new();
        for reference in shared_md_refs::find_shared_md_refs(&content, "skills") {
            if seen.insert(reference.relative_path.clone())
                && !Path::new(&reference.relative_path).is_file()
            {
                diag.report_at_with(
                    LintRule::SharedMdMissing,
                    &display,
                    &format!(
                        "shared markdown reference missing on disk: {} (in {display}, expected {})",
                        reference.reference, reference.relative_path
                    ),
                    DiagnosticMetadata::at_line(reference.line),
                );
            }
        }
    }
}

fn has_utf8_bom_before_opening_delimiter(content: &str) -> bool {
    const BOM: &[u8] = &[0xEF, 0xBB, 0xBF];
    let bytes = content.as_bytes();
    if !bytes.starts_with(BOM) {
        return false;
    }
    content[BOM.len()..]
        .lines()
        .next()
        .is_some_and(|line| line == "---")
}

/// S005 keeps a structured line only when the required key is present but not a
/// usable non-empty string. Absent or non-locatable keys stay file-level.
fn s005_location(
    fm_lines: &[String],
    parsed: Option<&crate::yaml::Value>,
    key: &str,
) -> DiagnosticMetadata {
    let present = parsed.map_or_else(
        || frontmatter::field_exists(fm_lines, key),
        |yaml| {
            yaml.as_mapping()
                .is_some_and(|mapping| mapping.get(key).is_some())
        },
    );
    if !present {
        return DiagnosticMetadata::default();
    }
    frontmatter::simple_top_level_key_line(fm_lines, key)
        .map_or_else(DiagnosticMetadata::default, DiagnosticMetadata::at_line)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostic::DiagnosticCollector;

    // V5: validate_skills_layout
    #[test]
    #[serial_test::serial]
    fn test_v5_valid_skills_layout() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write("skills/my-skill/SKILL.md", "---\nname: my-skill\n---\n").unwrap();

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skills_layout(&mut diag, &crate::config::ExcludeSet::default());
        assert_eq!(diag.error_count(), 0);
    }

    #[test]
    #[serial_test::serial]
    fn test_v5_missing_skills_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skills_layout(&mut diag, &crate::config::ExcludeSet::default());
        assert_eq!(diag.error_count(), 0);
    }

    #[test]
    #[serial_test::serial]
    fn test_v5_missing_skill_md() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("skills/my-skill").unwrap();
        // No SKILL.md file

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skills_layout(&mut diag, &crate::config::ExcludeSet::default());
        // Missing SKILL.md + no skills found = 2 errors
        assert!(diag.error_count() >= 1);
        assert!(diag.errors().iter().any(|e| e.contains("missing SKILL.md")));
        assert!(
            diag.diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.rule == LintRule::SkillMdMissing)
        );
        assert!(
            diag.diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.rule == LintRule::NoExportedSkills)
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_v5_shared_dir_skipped() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("skills/shared").unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write("skills/my-skill/SKILL.md", "---\nname: my-skill\n---\n").unwrap();

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skills_layout(&mut diag, &crate::config::ExcludeSet::default());
        assert_eq!(diag.error_count(), 0);
    }

    // V6: validate_skill_frontmatter (public skills)
    #[test]
    #[serial_test::serial]
    fn test_v6_valid_frontmatter() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A skill\n---\nBody\n",
        )
        .unwrap();

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_frontmatter(&mut diag, &crate::config::ExcludeSet::default());
        assert_eq!(diag.error_count(), 0);
    }

    #[test]
    #[serial_test::serial]
    fn name_directory_match_uses_the_canonical_yaml_scalar() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all(".agents/skills/valid-name").unwrap();
        std::fs::write(
            ".agents/skills/valid-name/SKILL.md",
            "---\nname: valid-name # comment\ndescription: A skill\n---\nBody\n",
        )
        .unwrap();

        let mut diag = DiagnosticCollector::new_all_enabled();
        let mut prompt_pass = super::super::prompt_content::PromptContentPass::default();
        validate_agent_skill_frontmatter_with_prompt_pass(
            &mut diag,
            &crate::config::ExcludeSet::default(),
            &mut prompt_pass,
        );
        assert!(
            diag.diagnostics()
                .iter()
                .all(|item| item.rule != LintRule::FrontmatterNameMismatch)
        );
    }

    #[test]
    #[serial_test::serial]
    fn non_string_name_is_owned_by_frontmatter_without_name_format_findings() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();
        std::fs::create_dir_all(".agents/skills/example").unwrap();
        std::fs::write(
            ".agents/skills/example/SKILL.md",
            "---\nname: [not-a-scalar]\ndescription: A skill\n---\nBody\n",
        )
        .unwrap();

        let exclude = crate::config::ExcludeSet::default();
        let mut diag = DiagnosticCollector::new_all_enabled();
        let mut prompt_pass = super::super::prompt_content::PromptContentPass::default();
        validate_agent_skill_frontmatter_with_prompt_pass(&mut diag, &exclude, &mut prompt_pass);
        crate::validators::skill_content::validate_agent_skills_contract(
            ".agents/skills",
            &mut diag,
            &exclude,
        );
        assert_eq!(
            diag.diagnostics()
                .iter()
                .filter(|item| item.rule == LintRule::FrontmatterFieldMissing)
                .count(),
            1
        );
        assert!(diag.diagnostics().iter().all(|item| {
            !matches!(
                item.rule,
                LintRule::NameTooLong | LintRule::NameInvalidChars | LintRule::NameBadHyphens
            )
        }));
    }

    #[test]
    #[serial_test::serial]
    fn test_v6_missing_name() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\ndescription: A skill\n---\nBody\n",
        )
        .unwrap();

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_frontmatter(&mut diag, &crate::config::ExcludeSet::default());
        assert!(diag.error_count() >= 1);
        assert!(diag.errors().iter().any(|e| e.contains("name")));
    }

    #[test]
    #[serial_test::serial]
    fn test_v6_name_mismatch() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: wrong-name\ndescription: A skill\n---\nBody\n",
        )
        .unwrap();

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_frontmatter(&mut diag, &crate::config::ExcludeSet::default());
        assert!(diag.error_count() >= 1);
        assert!(diag.errors().iter().any(|e| e.contains("does not match")));
    }

    #[test]
    #[serial_test::serial]
    fn test_v6_malformed_frontmatter() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write("skills/my-skill/SKILL.md", "no frontmatter\n").unwrap();

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_frontmatter(&mut diag, &crate::config::ExcludeSet::default());
        assert!(diag.error_count() >= 1);
        assert!(diag.errors().iter().any(|e| e.contains("malformed")));
    }

    // V6-adapted: validate_private_skill_frontmatter (Basic mode)
    #[test]
    #[serial_test::serial]
    fn test_v6a_valid_private_skill() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all(".claude/skills/my-skill").unwrap();
        std::fs::write(
            ".claude/skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: Private skill\n---\n",
        )
        .unwrap();

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_private_skill_frontmatter(&mut diag, &crate::config::ExcludeSet::default());
        assert_eq!(diag.error_count(), 0);
    }

    #[test]
    #[serial_test::serial]
    fn test_v6a_missing_description() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all(".claude/skills/my-skill").unwrap();
        std::fs::write(
            ".claude/skills/my-skill/SKILL.md",
            "---\nname: my-skill\n---\n",
        )
        .unwrap();

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_private_skill_frontmatter(&mut diag, &crate::config::ExcludeSet::default());
        assert!(diag.error_count() >= 1);
        assert!(diag.errors().iter().any(|e| e.contains("description")));
    }

    #[test]
    #[serial_test::serial]
    fn test_v6a_no_private_skills_dir_silent() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_private_skill_frontmatter(&mut diag, &crate::config::ExcludeSet::default());
        assert_eq!(diag.error_count(), 0);
    }

    // V15: validate_shared_md_references
    #[test]
    #[serial_test::serial]
    fn test_v15_valid_shared_reference() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("skills/shared").unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write("skills/shared/helpers.md", "# Helpers\n").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: s\n---\nSee ${CLAUDE_PLUGIN_ROOT}/skills/shared/helpers.md\n",
        )
        .unwrap();

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_shared_md_references(&mut diag, &crate::config::ExcludeSet::default());
        assert_eq!(diag.error_count(), 0);
    }

    #[test]
    #[serial_test::serial]
    fn test_v15_missing_shared_reference() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: s\n---\nSee ${CLAUDE_PLUGIN_ROOT}/skills/shared/nonexistent.md\n",
        )
        .unwrap();

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_shared_md_references(&mut diag, &crate::config::ExcludeSet::default());
        assert_eq!(diag.error_count(), 1);
        assert!(diag.errors()[0].contains("missing on disk"));
    }

    #[test]
    #[serial_test::serial]
    fn test_v15_subdirectory_shared_reference() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("skills/shared/sub").unwrap();
        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write("skills/shared/sub/util.md", "# Util\n").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: s\n---\nSee ${CLAUDE_PLUGIN_ROOT}/skills/shared/sub/util.md\n",
        )
        .unwrap();

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_shared_md_references(&mut diag, &crate::config::ExcludeSet::default());
        assert_eq!(diag.error_count(), 0);
    }

    #[test]
    #[serial_test::serial]
    fn test_v15_missing_subdirectory_shared_reference() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: s\n---\nSee ${CLAUDE_PLUGIN_ROOT}/skills/shared/sub/missing.md\n",
        )
        .unwrap();

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_shared_md_references(&mut diag, &crate::config::ExcludeSet::default());
        assert_eq!(diag.error_count(), 1);
        assert!(diag.errors()[0].contains("missing on disk"));
    }

    #[test]
    #[serial_test::serial]
    fn test_x001_invalid_yaml_frontmatter() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("skills/my-skill").unwrap();
        // Tab indentation is invalid YAML.
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\n\tdescription: bad\n---\nBody\n",
        )
        .unwrap();

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_frontmatter(&mut diag, &crate::config::ExcludeSet::default());
        let diagnostic = diag
            .diagnostics()
            .iter()
            .find(|diagnostic| diagnostic.rule == LintRule::FrontmatterYamlInvalid)
            .unwrap_or_else(|| {
                panic!(
                    "expected X001: {:?}",
                    diag.diagnostics()
                        .iter()
                        .map(|diagnostic| diagnostic.message.as_str())
                        .collect::<Vec<_>>()
                )
            });
        assert_eq!(
            diagnostic
                .location
                .map(|location| location.start().line_number()),
            Some(3)
        );
        assert!(
            !diagnostic.message.contains("at line"),
            "message must not embed parser coordinates: {}",
            diagnostic.message
        );
        let file_line_hits = diagnostic.message.matches(":3:").count();
        assert_eq!(
            file_line_hits, 1,
            "file line must appear exactly once: {}",
            diagnostic.message
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_x001_allows_trailing_bare_null_key() {
        // A frontmatter block whose final line is a bare `key:` (a null value)
        // is valid YAML; line extraction dropped the trailing newline the real
        // file carries before the closing `---`, which the strict parse restores
        // so this no longer emits a spurious X001.
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A valid skill description here\npaths:\n---\nBody\n",
        )
        .unwrap();

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_frontmatter(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            diag.diagnostics()
                .iter()
                .all(|diagnostic| diagnostic.rule != LintRule::FrontmatterYamlInvalid),
            "trailing bare null key must not emit X001: {:?}",
            diag.diagnostics()
                .iter()
                .map(|diagnostic| diagnostic.message.as_str())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_cc_sk_010_skill_hooks_schema() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A skill for testing hooks\nhooks:\n  NotAnEvent:\n    - hooks:\n        - type: command\n          command: echo hi\n---\nBody\n",
        )
        .unwrap();

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_frontmatter(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            diag.diagnostics()
                .iter()
                .any(|d| d.rule == LintRule::HookEventInvalid && d.message.contains("frontmatter")),
            "expected H008 on skill frontmatter: {:?}",
            diag.diagnostics()
                .iter()
                .map(|d| format!("{}:{}", d.rule.code(), d.message))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_x002_unclosed_fence_in_skill() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A skill for fence testing\n---\n```bash\necho hi\n",
        )
        .unwrap();

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_frontmatter(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            diag.diagnostics()
                .iter()
                .any(|d| d.rule == LintRule::UnclosedCodeFence)
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s073_deep_relative_md_link() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A skill for depth testing\n---\nSee [deep](refs/deep/nested/file.md)\n",
        )
        .unwrap();

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_frontmatter(&mut diag, &crate::config::ExcludeSet::default());
        let diagnostic = diag
            .diagnostics()
            .iter()
            .find(|d| d.rule == LintRule::SkillRefNested)
            .expect("expected S073");
        assert_eq!(
            diagnostic.location.map(|location| location.start()),
            Some(crate::diagnostic::SourcePosition::line(5))
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s073_skips_uri_schemes_and_non_md() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A skill for scheme and non-md depth skips\n---\n\
             See [hosts](file:///etc/hosts/extra)\n\
             Mail [x](mailto:x@y)\n\
             Data [csv](data/2024/q1/report.csv)\n\
             Anchor [ok](dir/file.md#anchor)\n",
        )
        .unwrap();

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_frontmatter(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .diagnostics()
                .iter()
                .any(|d| d.rule == LintRule::SkillRefNested),
            "unexpected S073: {:?}",
            diag.diagnostics()
                .iter()
                .map(|d| d.message.as_str())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s073_parent_hop_and_case_insensitive_md() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("skills/my-skill").unwrap();
        std::fs::write(
            "skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A skill for parent-hop depth testing\n---\n\
             See [parent](../other/file.md)\n\
             And [upper](refs/deep/FILE.MD)\n",
        )
        .unwrap();

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_frontmatter(&mut diag, &crate::config::ExcludeSet::default());
        let nested: Vec<_> = diag
            .diagnostics()
            .iter()
            .filter(|d| d.rule == LintRule::SkillRefNested)
            .collect();
        assert_eq!(nested.len(), 2, "expected both deep links: {nested:?}");
        assert!(nested.iter().all(|d| d.location.is_some()));
    }

    #[test]
    #[serial_test::serial]
    fn test_s072_counts_dist_and_plain_subdir() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("skills/heavy/dist").unwrap();
        std::fs::write(
            "skills/heavy/SKILL.md",
            "---\nname: heavy\ndescription: A skill for oversized directory testing\n---\nBody\n",
        )
        .unwrap();
        // 9MB under dist/ — previously skipped by IGNORED_DIRECTORY_NAMES.
        std::fs::write("skills/heavy/dist/blob.bin", vec![0u8; 9 * 1024 * 1024]).unwrap();

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_frontmatter(&mut diag, &crate::config::ExcludeSet::default());
        let diagnostic = diag
            .diagnostics()
            .iter()
            .find(|d| d.rule == LintRule::SkillDirOversized)
            .expect("expected S072 for dist/");
        assert!(diagnostic.message.contains("9437184") || diagnostic.message.contains("bytes"));

        // Regression: plain subdir still counted.
        let tmp2 = tempfile::tempdir().unwrap();
        std::env::set_current_dir(tmp2.path()).unwrap();
        std::fs::create_dir_all("skills/heavy/assets").unwrap();
        std::fs::write(
            "skills/heavy/SKILL.md",
            "---\nname: heavy\ndescription: A skill for oversized directory testing\n---\nBody\n",
        )
        .unwrap();
        std::fs::write("skills/heavy/assets/blob.bin", vec![0u8; 9 * 1024 * 1024]).unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_frontmatter(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            diag.diagnostics()
                .iter()
                .any(|d| d.rule == LintRule::SkillDirOversized)
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s072_skips_git_and_handles_symlinks() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("skills/linky/.git/objects").unwrap();
        std::fs::write(
            "skills/linky/SKILL.md",
            "---\nname: linky\ndescription: A skill for symlink size accounting\n---\nBody\n",
        )
        .unwrap();
        // Large payload only under .git must not trip S072.
        std::fs::write(
            "skills/linky/.git/objects/pack.bin",
            vec![0u8; 9 * 1024 * 1024],
        )
        .unwrap();

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_frontmatter(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .diagnostics()
                .iter()
                .any(|d| d.rule == LintRule::SkillDirOversized),
            ".git contents must not count"
        );

        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            let outside = tmp.path().join("outside-large.bin");
            std::fs::write(&outside, vec![0u8; 9 * 1024 * 1024]).unwrap();
            symlink(&outside, "skills/linky/via-file-link.bin").unwrap();

            let mut diag = DiagnosticCollector::new_all_enabled();
            validate_skill_frontmatter(&mut diag, &crate::config::ExcludeSet::default());
            assert!(
                diag.diagnostics()
                    .iter()
                    .any(|d| d.rule == LintRule::SkillDirOversized),
                "file symlink target size must count"
            );

            // Directory symlink must not be followed (and must not double-count).
            let tmp3 = tempfile::tempdir().unwrap();
            std::env::set_current_dir(tmp3.path()).unwrap();
            std::fs::create_dir_all("skills/linky").unwrap();
            std::fs::create_dir_all("external-dist").unwrap();
            std::fs::write(
                "skills/linky/SKILL.md",
                "---\nname: linky\ndescription: A skill for dir-symlink size accounting\n---\nBody\n",
            )
            .unwrap();
            std::fs::write("external-dist/blob.bin", vec![0u8; 9 * 1024 * 1024]).unwrap();
            symlink(tmp3.path().join("external-dist"), "skills/linky/dist").unwrap();
            let mut diag = DiagnosticCollector::new_all_enabled();
            validate_skill_frontmatter(&mut diag, &crate::config::ExcludeSet::default());
            assert!(
                !diag
                    .diagnostics()
                    .iter()
                    .any(|d| d.rule == LintRule::SkillDirOversized),
                "directory symlink must not be followed"
            );
        }
    }

    #[test]
    #[serial_test::serial]
    fn test_s006_private_skill_name_mismatch() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all(".claude/skills/my-skill").unwrap();
        std::fs::write(
            ".claude/skills/my-skill/SKILL.md",
            "---\nname: other-name\ndescription: A skill for basic mode\n---\nBody\n",
        )
        .unwrap();

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_private_skill_frontmatter(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            diag.diagnostics()
                .iter()
                .any(|d| d.rule == LintRule::FrontmatterNameMismatch)
        );
    }

    #[test]
    #[serial_test::serial]
    fn s007_uses_canonical_yaml_values_with_an_invalid_yaml_fallback() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        for (name, optional_field) in [
            ("continued", "argument-hint:\n  \"[issue-number]\""),
            ("folded", "argument-hint: >-\n  [issue-number]"),
            ("quoted-empty", "argument-hint: \"\""),
            ("quoted-key", "\"argument-hint\": \"\""),
            ("flow-tools", "allowed-tools: [Read, Write]"),
            ("block-tools", "allowed-tools:\n  - Read\n  - Write"),
            ("invalid", "argument-hint:\n\tinvalid: yaml"),
        ] {
            let path = format!(".claude/skills/{name}/SKILL.md");
            std::fs::create_dir_all(
                std::path::Path::new(&path)
                    .parent()
                    .expect("skill has a parent directory"),
            )
            .unwrap();
            std::fs::write(
                path,
                format!(
                    "---\nname: {name}\ndescription: A valid skill description\n{optional_field}\n---\nBody\n"
                ),
            )
            .unwrap();
        }

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_private_skill_frontmatter(&mut diag, &crate::config::ExcludeSet::default());
        let s007_subjects = diag
            .diagnostics()
            .iter()
            .filter(|diagnostic| diagnostic.rule == LintRule::FrontmatterFieldEmpty)
            .map(|diagnostic| {
                diagnostic
                    .subject_path
                    .as_ref()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect::<Vec<_>>();
        assert_eq!(
            s007_subjects,
            vec![
                ".claude/skills/invalid/SKILL.md".to_string(),
                ".claude/skills/quoted-empty/SKILL.md".to_string(),
                ".claude/skills/quoted-key/SKILL.md".to_string(),
            ]
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s008_dedupes_and_locates_missing_refs() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("skills/demo").unwrap();
        std::fs::write(
            "skills/demo/SKILL.md",
            "---\nname: demo\ndescription: s\n---\n\
             See ${CLAUDE_PLUGIN_ROOT}/skills/shared/missing.md\n\
             And again ${CLAUDE_PLUGIN_ROOT}/skills/shared/missing.md\n\
             Brace-less $CLAUDE_PLUGIN_ROOT/skills/shared/missing.md\n\
             Only brace-less $CLAUDE_PLUGIN_ROOT/skills/shared/unbraced-missing.md\n",
        )
        .unwrap();

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_shared_md_references(&mut diag, &crate::config::ExcludeSet::default());
        let missing: Vec<_> = diag
            .diagnostics()
            .iter()
            .filter(|d| d.rule == LintRule::SharedMdMissing)
            .collect();
        assert_eq!(missing.len(), 2, "{missing:?}");
        let first = missing
            .iter()
            .find(|d| d.message.contains("shared/missing.md"))
            .expect("missing.md");
        assert_eq!(
            first
                .location
                .map(|location| location.start().line_number()),
            Some(5)
        );
        let unbraced = missing
            .iter()
            .find(|d| d.message.contains("unbraced-missing.md"))
            .expect("unbraced");
        assert!(unbraced.message.contains("$CLAUDE_PLUGIN_ROOT/"));
        assert_eq!(
            unbraced
                .location
                .map(|location| location.start().line_number()),
            Some(8)
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s008_ignores_commented_and_prefix_tokens() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("skills/shared").unwrap();
        std::fs::create_dir_all("skills/demo").unwrap();
        std::fs::write("skills/shared/prefix.md", "# Prefix\n").unwrap();
        std::fs::write(
            "skills/demo/SKILL.md",
            "---\nname: demo\ndescription: s\n---\n\
             <!-- ${CLAUDE_PLUGIN_ROOT}/skills/shared/commented.md -->\n\
             ${CLAUDE_PLUGIN_ROOT}/skills/shared/prefix.md.backup\n\
             ${CLAUDE_PLUGIN_ROOT}/skills/shared/prefix.mdx\n\
             ${CLAUDE_PLUGIN_ROOT}/skills/shared/prefix.md/child\n",
        )
        .unwrap();

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_shared_md_references(&mut diag, &crate::config::ExcludeSet::default());
        assert!(
            !diag
                .diagnostics()
                .iter()
                .any(|d| d.rule == LintRule::SharedMdMissing),
            "{:?}",
            diag.diagnostics()
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s004_bom_hint_and_line_location() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("skills/bom").unwrap();
        let mut bom_content = vec![0xEF, 0xBB, 0xBF];
        bom_content.extend_from_slice(b"---\nname: bom\ndescription: s\n---\nBody\n");
        std::fs::write("skills/bom/SKILL.md", bom_content).unwrap();

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_frontmatter(&mut diag, &crate::config::ExcludeSet::default());
        let bom = diag
            .diagnostics()
            .iter()
            .find(|d| d.rule == LintRule::FrontmatterMalformed)
            .expect("S004");
        assert!(
            bom.message
                .contains("file starts with a UTF-8 byte-order mark; remove it"),
            "{}",
            bom.message
        );
        assert_eq!(
            bom.location.map(|location| location.start().line_number()),
            Some(1)
        );

        std::fs::create_dir_all("skills/plain").unwrap();
        std::fs::write("skills/plain/SKILL.md", "no frontmatter\n").unwrap();
        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_frontmatter(&mut diag, &crate::config::ExcludeSet::default());
        let plain = diag
            .diagnostics()
            .iter()
            .find(|d| {
                d.rule == LintRule::FrontmatterMalformed
                    && d.subject_path.as_deref()
                        == Some(std::path::Path::new("skills/plain/SKILL.md"))
            })
            .expect("plain S004");
        assert!(
            !plain.message.contains("byte-order mark"),
            "{}",
            plain.message
        );
        assert_eq!(
            plain
                .location
                .map(|location| location.start().line_number()),
            Some(1)
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_s005_s006_s007_structured_locations() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_helpers::CwdGuard::new();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir_all("skills/empty-name").unwrap();
        std::fs::write(
            "skills/empty-name/SKILL.md",
            "---\nname:\ndescription: A valid description for routing\n---\nBody\n",
        )
        .unwrap();
        std::fs::create_dir_all("skills/missing-desc").unwrap();
        std::fs::write(
            "skills/missing-desc/SKILL.md",
            "---\nname: missing-desc\n---\nBody\n",
        )
        .unwrap();
        std::fs::create_dir_all("skills/wrong-name").unwrap();
        std::fs::write(
            "skills/wrong-name/SKILL.md",
            "---\nname: other\ndescription: A valid description for routing\n---\nBody\n",
        )
        .unwrap();
        std::fs::create_dir_all("skills/empty-optional").unwrap();
        std::fs::write(
            "skills/empty-optional/SKILL.md",
            "---\nname: empty-optional\ndescription: A valid description for routing\nargument-hint:\n---\nBody\n",
        )
        .unwrap();

        let mut diag = DiagnosticCollector::new_all_enabled();
        validate_skill_frontmatter(&mut diag, &crate::config::ExcludeSet::default());

        let empty_name = diag
            .diagnostics()
            .iter()
            .find(|d| {
                d.rule == LintRule::FrontmatterFieldMissing
                    && d.subject_path.as_deref()
                        == Some(std::path::Path::new("skills/empty-name/SKILL.md"))
                    && d.message.contains("'name'")
            })
            .expect("S005 empty name");
        assert_eq!(
            empty_name.location.map(|l| l.start().line_number()),
            Some(2)
        );

        let missing_desc = diag
            .diagnostics()
            .iter()
            .find(|d| {
                d.rule == LintRule::FrontmatterFieldMissing
                    && d.subject_path.as_deref()
                        == Some(std::path::Path::new("skills/missing-desc/SKILL.md"))
            })
            .expect("S005 missing description");
        assert!(missing_desc.location.is_none());

        let mismatch = diag
            .diagnostics()
            .iter()
            .find(|d| d.rule == LintRule::FrontmatterNameMismatch)
            .expect("S006");
        assert_eq!(mismatch.location.map(|l| l.start().line_number()), Some(2));

        let empty_optional = diag
            .diagnostics()
            .iter()
            .find(|d| d.rule == LintRule::FrontmatterFieldEmpty)
            .expect("S007");
        assert_eq!(
            empty_optional.location.map(|l| l.start().line_number()),
            Some(4)
        );
    }
}
