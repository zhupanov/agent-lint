use crate::diagnostic::DiagnosticCollector;
use crate::frontmatter;
use crate::rules::LintRule;
use crate::validators::skills::SkillInfo;

pub(super) fn check_frontmatter_fields(info: &SkillInfo, diag: &mut DiagnosticCollector) {
    // S023: boolean fields
    for field_name in &["user-invocable", "disable-model-invocation"] {
        match frontmatter::get_field_state(&info.fm_lines, field_name) {
            frontmatter::FieldState::Value(val) => {
                if val != "true" && val != "false" {
                    diag.report(
                        LintRule::BoolFieldInvalid,
                        &format!(
                            "{}: '{}' must be true or false, got '{}'",
                            info.path, field_name, val
                        ),
                    );
                }
            }
            frontmatter::FieldState::Empty => {
                diag.report(
                    LintRule::BoolFieldInvalid,
                    &format!(
                        "{}: '{}' is present but empty (must be true or false)",
                        info.path, field_name
                    ),
                );
            }
            frontmatter::FieldState::Missing => {} // Not required
        }
    }

    // S024: context field
    match frontmatter::get_field_state(&info.fm_lines, "context") {
        frontmatter::FieldState::Value(val) => {
            if val != "fork" {
                diag.report(
                    LintRule::ContextFieldInvalid,
                    &format!("{}: 'context' must be 'fork', got '{}'", info.path, val),
                );
            }
        }
        frontmatter::FieldState::Empty => {
            diag.report(
                LintRule::ContextFieldInvalid,
                &format!(
                    "{}: 'context' is present but empty (must be 'fork')",
                    info.path
                ),
            );
        }
        frontmatter::FieldState::Missing => {}
    }

    // S025: effort field
    match frontmatter::get_field_state(&info.fm_lines, "effort") {
        frontmatter::FieldState::Value(val) => {
            if !["low", "medium", "high", "max"].contains(&val.as_str()) {
                diag.report(
                    LintRule::EffortFieldInvalid,
                    &format!(
                        "{}: 'effort' must be low/medium/high/max, got '{}'",
                        info.path, val
                    ),
                );
            }
        }
        frontmatter::FieldState::Empty => {
            diag.report(
                LintRule::EffortFieldInvalid,
                &format!("{}: 'effort' is present but empty", info.path),
            );
        }
        frontmatter::FieldState::Missing => {}
    }

    // S026: shell field
    match frontmatter::get_field_state(&info.fm_lines, "shell") {
        frontmatter::FieldState::Value(val) => {
            if !["bash", "powershell"].contains(&val.as_str()) {
                diag.report(
                    LintRule::ShellFieldInvalid,
                    &format!(
                        "{}: 'shell' must be bash/powershell, got '{}'",
                        info.path, val
                    ),
                );
            }
        }
        frontmatter::FieldState::Empty => {
            diag.report(
                LintRule::ShellFieldInvalid,
                &format!("{}: 'shell' is present but empty", info.path),
            );
        }
        frontmatter::FieldState::Missing => {}
    }

    // S027: unreachable skill
    let dmi = frontmatter::get_field(&info.fm_lines, "disable-model-invocation");
    let ui = frontmatter::get_field(&info.fm_lines, "user-invocable");
    if dmi.as_deref() == Some("true") && ui.as_deref() == Some("false") {
        diag.report(
            LintRule::SkillUnreachable,
            &format!(
                "{}: skill is unreachable (disable-model-invocation: true and user-invocable: false)",
                info.path
            ),
        );
    }
}
