use crate::context::{LintContext, ManifestState};
use crate::diagnostic::DiagnosticCollector;
use crate::rules::LintRule;
use regex::Regex;

/// V1: Validate .claude-plugin/plugin.json
pub fn validate_plugin_json(ctx: &LintContext, diag: &mut DiagnosticCollector) {
    let f = ".claude-plugin/plugin.json";
    let val = match &ctx.plugin_json {
        ManifestState::Missing => {
            diag.report(LintRule::PluginJsonMissing, &format!("{f} is missing"));
            return;
        }
        ManifestState::Invalid(e) => {
            diag.report(LintRule::PluginJsonInvalid, e);
            return;
        }
        ManifestState::Parsed(v) => v,
    };

    let name = val.get("name").and_then(|v| v.as_str()).unwrap_or("");
    let version = val.get("version").and_then(|v| v.as_str()).unwrap_or("");

    if name.is_empty() {
        diag.report(
            LintRule::PluginFieldMissing,
            &format!("{f} missing required field: name"),
        );
    }
    if version.is_empty() {
        diag.report(
            LintRule::PluginFieldMissing,
            &format!("{f} missing required field: version"),
        );
    } else {
        let semver_re = Regex::new(r"^[0-9]+\.[0-9]+\.[0-9]+$").unwrap();
        if !semver_re.is_match(version) {
            diag.report(
                LintRule::PluginVersionFormat,
                &format!("{f} version '{version}' is not strict MAJOR.MINOR.PATCH semver"),
            );
        }
    }
}

/// V2: Validate .claude-plugin/marketplace.json
pub fn validate_marketplace_json(ctx: &LintContext, diag: &mut DiagnosticCollector) {
    let f = ".claude-plugin/marketplace.json";
    let val = match &ctx.marketplace_json {
        ManifestState::Missing => {
            diag.report(LintRule::MarketplaceJsonMissing, &format!("{f} is missing"));
            return;
        }
        ManifestState::Invalid(e) => {
            diag.report(LintRule::MarketplaceJsonInvalid, e);
            return;
        }
        ManifestState::Parsed(v) => v,
    };

    let mp_name = val.get("name").and_then(|v| v.as_str()).unwrap_or("");
    let mp_owner = val
        .get("owner")
        .and_then(|o| o.get("name"))
        .and_then(|v| v.as_str())
        .unwrap_or("");

    if mp_name.is_empty() {
        diag.report(
            LintRule::MarketplaceFieldMissing,
            &format!("{f} missing required field: name"),
        );
    }
    if mp_owner.is_empty() {
        diag.report(
            LintRule::MarketplaceFieldMissing,
            &format!("{f} missing required field: owner.name"),
        );
    }

    let plugins = val.get("plugins").and_then(|v| v.as_array());
    match plugins {
        None => {
            diag.report(
                LintRule::MarketplacePluginsEmpty,
                &format!("{f} has empty plugins array"),
            );
        }
        Some(arr) if arr.is_empty() => {
            diag.report(
                LintRule::MarketplacePluginsEmpty,
                &format!("{f} has empty plugins array"),
            );
        }
        Some(arr) => {
            for (i, plugin) in arr.iter().enumerate() {
                let pname = plugin.get("name").and_then(|v| v.as_str()).unwrap_or("");
                let has_source = match plugin.get("source") {
                    Some(s) => {
                        (s.is_string() && !s.as_str().unwrap_or("").is_empty()) || s.is_object()
                    }
                    None => false,
                };
                if pname.is_empty() || !has_source {
                    diag.report(
                        LintRule::MarketplacePluginInvalid,
                        &format!(
                            "{f} has plugin entry with missing/invalid name or source (plugins[{i}])"
                        ),
                    );
                }
            }
        }
    }
}

/// V12: Validate marketplace.json enriched metadata (larch convention)
pub fn validate_marketplace_enriched(ctx: &LintContext, diag: &mut DiagnosticCollector) {
    let f = ".claude-plugin/marketplace.json";
    let val = match &ctx.marketplace_json {
        ManifestState::Parsed(v) => v,
        _ => return, // Missing/invalid already reported by V2
    };

    let email = val
        .get("owner")
        .and_then(|o| o.get("email"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if email.is_empty() {
        diag.report(
            LintRule::MarketplaceEnrichedMissing,
            &format!("{f} missing required field: owner.email"),
        );
    }

    if let Some(plugins) = val.get("plugins").and_then(|v| v.as_array()) {
        for (i, plugin) in plugins.iter().enumerate() {
            let cat = plugin
                .get("category")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if cat.is_empty() {
                diag.report(
                    LintRule::MarketplaceEnrichedMissing,
                    &format!("{f} plugins[{i}] missing required field: category"),
                );
            }
        }
    }
}

/// V13: Validate plugin.json enriched metadata (larch convention)
pub fn validate_plugin_enriched(ctx: &LintContext, diag: &mut DiagnosticCollector) {
    let f = ".claude-plugin/plugin.json";
    let val = match &ctx.plugin_json {
        ManifestState::Parsed(v) => v,
        _ => return, // Missing/invalid already reported by V1
    };

    let desc = val
        .get("description")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if desc.is_empty() {
        diag.report(
            LintRule::PluginEnrichedMissing,
            &format!("{f} missing required field: description"),
        );
    }

    let email = val
        .get("author")
        .and_then(|o| o.get("email"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if email.is_empty() {
        diag.report(
            LintRule::PluginEnrichedMissing,
            &format!("{f} missing required field: author.email"),
        );
    }

    // keywords must be a non-empty array
    match val.get("keywords") {
        Some(kw) if kw.is_array() && !kw.as_array().unwrap().is_empty() => {}
        _ => {
            diag.report(
                LintRule::PluginEnrichedMissing,
                &format!("{f} keywords must be a non-empty array"),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::LintMode;
    use serde_json::json;

    fn make_ctx(plugin: ManifestState, marketplace: ManifestState) -> LintContext {
        LintContext {
            repo_root: String::new(),
            mode: LintMode::Plugin,
            plugin_json: plugin,
            marketplace_json: marketplace,
            hooks_json: ManifestState::Missing,
            settings_json: ManifestState::Missing,
        }
    }

    // V1: validate_plugin_json
    #[test]
    fn test_v1_valid_plugin_json() {
        let val = json!({"name": "my-plugin", "version": "1.2.3"});
        let ctx = make_ctx(ManifestState::Parsed(val), ManifestState::Missing);
        let mut diag = DiagnosticCollector::new();
        validate_plugin_json(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 0);
    }

    #[test]
    fn test_v1_missing_plugin_json() {
        let ctx = make_ctx(ManifestState::Missing, ManifestState::Missing);
        let mut diag = DiagnosticCollector::new();
        validate_plugin_json(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 1);
        assert!(diag.errors()[0].contains("is missing"));
    }

    #[test]
    fn test_v1_invalid_plugin_json() {
        let ctx = make_ctx(
            ManifestState::Invalid("parse error".to_string()),
            ManifestState::Missing,
        );
        let mut diag = DiagnosticCollector::new();
        validate_plugin_json(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 1);
        assert!(diag.errors()[0].contains("parse error"));
    }

    #[test]
    fn test_v1_missing_name() {
        let val = json!({"version": "1.0.0"});
        let ctx = make_ctx(ManifestState::Parsed(val), ManifestState::Missing);
        let mut diag = DiagnosticCollector::new();
        validate_plugin_json(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 1);
        assert!(diag.errors()[0].contains("name"));
    }

    #[test]
    fn test_v1_invalid_semver() {
        let val = json!({"name": "p", "version": "not-a-version"});
        let ctx = make_ctx(ManifestState::Parsed(val), ManifestState::Missing);
        let mut diag = DiagnosticCollector::new();
        validate_plugin_json(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 1);
        assert!(diag.errors()[0].contains("semver"));
    }

    #[test]
    fn test_v1_missing_version() {
        let val = json!({"name": "p"});
        let ctx = make_ctx(ManifestState::Parsed(val), ManifestState::Missing);
        let mut diag = DiagnosticCollector::new();
        validate_plugin_json(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 1);
        assert!(diag.errors()[0].contains("version"));
    }

    // V2: validate_marketplace_json
    #[test]
    fn test_v2_valid_marketplace_json() {
        let val = json!({
            "name": "mp",
            "owner": {"name": "owner-name"},
            "plugins": [{"name": "p1", "source": "https://example.com"}]
        });
        let ctx = make_ctx(ManifestState::Missing, ManifestState::Parsed(val));
        let mut diag = DiagnosticCollector::new();
        validate_marketplace_json(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 0);
    }

    #[test]
    fn test_v2_missing_marketplace_json() {
        let ctx = make_ctx(ManifestState::Missing, ManifestState::Missing);
        let mut diag = DiagnosticCollector::new();
        validate_marketplace_json(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 1);
        assert!(diag.errors()[0].contains("is missing"));
    }

    #[test]
    fn test_v2_empty_plugins_array() {
        let val = json!({"name": "mp", "owner": {"name": "o"}, "plugins": []});
        let ctx = make_ctx(ManifestState::Missing, ManifestState::Parsed(val));
        let mut diag = DiagnosticCollector::new();
        validate_marketplace_json(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 1);
        assert!(diag.errors()[0].contains("empty plugins array"));
    }

    #[test]
    fn test_v2_missing_owner_name() {
        let val = json!({
            "name": "mp",
            "owner": {},
            "plugins": [{"name": "p", "source": "s"}]
        });
        let ctx = make_ctx(ManifestState::Missing, ManifestState::Parsed(val));
        let mut diag = DiagnosticCollector::new();
        validate_marketplace_json(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 1);
        assert!(diag.errors()[0].contains("owner.name"));
    }

    #[test]
    fn test_v2_plugin_entry_missing_source() {
        let val = json!({
            "name": "mp",
            "owner": {"name": "o"},
            "plugins": [{"name": "p"}]
        });
        let ctx = make_ctx(ManifestState::Missing, ManifestState::Parsed(val));
        let mut diag = DiagnosticCollector::new();
        validate_marketplace_json(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 1);
        assert!(diag.errors()[0].contains("plugins[0]"));
    }

    // V12: validate_marketplace_enriched
    #[test]
    fn test_v12_valid_enriched() {
        let val = json!({
            "name": "mp",
            "owner": {"name": "o", "email": "a@b.com"},
            "plugins": [{"name": "p", "source": "s", "category": "lint"}]
        });
        let ctx = make_ctx(ManifestState::Missing, ManifestState::Parsed(val));
        let mut diag = DiagnosticCollector::new();
        validate_marketplace_enriched(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 0);
    }

    #[test]
    fn test_v12_missing_owner_email() {
        let val = json!({
            "name": "mp",
            "owner": {"name": "o"},
            "plugins": [{"name": "p", "source": "s", "category": "lint"}]
        });
        let ctx = make_ctx(ManifestState::Missing, ManifestState::Parsed(val));
        let mut diag = DiagnosticCollector::new();
        validate_marketplace_enriched(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 1);
        assert!(diag.errors()[0].contains("owner.email"));
    }

    #[test]
    fn test_v12_missing_category() {
        let val = json!({
            "name": "mp",
            "owner": {"name": "o", "email": "a@b.com"},
            "plugins": [{"name": "p", "source": "s"}]
        });
        let ctx = make_ctx(ManifestState::Missing, ManifestState::Parsed(val));
        let mut diag = DiagnosticCollector::new();
        validate_marketplace_enriched(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 1);
        assert!(diag.errors()[0].contains("category"));
    }

    #[test]
    fn test_v12_skips_when_not_parsed() {
        let ctx = make_ctx(ManifestState::Missing, ManifestState::Missing);
        let mut diag = DiagnosticCollector::new();
        validate_marketplace_enriched(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 0);
    }

    // V13: validate_plugin_enriched
    #[test]
    fn test_v13_valid_enriched() {
        let val = json!({
            "name": "p",
            "version": "1.0.0",
            "description": "A plugin",
            "author": {"email": "a@b.com"},
            "keywords": ["lint"]
        });
        let ctx = make_ctx(ManifestState::Parsed(val), ManifestState::Missing);
        let mut diag = DiagnosticCollector::new();
        validate_plugin_enriched(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 0);
    }

    #[test]
    fn test_v13_missing_description() {
        let val = json!({
            "name": "p",
            "version": "1.0.0",
            "author": {"email": "a@b.com"},
            "keywords": ["lint"]
        });
        let ctx = make_ctx(ManifestState::Parsed(val), ManifestState::Missing);
        let mut diag = DiagnosticCollector::new();
        validate_plugin_enriched(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 1);
        assert!(diag.errors()[0].contains("description"));
    }

    #[test]
    fn test_v13_empty_keywords() {
        let val = json!({
            "name": "p",
            "version": "1.0.0",
            "description": "desc",
            "author": {"email": "a@b.com"},
            "keywords": []
        });
        let ctx = make_ctx(ManifestState::Parsed(val), ManifestState::Missing);
        let mut diag = DiagnosticCollector::new();
        validate_plugin_enriched(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 1);
        assert!(diag.errors()[0].contains("keywords"));
    }

    #[test]
    fn test_v13_skips_when_not_parsed() {
        let ctx = make_ctx(ManifestState::Missing, ManifestState::Missing);
        let mut diag = DiagnosticCollector::new();
        validate_plugin_enriched(&ctx, &mut diag);
        assert_eq!(diag.error_count(), 0);
    }
}
