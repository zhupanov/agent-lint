//! AST-backed architectural checks over production Rust source.
//!
//! This module is test-only so `syn` does not enter the shipped dependency
//! graph. The loader follows the same ordinary module-file layout as rustc and
//! deliberately excludes every item that is positively gated on `test`.

use proc_macro2::Span;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use syn::spanned::Spanned;
use syn::visit::{self, Visit};
use syn::{Attribute, Item, ItemMod, Meta, UseTree};

const FORBIDDEN_VALIDATOR_MACROS: &[&str] = &["dbg", "eprint", "eprintln", "print", "println"];

const FORBIDDEN_VALIDATOR_CALL_SUFFIXES: &[&str] = &[
    "process::exit",
    "process::abort",
    "process::Command::new",
    "Command::new",
    "env::set_current_dir",
    "env::set_var",
    "env::remove_var",
    "fs::write",
    "fs::copy",
    "fs::hard_link",
    "fs::rename",
    "fs::remove_file",
    "fs::remove_dir",
    "fs::remove_dir_all",
    "fs::set_permissions",
    "File::create",
    "File::options",
    "OpenOptions::new",
];

const FORBIDDEN_MUTATION_METHODS: &[&str] =
    &["set_len", "set_permissions", "write_all", "write_fmt"];

const VALIDATOR_IO_WRITE_IMPORT_EXCEPTIONS: &[(&str, &str)] = &[];

const VALIDATOR_PRIMITIVES: &[(&str, &str)] = &[
    ("agent_discovery", "canonical runtime-agent inventory"),
    (
        "codex_constants",
        "Codex vocabulary data without validator policy",
    ),
    ("common", "format-neutral validator helpers"),
    ("shared_md_refs", "shared Markdown-reference facts"),
    ("skill_discovery", "canonical runtime-skill inventory"),
];

const ALLOWED_VALIDATOR_PEER_EDGES: &[(&str, &str, &str)] = &[
    (
        "agents",
        "hook_schema",
        "agent frontmatter delegates hook-object semantics to the shared hook-schema engine",
    ),
    (
        "agents",
        "markdown_structure",
        "agent Markdown validation delegates its inseparable structure sub-check",
    ),
    (
        "agents",
        "prompt_content",
        "agent content feeds the shared prompt-content pass",
    ),
    (
        "codex_surfaces",
        "prompt_content",
        "Codex prompt surfaces feed the shared prompt-content pass",
    ),
    (
        "contracts",
        "npm_scripts",
        "L006 is dispatched as the npm-script sub-check of the shared contract pass",
    ),
    (
        "contracts",
        "shell",
        "shared contract rules consume the shell analyzer rather than a peer platform validator",
    ),
    (
        "cursor",
        "prompt_content",
        "Cursor prompt surfaces feed the shared prompt-content pass",
    ),
    (
        "desc_overlap",
        "cursor",
        "description overlap consumes Cursor's canonical agent-path discovery",
    ),
    (
        "desc_overlap",
        "skills",
        "description overlap consumes parsed skill records",
    ),
    (
        "docs",
        "markdown_structure",
        "CLAUDE.md documentation validation delegates its inseparable structure sub-check",
    ),
    (
        "hooks",
        "hook_schema",
        "hook surface adapters delegate object semantics to the shared hook-schema engine",
    ),
    (
        "instruction_files",
        "codex_config",
        "instruction-file validation consumes typed Codex project-document settings",
    ),
    (
        "instruction_files",
        "prompt_content",
        "instruction-file content feeds the shared prompt-content pass",
    ),
    (
        "skill_content",
        "manifest",
        "skill-content contract checks consume manifest-declared agent roots",
    ),
    (
        "skill_content",
        "prompt_content",
        "skill body content feeds the shared prompt-content pass",
    ),
    (
        "skill_content",
        "skills",
        "skill-content rules consume the canonical parsed SkillInfo record",
    ),
    (
        "skill_discovery",
        "manifest",
        "skill discovery consumes manifest-declared roots from the manifest owner",
    ),
    (
        "skills",
        "hook_schema",
        "skill frontmatter delegates hook-object semantics to the shared hook-schema engine",
    ),
    (
        "skills",
        "markdown_structure",
        "skill Markdown validation delegates its inseparable structure sub-check",
    ),
    (
        "skills",
        "prompt_content",
        "skill content feeds the shared prompt-content pass",
    ),
    (
        "skills",
        "skill_content",
        "skill validation consumes shared skill-content field contracts",
    ),
];

struct SourceModule {
    path: String,
    module: Vec<String>,
    syntax: syn::File,
}

/// A source file together with whether rustc only compiles it for tests.
///
/// Unlike the production-policy loader above, the CWD policy deliberately
/// follows `#[cfg(test)]` modules: those are the code it is meant to audit.
struct TestSourceModule {
    source: SourceModule,
    test_side: bool,
}

#[derive(Debug, Clone)]
struct Import {
    raw: Vec<String>,
    local: Option<String>,
    module: Vec<String>,
    span: Span,
    glob: bool,
}

#[derive(Debug, Clone)]
struct Alias {
    raw: Vec<String>,
    module: Vec<String>,
}

#[derive(Debug, Clone, Eq, Ord, PartialEq, PartialOrd)]
struct Finding {
    path: String,
    line: usize,
    kind: &'static str,
    rendered_path: String,
    detail: String,
}

#[derive(Debug, Clone, Eq, Ord, PartialEq, PartialOrd)]
struct PeerEdge {
    source: String,
    target: String,
    path: String,
    line: usize,
}

impl Finding {
    fn render(&self) -> String {
        format!(
            "{}:{}: {}: {}",
            self.path, self.line, self.kind, self.detail
        )
    }
}

fn load_production_modules(root: &Path) -> Result<Vec<SourceModule>, String> {
    let mut modules = Vec::new();
    let mut visited = HashSet::new();
    let shared_sources = (root == Path::new(env!("CARGO_MANIFEST_DIR"))).then(|| {
        crate::test_helpers::source_files()
            .into_iter()
            .map(|(path, source)| (root.join("src").join(path), source))
            .collect::<HashMap<_, _>>()
    });
    load_module_file(
        root,
        &root.join("src/main.rs"),
        Vec::new(),
        root.join("src"),
        shared_sources.as_ref(),
        &mut visited,
        &mut modules,
    )?;
    modules.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(modules)
}

fn load_module_file(
    root: &Path,
    file: &Path,
    module: Vec<String>,
    child_dir: PathBuf,
    shared_sources: Option<&HashMap<PathBuf, String>>,
    visited: &mut HashSet<PathBuf>,
    modules: &mut Vec<SourceModule>,
) -> Result<(), String> {
    let canonical_key = file.to_path_buf();
    if !visited.insert(canonical_key) {
        return Ok(());
    }
    let source = match shared_sources.and_then(|sources| sources.get(file)) {
        Some(source) => source.clone(),
        None => fs::read_to_string(file).map_err(|error| format!("{}: {error}", file.display()))?,
    };
    let syntax =
        syn::parse_file(&source).map_err(|error| format!("{}: {error}", file.display()))?;
    let path = file
        .strip_prefix(root)
        .unwrap_or(file)
        .to_string_lossy()
        .replace('\\', "/");
    discover_external_modules(
        root,
        &syntax.items,
        &module,
        &child_dir,
        shared_sources,
        visited,
        modules,
    )?;
    modules.push(SourceModule {
        path,
        module,
        syntax,
    });
    Ok(())
}

fn discover_external_modules(
    root: &Path,
    items: &[Item],
    module: &[String],
    child_dir: &Path,
    shared_sources: Option<&HashMap<PathBuf, String>>,
    visited: &mut HashSet<PathBuf>,
    modules: &mut Vec<SourceModule>,
) -> Result<(), String> {
    for item in items {
        let Item::Mod(item_mod) = item else { continue };
        if is_test_only(&item_mod.attrs) {
            continue;
        }
        let mut nested_module = module.to_vec();
        nested_module.push(item_mod.ident.to_string());
        if let Some((_, nested_items)) = &item_mod.content {
            discover_external_modules(
                root,
                nested_items,
                &nested_module,
                &child_dir.join(item_mod.ident.to_string()),
                shared_sources,
                visited,
                modules,
            )?;
            continue;
        }
        let stem = item_mod.ident.to_string();
        let flat = child_dir.join(format!("{stem}.rs"));
        let nested = child_dir.join(&stem).join("mod.rs");
        let (file, next_child_dir) = if flat.is_file() {
            (flat, child_dir.join(stem))
        } else if nested.is_file() {
            (nested, child_dir.join(stem))
        } else {
            return Err(format!(
                "cannot resolve external module {} declared below {}",
                item_mod.ident,
                child_dir.display()
            ));
        };
        load_module_file(
            root,
            &file,
            nested_module,
            next_child_dir,
            shared_sources,
            visited,
            modules,
        )?;
    }
    Ok(())
}

fn is_test_only(attrs: &[Attribute]) -> bool {
    attrs.iter().any(|attr| {
        attr.path().is_ident("cfg")
            && attr
                .parse_args::<Meta>()
                .is_ok_and(|meta| meta_has_positive_test(&meta))
    })
}

fn meta_has_positive_test(meta: &Meta) -> bool {
    match meta {
        Meta::Path(path) => path.is_ident("test"),
        Meta::List(list) if list.path.is_ident("not") => false,
        Meta::List(list) => list
            .parse_args_with(syn::punctuated::Punctuated::<Meta, syn::Token![,]>::parse_terminated)
            .is_ok_and(|items| items.iter().any(meta_has_positive_test)),
        Meta::NameValue(_) => false,
    }
}

fn item_attrs(item: &Item) -> &[Attribute] {
    match item {
        Item::Const(item) => &item.attrs,
        Item::Enum(item) => &item.attrs,
        Item::ExternCrate(item) => &item.attrs,
        Item::Fn(item) => &item.attrs,
        Item::ForeignMod(item) => &item.attrs,
        Item::Impl(item) => &item.attrs,
        Item::Macro(item) => &item.attrs,
        Item::Mod(item) => &item.attrs,
        Item::Static(item) => &item.attrs,
        Item::Struct(item) => &item.attrs,
        Item::Trait(item) => &item.attrs,
        Item::TraitAlias(item) => &item.attrs,
        Item::Type(item) => &item.attrs,
        Item::Union(item) => &item.attrs,
        Item::Use(item) => &item.attrs,
        Item::Verbatim(_) => &[],
        _ => &[],
    }
}

fn impl_item_attrs(item: &syn::ImplItem) -> &[Attribute] {
    match item {
        syn::ImplItem::Const(item) => &item.attrs,
        syn::ImplItem::Fn(item) => &item.attrs,
        syn::ImplItem::Type(item) => &item.attrs,
        syn::ImplItem::Macro(item) => &item.attrs,
        syn::ImplItem::Verbatim(_) => &[],
        _ => &[],
    }
}

fn trait_item_attrs(item: &syn::TraitItem) -> &[Attribute] {
    match item {
        syn::TraitItem::Const(item) => &item.attrs,
        syn::TraitItem::Fn(item) => &item.attrs,
        syn::TraitItem::Type(item) => &item.attrs,
        syn::TraitItem::Macro(item) => &item.attrs,
        syn::TraitItem::Verbatim(_) => &[],
        _ => &[],
    }
}

fn foreign_item_attrs(item: &syn::ForeignItem) -> &[Attribute] {
    match item {
        syn::ForeignItem::Fn(item) => &item.attrs,
        syn::ForeignItem::Static(item) => &item.attrs,
        syn::ForeignItem::Type(item) => &item.attrs,
        syn::ForeignItem::Macro(item) => &item.attrs,
        syn::ForeignItem::Verbatim(_) => &[],
        _ => &[],
    }
}

struct ImportCollector {
    module: Vec<String>,
    imports: Vec<Import>,
}

impl<'ast> Visit<'ast> for ImportCollector {
    fn visit_item(&mut self, item: &'ast Item) {
        if !is_test_only(item_attrs(item)) {
            visit::visit_item(self, item);
        }
    }

    fn visit_item_mod(&mut self, item: &'ast ItemMod) {
        if is_test_only(&item.attrs) {
            return;
        }
        let Some((_, items)) = &item.content else {
            return;
        };
        self.module.push(item.ident.to_string());
        for item in items {
            self.visit_item(item);
        }
        self.module.pop();
    }

    fn visit_impl_item(&mut self, item: &'ast syn::ImplItem) {
        if !is_test_only(impl_item_attrs(item)) {
            visit::visit_impl_item(self, item);
        }
    }

    fn visit_trait_item(&mut self, item: &'ast syn::TraitItem) {
        if !is_test_only(trait_item_attrs(item)) {
            visit::visit_trait_item(self, item);
        }
    }

    fn visit_foreign_item(&mut self, item: &'ast syn::ForeignItem) {
        if !is_test_only(foreign_item_attrs(item)) {
            visit::visit_foreign_item(self, item);
        }
    }

    fn visit_item_use(&mut self, item: &'ast syn::ItemUse) {
        if is_test_only(&item.attrs) {
            return;
        }
        flatten_use_tree(&item.tree, Vec::new(), &self.module, &mut self.imports);
    }
}

fn flatten_use_tree(
    tree: &UseTree,
    mut prefix: Vec<String>,
    module: &[String],
    imports: &mut Vec<Import>,
) {
    match tree {
        UseTree::Path(path) => {
            prefix.push(path.ident.to_string());
            flatten_use_tree(&path.tree, prefix, module, imports);
        }
        UseTree::Name(name) => {
            let ident = name.ident.to_string();
            let local = if ident == "self" {
                prefix.last().cloned()
            } else {
                prefix.push(ident.clone());
                Some(ident)
            };
            imports.push(Import {
                raw: prefix,
                local,
                module: module.to_vec(),
                span: name.span(),
                glob: false,
            });
        }
        UseTree::Rename(rename) => {
            let ident = rename.ident.to_string();
            if ident != "self" {
                prefix.push(ident);
            }
            imports.push(Import {
                raw: prefix,
                local: Some(rename.rename.to_string()),
                module: module.to_vec(),
                span: rename.span(),
                glob: false,
            });
        }
        UseTree::Glob(glob) => imports.push(Import {
            raw: prefix,
            local: None,
            module: module.to_vec(),
            span: glob.span(),
            glob: true,
        }),
        UseTree::Group(group) => {
            for tree in &group.items {
                flatten_use_tree(tree, prefix.clone(), module, imports);
            }
        }
    }
}

fn aliases_for(imports: &[Import]) -> HashMap<String, Alias> {
    imports
        .iter()
        .filter_map(|import| {
            import.local.as_ref().map(|local| {
                (
                    local.clone(),
                    Alias {
                        raw: import.raw.clone(),
                        module: import.module.clone(),
                    },
                )
            })
        })
        .collect()
}

fn resolve_path(
    raw: &[String],
    module: &[String],
    aliases: &HashMap<String, Alias>,
) -> Vec<String> {
    resolve_path_inner(raw, module, aliases, &mut HashSet::new())
}

fn resolve_path_inner(
    raw: &[String],
    module: &[String],
    aliases: &HashMap<String, Alias>,
    resolving: &mut HashSet<String>,
) -> Vec<String> {
    let Some(first) = raw.first() else {
        return Vec::new();
    };
    if let Some(alias) = aliases.get(first)
        && resolving.insert(first.clone())
    {
        let mut resolved = resolve_path_inner(&alias.raw, &alias.module, aliases, resolving);
        resolved.extend_from_slice(&raw[1..]);
        return resolved;
    }
    if first == "crate" {
        return raw[1..].to_vec();
    }
    if first == "self" {
        let mut resolved = module.to_vec();
        resolved.extend_from_slice(&raw[1..]);
        return resolved;
    }
    if first == "super" {
        let mut resolved = module.to_vec();
        let mut index = 0;
        while raw.get(index).is_some_and(|segment| segment == "super") {
            resolved.pop();
            index += 1;
        }
        resolved.extend_from_slice(&raw[index..]);
        return resolved;
    }
    raw.to_vec()
}

fn path_segments(path: &syn::Path) -> Vec<String> {
    path.segments
        .iter()
        .map(|segment| segment.ident.to_string())
        .collect()
}

fn domain(module: &[String]) -> Option<&str> {
    (module.first().is_some_and(|part| part == "validators") && module.len() > 1)
        .then(|| module[1].as_str())
}

fn has_suffix(path: &[String], suffix: &str) -> bool {
    let suffix = suffix.split("::").collect::<Vec<_>>();
    path.len() >= suffix.len()
        && path[path.len() - suffix.len()..]
            .iter()
            .map(String::as_str)
            .eq(suffix)
}

fn location(span: Span) -> usize {
    span.start().line
}

fn peer_target(path: &[String]) -> Option<&str> {
    (path.first().is_some_and(|part| part == "validators") && path.len() > 1)
        .then(|| path[1].as_str())
}

fn is_validator_primitive(target: &str) -> bool {
    VALIDATOR_PRIMITIVES
        .iter()
        .any(|(primitive, _)| *primitive == target)
}

struct PeerEdgeCollector<'a> {
    path: &'a str,
    module: Vec<String>,
    aliases: &'a HashMap<String, Alias>,
    edges: BTreeMap<(String, String), PeerEdge>,
}

impl PeerEdgeCollector<'_> {
    fn inspect_path(&mut self, path: &syn::Path, span: Span) {
        let raw = path_segments(path);
        self.inspect_raw_path(&raw, span);
    }

    fn inspect_raw_path(&mut self, raw: &[String], span: Span) {
        let Some(source) = domain(&self.module) else {
            return;
        };
        let resolved = resolve_path(raw, &self.module, self.aliases);
        let Some(target) = peer_target(&resolved) else {
            return;
        };
        if source == target || is_validator_primitive(target) {
            return;
        }
        let edge = PeerEdge {
            source: source.to_string(),
            target: target.to_string(),
            path: self.path.to_string(),
            line: location(span),
        };
        self.edges
            .entry((edge.source.clone(), edge.target.clone()))
            .or_insert(edge);
    }
}

impl<'ast> Visit<'ast> for PeerEdgeCollector<'_> {
    fn visit_item(&mut self, item: &'ast Item) {
        if !is_test_only(item_attrs(item)) {
            visit::visit_item(self, item);
        }
    }

    fn visit_item_mod(&mut self, item: &'ast ItemMod) {
        if is_test_only(&item.attrs) {
            return;
        }
        let Some((_, items)) = &item.content else {
            return;
        };
        self.module.push(item.ident.to_string());
        for item in items {
            self.visit_item(item);
        }
        self.module.pop();
    }

    fn visit_impl_item(&mut self, item: &'ast syn::ImplItem) {
        if !is_test_only(impl_item_attrs(item)) {
            visit::visit_impl_item(self, item);
        }
    }

    fn visit_trait_item(&mut self, item: &'ast syn::TraitItem) {
        if !is_test_only(trait_item_attrs(item)) {
            visit::visit_trait_item(self, item);
        }
    }

    fn visit_foreign_item(&mut self, item: &'ast syn::ForeignItem) {
        if !is_test_only(foreign_item_attrs(item)) {
            visit::visit_foreign_item(self, item);
        }
    }

    fn visit_item_use(&mut self, _item: &'ast syn::ItemUse) {}

    fn visit_expr_path(&mut self, expression: &'ast syn::ExprPath) {
        self.inspect_path(&expression.path, expression.span());
        visit::visit_expr_path(self, expression);
    }

    fn visit_type_path(&mut self, ty: &'ast syn::TypePath) {
        self.inspect_path(&ty.path, ty.span());
        visit::visit_type_path(self, ty);
    }

    fn visit_macro(&mut self, item: &'ast syn::Macro) {
        self.inspect_path(&item.path, item.path.span());
        visit::visit_macro(self, item);
    }
}

fn peer_edges(source: &SourceModule) -> BTreeMap<(String, String), PeerEdge> {
    let mut imports = ImportCollector {
        module: source.module.clone(),
        imports: Vec::new(),
    };
    imports.visit_file(&source.syntax);
    let aliases = aliases_for(&imports.imports);
    let mut collector = PeerEdgeCollector {
        path: &source.path,
        module: source.module.clone(),
        aliases: &aliases,
        edges: BTreeMap::new(),
    };
    for import in &imports.imports {
        collector.module.clone_from(&import.module);
        collector.inspect_raw_path(&import.raw, import.span);
    }
    collector.module.clone_from(&source.module);
    collector.visit_file(&source.syntax);
    collector.edges
}

fn validator_peer_edges(root: &Path) -> Result<BTreeMap<(String, String), PeerEdge>, String> {
    let mut edges = BTreeMap::new();
    for source in load_production_modules(root)? {
        for (key, edge) in peer_edges(&source) {
            edges.entry(key).or_insert(edge);
        }
    }
    Ok(edges)
}

fn inventory_errors(edges: &BTreeMap<(String, String), PeerEdge>) -> Vec<String> {
    inventory_errors_for(edges, VALIDATOR_PRIMITIVES, ALLOWED_VALIDATOR_PEER_EDGES)
}

fn inventory_errors_for(
    edges: &BTreeMap<(String, String), PeerEdge>,
    primitives: &[(&str, &str)],
    allowed_rows: &[(&str, &str, &str)],
) -> Vec<String> {
    let mut errors = Vec::new();
    let mut allowed = BTreeSet::new();
    let mut primitive_names = BTreeSet::new();
    for (primitive, reason) in primitives {
        if !primitive_names.insert(*primitive) {
            errors.push(format!("duplicate validator primitive: {primitive}"));
        }
        if reason.trim().is_empty() {
            errors.push(format!("validator primitive {primitive} needs a reason"));
        }
    }
    for (source, target, reason) in allowed_rows {
        if !allowed.insert((*source, *target)) {
            errors.push(format!(
                "duplicate validator peer edge: {source} -> {target}"
            ));
        }
        if reason.trim().is_empty() {
            errors.push(format!(
                "validator peer edge {source} -> {target} needs a reason"
            ));
        }
        if source == target {
            errors.push(format!(
                "validator peer edge cannot be self-referential: {source}"
            ));
        }
        if primitive_names.contains(target) {
            errors.push(format!(
                "validator peer edge {source} -> {target} targets a shared primitive"
            ));
        }
    }
    for ((source, target), edge) in edges {
        if !allowed.contains(&(source.as_str(), target.as_str())) {
            errors.push(format!(
                "new validator peer edge: {source} -> {target} (first seen at {}:{}); route through validators/mod.rs or add a narrowly justified edge",
                edge.path, edge.line
            ));
        }
    }
    for (source, target) in allowed {
        if !edges.contains_key(&(source.to_string(), target.to_string())) {
            errors.push(format!(
                "stale validator peer edge: {source} -> {target}; remove its allowlist row"
            ));
        }
    }
    errors
}

struct PolicyVisitor<'a> {
    path: &'a str,
    module: Vec<String>,
    aliases: &'a HashMap<String, Alias>,
    findings: BTreeSet<Finding>,
}

impl PolicyVisitor<'_> {
    fn inspect_path(&mut self, path: &syn::Path, span: Span, called: bool) {
        let raw = path_segments(path);
        let resolved = resolve_path(&raw, &self.module, self.aliases);
        if self.path != "src/traversal.rs" && resolved.first().is_some_and(|part| part == "walkdir")
        {
            self.add_finding(
                span,
                "traversal-ownership",
                &resolved,
                format!(
                    "forbidden walkdir path {}; src/traversal.rs owns recursive walking",
                    resolved.join("::")
                ),
            );
        }
        if !called || domain(&self.module).is_none() {
            return;
        }
        if let Some(suffix) = FORBIDDEN_VALIDATOR_CALL_SUFFIXES
            .iter()
            .find(|suffix| has_suffix(&resolved, suffix))
        {
            let rendered = if resolved.is_empty() {
                (*suffix).to_string()
            } else {
                resolved.join("::")
            };
            self.add_finding(
                span,
                "validator-purity",
                &resolved,
                format!(
                    "forbidden call {}; validators report facts and autofix.rs owns mutation",
                    rendered
                ),
            );
        }
    }

    fn add_finding(&mut self, span: Span, kind: &'static str, path: &[String], detail: String) {
        self.findings.insert(Finding {
            path: self.path.to_string(),
            line: location(span),
            kind,
            rendered_path: path.join("::"),
            detail,
        });
    }
}

impl<'ast> Visit<'ast> for PolicyVisitor<'_> {
    fn visit_item(&mut self, item: &'ast Item) {
        if !is_test_only(item_attrs(item)) {
            visit::visit_item(self, item);
        }
    }

    fn visit_item_mod(&mut self, item: &'ast ItemMod) {
        if is_test_only(&item.attrs) {
            return;
        }
        let Some((_, items)) = &item.content else {
            return;
        };
        self.module.push(item.ident.to_string());
        for item in items {
            self.visit_item(item);
        }
        self.module.pop();
    }

    fn visit_impl_item(&mut self, item: &'ast syn::ImplItem) {
        if !is_test_only(impl_item_attrs(item)) {
            visit::visit_impl_item(self, item);
        }
    }

    fn visit_trait_item(&mut self, item: &'ast syn::TraitItem) {
        if !is_test_only(trait_item_attrs(item)) {
            visit::visit_trait_item(self, item);
        }
    }

    fn visit_foreign_item(&mut self, item: &'ast syn::ForeignItem) {
        if !is_test_only(foreign_item_attrs(item)) {
            visit::visit_foreign_item(self, item);
        }
    }

    fn visit_item_use(&mut self, _item: &'ast syn::ItemUse) {}

    fn visit_expr_call(&mut self, expression: &'ast syn::ExprCall) {
        if let syn::Expr::Path(path) = &*expression.func {
            self.inspect_path(&path.path, path.span(), true);
        }
        visit::visit_expr_call(self, expression);
    }

    fn visit_expr_method_call(&mut self, expression: &'ast syn::ExprMethodCall) {
        if domain(&self.module).is_some()
            && FORBIDDEN_MUTATION_METHODS.contains(&expression.method.to_string().as_str())
        {
            let rendered = expression.method.to_string();
            self.add_finding(
                expression.method.span(),
                "validator-purity",
                std::slice::from_ref(&rendered),
                format!(
                    "forbidden mutation method {rendered}; validators report facts and autofix.rs owns mutation"
                ),
            );
        }
        visit::visit_expr_method_call(self, expression);
    }

    fn visit_macro(&mut self, item: &'ast syn::Macro) {
        let raw = path_segments(&item.path);
        let resolved = resolve_path(&raw, &self.module, self.aliases);
        if domain(&self.module).is_some()
            && raw
                .last()
                .is_some_and(|name| FORBIDDEN_VALIDATOR_MACROS.contains(&name.as_str()))
        {
            self.add_finding(
                item.path.span(),
                "validator-purity",
                &resolved,
                format!("forbidden macro {}", resolved.join("::")),
            );
        }
        self.inspect_path(&item.path, item.path.span(), false);
        visit::visit_macro(self, item);
    }

    fn visit_expr_path(&mut self, expression: &'ast syn::ExprPath) {
        self.inspect_path(&expression.path, expression.span(), false);
        visit::visit_expr_path(self, expression);
    }

    fn visit_type_path(&mut self, ty: &'ast syn::TypePath) {
        self.inspect_path(&ty.path, ty.span(), false);
        visit::visit_type_path(self, ty);
    }
}

fn inspect_module(source: &SourceModule) -> BTreeSet<Finding> {
    let mut collector = ImportCollector {
        module: source.module.clone(),
        imports: Vec::new(),
    };
    collector.visit_file(&source.syntax);
    let aliases = aliases_for(&collector.imports);
    let mut visitor = PolicyVisitor {
        path: &source.path,
        module: source.module.clone(),
        aliases: &aliases,
        findings: BTreeSet::new(),
    };

    for import in &collector.imports {
        let resolved = resolve_path(&import.raw, &import.module, &aliases);
        visitor.module.clone_from(&import.module);
        if source.path != "src/traversal.rs"
            && resolved.first().is_some_and(|part| part == "walkdir")
        {
            visitor.add_finding(
                import.span,
                "traversal-ownership",
                &resolved,
                format!(
                    "forbidden walkdir import {}; src/traversal.rs owns recursive walking",
                    resolved.join("::")
                ),
            );
        }
        if domain(&import.module).is_none() {
            continue;
        }
        if import.glob
            && ["std::fs", "std::env", "std::process"]
                .iter()
                .any(|owner| resolved.join("::") == *owner)
        {
            visitor.add_finding(
                import.span,
                "validator-purity",
                &resolved,
                format!(
                    "forbidden glob import {}::*; mutation targets cannot be proven",
                    resolved.join("::")
                ),
            );
        }
        if resolved == ["std", "io", "Write"]
            && !VALIDATOR_IO_WRITE_IMPORT_EXCEPTIONS
                .iter()
                .any(|(path, _)| *path == source.path)
        {
            visitor.add_finding(
                import.span,
                "validator-purity",
                &resolved,
                "forbidden import std::io::Write; syntax-only analysis cannot prove write receivers are non-filesystem".to_string(),
            );
        }
    }
    visitor.module.clone_from(&source.module);
    visitor.visit_file(&source.syntax);
    visitor.findings
}

fn inspect_tree(root: &Path) -> Result<BTreeSet<Finding>, String> {
    let mut findings = BTreeSet::new();
    for source in load_production_modules(root)? {
        findings.extend(inspect_module(&source));
    }
    Ok(findings)
}

const UNREACHABLE_CWD_HELPERS: &[(&str, &str)] = &[];

#[derive(Clone, Debug)]
struct CwdFunction {
    identity: String,
    path: String,
    module: Vec<String>,
    name: String,
    line: usize,
    is_test: bool,
    serial_key: Option<Option<String>>,
    calls: Vec<Vec<String>>,
    mutations: Vec<CwdEvent>,
    guards: Vec<CwdEvent>,
    drops: Vec<CwdEvent>,
    unsafe_guard: bool,
    env_mutations: Vec<CwdEvent>,
}

#[derive(Clone, Debug)]
struct CwdEvent {
    line: usize,
    order: usize,
    scope: Vec<usize>,
    name: Option<String>,
}

fn is_prefix(prefix: &[usize], value: &[usize]) -> bool {
    value.starts_with(prefix)
}

fn test_identity(path: &str, module: &[String], name: &str) -> String {
    std::iter::once(path)
        .chain(module.iter().map(String::as_str))
        .chain(std::iter::once(name))
        .collect::<Vec<_>>()
        .join("::")
}

fn source_path(root: &Path, file: &Path) -> String {
    file.strip_prefix(root)
        .unwrap_or(file)
        .to_string_lossy()
        .replace('\\', "/")
}

fn load_test_modules(root: &Path) -> Result<Vec<TestSourceModule>, String> {
    let mut modules = Vec::new();
    let mut visited = HashSet::new();
    load_test_module_file(
        root,
        &root.join("src/main.rs"),
        Vec::new(),
        root.join("src"),
        false,
        &mut visited,
        &mut modules,
    )?;
    let tests = root.join("tests");
    if tests.is_dir() {
        for entry in
            fs::read_dir(&tests).map_err(|error| format!("{}: {error}", tests.display()))?
        {
            let file = entry
                .map_err(|error| format!("{}: {error}", tests.display()))?
                .path();
            if file.extension().is_some_and(|extension| extension == "rs") {
                let stem = file
                    .file_stem()
                    .expect("Rust file has stem")
                    .to_string_lossy();
                load_test_module_file(
                    root,
                    &file,
                    Vec::new(),
                    tests.join(stem.as_ref()),
                    true,
                    &mut visited,
                    &mut modules,
                )?;
            }
        }
    }
    modules.sort_by(|left, right| left.source.path.cmp(&right.source.path));
    Ok(modules)
}

fn load_test_module_file(
    root: &Path,
    file: &Path,
    module: Vec<String>,
    child_dir: PathBuf,
    test_side: bool,
    visited: &mut HashSet<PathBuf>,
    modules: &mut Vec<TestSourceModule>,
) -> Result<(), String> {
    if !visited.insert(file.to_path_buf()) {
        return Ok(());
    }
    let source =
        fs::read_to_string(file).map_err(|error| format!("{}: {error}", file.display()))?;
    let syntax =
        syn::parse_file(&source).map_err(|error| format!("{}: {error}", file.display()))?;
    discover_test_modules(
        root,
        &syntax.items,
        &module,
        &child_dir,
        test_side,
        visited,
        modules,
    )?;
    modules.push(TestSourceModule {
        source: SourceModule {
            path: source_path(root, file),
            module,
            syntax,
        },
        test_side,
    });
    Ok(())
}

fn discover_test_modules(
    root: &Path,
    items: &[Item],
    module: &[String],
    child_dir: &Path,
    inherited_test_side: bool,
    visited: &mut HashSet<PathBuf>,
    modules: &mut Vec<TestSourceModule>,
) -> Result<(), String> {
    for item in items {
        let Item::Mod(item_mod) = item else { continue };
        let mut nested_module = module.to_vec();
        nested_module.push(item_mod.ident.to_string());
        let child_test_side = inherited_test_side || is_test_only(&item_mod.attrs);
        let Some((_, nested_items)) = &item_mod.content else {
            let stem = item_mod.ident.to_string();
            let flat = child_dir.join(format!("{stem}.rs"));
            let nested = child_dir.join(&stem).join("mod.rs");
            let (file, next_child_dir) = if flat.is_file() {
                (flat, child_dir.join(stem))
            } else if nested.is_file() {
                (nested, child_dir.join(stem))
            } else {
                return Err(format!(
                    "cannot resolve external module {} declared below {}",
                    item_mod.ident,
                    child_dir.display()
                ));
            };
            load_test_module_file(
                root,
                &file,
                nested_module,
                next_child_dir,
                child_test_side,
                visited,
                modules,
            )?;
            continue;
        };
        discover_test_modules(
            root,
            nested_items,
            &nested_module,
            &child_dir.join(item_mod.ident.to_string()),
            child_test_side,
            visited,
            modules,
        )?;
    }
    Ok(())
}

fn module_imports(items: &[Item], module: &[String], out: &mut BTreeMap<Vec<String>, Vec<Import>>) {
    let imports = out.entry(module.to_vec()).or_default();
    for item in items {
        if let Item::Use(item_use) = item {
            flatten_use_tree(&item_use.tree, Vec::new(), module, imports);
        }
    }
    for item in items {
        if let Item::Mod(item_mod) = item
            && let Some((_, nested)) = &item_mod.content
        {
            let mut child = module.to_vec();
            child.push(item_mod.ident.to_string());
            module_imports(nested, &child, out);
        }
    }
}

fn has_test_attr(attrs: &[Attribute]) -> bool {
    attrs.iter().any(|attr| attr.path().is_ident("test"))
}

fn serial_attribute(
    attrs: &[Attribute],
    aliases: &HashMap<String, Alias>,
) -> Option<Option<String>> {
    for attr in attrs {
        let raw = path_segments(attr.path());
        let recognized = raw == ["serial_test", "serial"]
            || (raw == ["serial"]
                && aliases.get("serial").is_some_and(|alias| {
                    resolve_path(&alias.raw, &alias.module, aliases) == ["serial_test", "serial"]
                }));
        if recognized {
            if matches!(&attr.meta, Meta::Path(_)) {
                return Some(None);
            } else {
                return Some(
                    attr.parse_args::<syn::Ident>()
                        .ok()
                        .map(|key| key.to_string()),
                );
            }
        }
    }
    None
}

struct CwdBodyVisitor<'a> {
    module: &'a [String],
    aliases: &'a HashMap<String, Alias>,
    calls: Vec<Vec<String>>,
    mutations: Vec<CwdEvent>,
    guards: Vec<CwdEvent>,
    drops: Vec<CwdEvent>,
    env_mutations: Vec<CwdEvent>,
    unsafe_guard: bool,
    scopes: Vec<usize>,
    next_scope: usize,
    next_order: usize,
}

impl CwdBodyVisitor<'_> {
    fn event(&mut self, span: Span, name: Option<String>) -> CwdEvent {
        self.next_order += 1;
        CwdEvent {
            line: location(span),
            order: self.next_order,
            scope: self.scopes.clone(),
            name,
        }
    }

    fn resolved(&self, path: &syn::Path) -> Vec<String> {
        resolve_path(&path_segments(path), self.module, self.aliases)
    }

    fn is_guard_new(&self, expression: &syn::Expr) -> bool {
        let syn::Expr::Call(call) = expression else {
            return false;
        };
        let syn::Expr::Path(path) = &*call.func else {
            return false;
        };
        has_suffix(&self.resolved(&path.path), "CwdGuard::new")
    }

    fn local_name(local: &syn::Local) -> Option<String> {
        let syn::Pat::Ident(ident) = &local.pat else {
            return None;
        };
        Some(ident.ident.to_string())
    }
}

impl<'ast> Visit<'ast> for CwdBodyVisitor<'_> {
    fn visit_block(&mut self, block: &'ast syn::Block) {
        self.next_scope += 1;
        self.scopes.push(self.next_scope);
        visit::visit_block(self, block);
        self.scopes.pop();
    }

    fn visit_local(&mut self, local: &'ast syn::Local) {
        if let (Some(name), Some(init)) = (Self::local_name(local), &local.init)
            && (name.starts_with("guard") || name.starts_with("_guard"))
            && self.is_guard_new(&init.expr)
        {
            let event = self.event(local.span(), Some(name));
            self.guards.push(event);
        }
        visit::visit_local(self, local);
    }

    fn visit_expr_call(&mut self, expression: &'ast syn::ExprCall) {
        if let syn::Expr::Path(path) = &*expression.func {
            let resolved = self.resolved(&path.path);
            if resolved == ["std", "env", "set_current_dir"] {
                let event = self.event(path.span(), None);
                self.mutations.push(event);
            } else if resolved == ["std", "env", "set_var"]
                || resolved == ["std", "env", "remove_var"]
            {
                let event = self.event(path.span(), None);
                self.env_mutations.push(event);
            } else if resolved.last().is_some_and(|name| name == "drop")
                || has_suffix(&resolved, "mem::forget")
            {
                let name = expression.args.first().and_then(|argument| match argument {
                    syn::Expr::Path(path) => path.path.get_ident().map(ToString::to_string),
                    _ => None,
                });
                if has_suffix(&resolved, "mem::forget") {
                    self.unsafe_guard = true;
                }
                let event = self.event(path.span(), name);
                self.drops.push(event);
            } else {
                self.calls.push(resolved);
            }
        }
        visit::visit_expr_call(self, expression);
    }

    fn visit_macro(&mut self, item: &'ast syn::Macro) {
        // `syn` keeps macro input opaque. Assertion and collection macros often
        // contain the only call to a test helper, so recover ordinary
        // comma-separated expressions when their token stream permits it.
        if let Ok(expressions) = item.parse_body_with(
            syn::punctuated::Punctuated::<syn::Expr, syn::Token![,]>::parse_terminated,
        ) {
            for expression in expressions {
                self.visit_expr(&expression);
            }
        }
    }

    fn visit_type_path(&mut self, ty: &'ast syn::TypePath) {
        if has_suffix(&self.resolved(&ty.path), "ManuallyDrop") {
            self.unsafe_guard = true;
        }
        visit::visit_type_path(self, ty);
    }
}

fn collect_cwd_functions(
    source: &TestSourceModule,
    imports: &BTreeMap<Vec<String>, Vec<Import>>,
    items: &[Item],
    module: &[String],
    inherited_test_side: bool,
    impl_name: Option<&str>,
    out: &mut Vec<CwdFunction>,
) {
    for item in items {
        match item {
            Item::Fn(function) => {
                let aliases = aliases_for(imports.get(module).map(Vec::as_slice).unwrap_or(&[]));
                let test_side = inherited_test_side
                    || has_test_attr(&function.attrs)
                    || is_test_only(&function.attrs);
                if !test_side {
                    continue;
                }
                let name = impl_name
                    .map(|owner| format!("{owner}::{}", function.sig.ident))
                    .unwrap_or_else(|| function.sig.ident.to_string());
                let mut visitor = CwdBodyVisitor {
                    module,
                    aliases: &aliases,
                    calls: Vec::new(),
                    mutations: Vec::new(),
                    guards: Vec::new(),
                    drops: Vec::new(),
                    env_mutations: Vec::new(),
                    unsafe_guard: false,
                    scopes: Vec::new(),
                    next_scope: 0,
                    next_order: 0,
                };
                visitor.visit_block(&function.block);
                out.push(CwdFunction {
                    identity: test_identity(&source.source.path, module, &name),
                    path: source.source.path.clone(),
                    module: module.to_vec(),
                    name,
                    line: location(function.sig.ident.span()),
                    is_test: has_test_attr(&function.attrs),
                    serial_key: serial_attribute(&function.attrs, &aliases),
                    calls: visitor.calls,
                    mutations: visitor.mutations,
                    guards: visitor.guards,
                    drops: visitor.drops,
                    unsafe_guard: visitor.unsafe_guard,
                    env_mutations: visitor.env_mutations,
                });
            }
            Item::Mod(item_mod) => {
                if let Some((_, nested)) = &item_mod.content {
                    let mut child = module.to_vec();
                    child.push(item_mod.ident.to_string());
                    collect_cwd_functions(
                        source,
                        imports,
                        nested,
                        &child,
                        inherited_test_side || is_test_only(&item_mod.attrs),
                        None,
                        out,
                    );
                }
            }
            Item::Impl(item_impl) => {
                let owner = match &*item_impl.self_ty {
                    syn::Type::Path(path) => path
                        .path
                        .segments
                        .last()
                        .map(|segment| segment.ident.to_string()),
                    _ => None,
                };
                for impl_item in &item_impl.items {
                    if let syn::ImplItem::Fn(function) = impl_item {
                        let aliases =
                            aliases_for(imports.get(module).map(Vec::as_slice).unwrap_or(&[]));
                        let test_side = inherited_test_side
                            || has_test_attr(&function.attrs)
                            || is_test_only(&function.attrs);
                        if !test_side {
                            continue;
                        }
                        let Some(owner) = &owner else { continue };
                        let name = format!("{owner}::{}", function.sig.ident);
                        let mut visitor = CwdBodyVisitor {
                            module,
                            aliases: &aliases,
                            calls: Vec::new(),
                            mutations: Vec::new(),
                            guards: Vec::new(),
                            drops: Vec::new(),
                            env_mutations: Vec::new(),
                            unsafe_guard: false,
                            scopes: Vec::new(),
                            next_scope: 0,
                            next_order: 0,
                        };
                        visitor.visit_block(&function.block);
                        out.push(CwdFunction {
                            identity: test_identity(&source.source.path, module, &name),
                            path: source.source.path.clone(),
                            module: module.to_vec(),
                            name,
                            line: location(function.sig.ident.span()),
                            is_test: has_test_attr(&function.attrs),
                            serial_key: serial_attribute(&function.attrs, &aliases),
                            calls: visitor.calls,
                            mutations: visitor.mutations,
                            guards: visitor.guards,
                            drops: visitor.drops,
                            unsafe_guard: visitor.unsafe_guard,
                            env_mutations: visitor.env_mutations,
                        });
                    }
                }
            }
            _ => {}
        }
    }
}

fn cwd_inventory(root: &Path) -> Result<Vec<CwdFunction>, String> {
    let mut functions = Vec::new();
    for source in load_test_modules(root)? {
        let mut imports = BTreeMap::new();
        module_imports(
            &source.source.syntax.items,
            &source.source.module,
            &mut imports,
        );
        collect_cwd_functions(
            &source,
            &imports,
            &source.source.syntax.items,
            &source.source.module,
            source.test_side,
            None,
            &mut functions,
        );
    }
    functions.sort_by(|left, right| left.identity.cmp(&right.identity));
    Ok(functions)
}

fn function_symbol(function: &CwdFunction) -> Vec<String> {
    let mut symbol = function.module.clone();
    symbol.extend(function.name.split("::").map(ToString::to_string));
    symbol
}

fn resolve_function_call(
    function: &CwdFunction,
    call: &[String],
    symbols: &HashMap<Vec<String>, usize>,
) -> Option<usize> {
    let mut candidate = function.module.clone();
    candidate.extend_from_slice(call);
    symbols
        .get(&candidate)
        .copied()
        .or_else(|| symbols.get(call).copied())
}

fn guard_is_live(function: &CwdFunction, mutation: &CwdEvent) -> bool {
    function.guards.iter().any(|guard| {
        guard.order < mutation.order
            && is_prefix(&guard.scope, &mutation.scope)
            && !function.drops.iter().any(|drop| {
                drop.order > guard.order && drop.order < mutation.order && drop.name == guard.name
            })
    })
}

fn cwd_findings(functions: &[CwdFunction]) -> Vec<String> {
    let symbols = functions
        .iter()
        .enumerate()
        .map(|(index, function)| (function_symbol(function), index))
        .collect::<HashMap<_, _>>();
    let mut callers = vec![Vec::new(); functions.len()];
    for (caller, function) in functions.iter().enumerate() {
        for call in &function.calls {
            if let Some(callee) = resolve_function_call(function, call, &symbols) {
                callers[callee].push(caller);
            }
        }
    }
    let exception = "src/test_helpers.rs::test_helpers::CwdGuard::drop";
    let mut failures = Vec::new();
    let mut direct_mutators = Vec::new();
    for (index, function) in functions.iter().enumerate() {
        if function.identity == exception {
            if function.mutations.len() != 1 {
                failures.push(format!("{}:{}: cwd-test-policy: CwdGuard::drop must contain exactly one restoration set_current_dir call", function.path, function.line));
            }
            continue;
        }
        if !function.mutations.is_empty() {
            direct_mutators.push(index);
            for mutation in &function.mutations {
                if function.unsafe_guard || !guard_is_live(function, mutation) {
                    failures.push(format!(
                        "{}:{}: cwd-test-policy: set_current_dir occurs before a live CwdGuard binding in helper {}",
                        function.path, mutation.line, function.name
                    ));
                }
            }
        }
        for env in &function.env_mutations {
            failures.push(format!(
                "{}:{}: cwd-test-policy: test-side process environment mutation is forbidden in helper {}",
                function.path, env.line, function.name
            ));
        }
    }
    if functions
        .iter()
        .any(|function| function.path == "src/test_helpers.rs")
        && !functions
            .iter()
            .any(|function| function.identity == exception)
    {
        failures.push("src/test_helpers.rs: cwd-test-policy: required CwdGuard::drop restoration function is missing".to_string());
    }

    let allowed = UNREACHABLE_CWD_HELPERS
        .iter()
        .map(|(identity, _)| *identity)
        .collect::<HashSet<_>>();
    for (identity, reason) in UNREACHABLE_CWD_HELPERS {
        let count = UNREACHABLE_CWD_HELPERS
            .iter()
            .filter(|(other, _)| other == identity)
            .count();
        if count > 1 || reason.trim().is_empty() {
            failures.push(format!(
                "cwd-test-policy: invalid unreachable CWD helper allowlist entry {identity}"
            ));
        }
    }
    let mut required = vec![false; functions.len()];
    let mut queue = VecDeque::new();
    for &mutator in &direct_mutators {
        required[mutator] = true;
        queue.push_back(mutator);
    }
    while let Some(callee) = queue.pop_front() {
        for &caller in &callers[callee] {
            if !required[caller] {
                required[caller] = true;
                queue.push_back(caller);
            }
        }
    }
    let serial_keys = functions
        .iter()
        .enumerate()
        .filter_map(|(index, function)| {
            (required[index] && function.is_test).then_some(function.serial_key.clone())
        })
        .collect::<Vec<_>>();
    let distinct_keys = serial_keys
        .iter()
        .filter_map(|key| {
            key.as_ref()
                .map(|key| key.as_deref().unwrap_or("<default>"))
        })
        .collect::<BTreeSet<_>>();
    if serial_keys.iter().any(Option::is_none) || distinct_keys.len() > 1 {
        failures.push(
            "cwd-test-policy: CWD-mutating tests use mixed or missing serial_test lock keys"
                .to_string(),
        );
    }
    for (index, function) in functions.iter().enumerate() {
        if required[index] && function.is_test && function.serial_key.is_none() {
            let mut chain = VecDeque::from([(index, vec![index])]);
            let mut seen = HashSet::from([index]);
            let mut found = vec![index];
            while let Some((node, path)) = chain.pop_front() {
                if direct_mutators.contains(&node) {
                    found = path;
                    break;
                }
                for call in &functions[node].calls {
                    if let Some(next) = resolve_function_call(&functions[node], call, &symbols)
                        && seen.insert(next)
                    {
                        let mut next_path = path.clone();
                        next_path.push(next);
                        chain.push_back((next, next_path));
                    }
                }
            }
            failures.push(format!(
                "{}:{}: cwd-test-policy: test {} reaches CWD-mutating helper {} but lacks #[serial_test::serial] ({})",
                function.path, function.line, function.name,
                functions[*found.last().unwrap()].name,
                found.iter().map(|&node| functions[node].name.as_str()).collect::<Vec<_>>().join(" -> ")
            ));
        }
    }
    for &mutator in &direct_mutators {
        let mut queue = VecDeque::from([mutator]);
        let mut seen = HashSet::from([mutator]);
        let mut reachable = functions[mutator].is_test;
        while let Some(callee) = queue.pop_front() {
            for &caller in &callers[callee] {
                if functions[caller].is_test {
                    reachable = true;
                }
                if seen.insert(caller) {
                    queue.push_back(caller);
                }
            }
        }
        let allowlisted = allowed.contains(functions[mutator].identity.as_str());
        if !reachable && !allowlisted {
            failures.push(format!(
                    "{}:{}: cwd-test-policy: unreachable CWD-mutating helper {}; delete it or add a reason-bearing baseline",
                    functions[mutator].path, functions[mutator].line, functions[mutator].name
                ));
        } else if reachable && allowlisted {
            failures.push(format!(
                "cwd-test-policy: stale unreachable CWD helper allowlist entry {}",
                functions[mutator].identity
            ));
        }
    }
    for (identity, _) in UNREACHABLE_CWD_HELPERS {
        if !direct_mutators
            .iter()
            .any(|&mutator| functions[mutator].identity == *identity)
        {
            failures.push(format!(
                "cwd-test-policy: stale unreachable CWD helper allowlist entry {identity}"
            ));
        }
    }
    failures.sort();
    failures
}

fn cwd_fixture(source: &str) -> Vec<CwdFunction> {
    let source = TestSourceModule {
        source: SourceModule {
            path: "src/example.rs".to_string(),
            module: vec!["tests".to_string()],
            syntax: syn::parse_file(source).unwrap(),
        },
        test_side: true,
    };
    let mut imports = BTreeMap::new();
    module_imports(
        &source.source.syntax.items,
        &source.source.module,
        &mut imports,
    );
    let mut functions = Vec::new();
    collect_cwd_functions(
        &source,
        &imports,
        &source.source.syntax.items,
        &source.source.module,
        true,
        None,
        &mut functions,
    );
    functions
}

#[test]
fn process_global_test_state_is_guarded_and_serialized() {
    let functions = cwd_inventory(Path::new(env!("CARGO_MANIFEST_DIR"))).unwrap();
    let failures = cwd_findings(&functions);
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn cwd_policy_fixtures_cover_guards_calls_and_attributes() {
    let pass = cwd_fixture(
        r#"
        #[test] #[serial_test::serial]
        fn direct() { let _guard = CwdGuard::new(); std::env::set_current_dir(".").unwrap(); }
        fn helper() { let guard = CwdGuard::new(); std::env::set_current_dir(".").unwrap(); }
        #[test] #[serial_test::serial]
        fn through_helper() { helper(); }
        fn first() { second(); }
        fn second() { first(); let _guard = CwdGuard::new(); std::env::set_current_dir(".").unwrap(); }
        #[test] #[serial_test::serial]
        fn cycle() { first(); }
        use serial_test::serial;
        #[test] #[serial]
        fn imported_serial() { let _guard = CwdGuard::new(); std::env::set_current_dir(".").unwrap(); }
        // std::env::set_current_dir("ignored");
        const NOTE: &str = "std::env::set_current_dir(ignored)";
        "#,
    );
    assert!(cwd_findings(&pass).is_empty(), "{:?}", cwd_findings(&pass));

    for source in [
        "#[test] #[serial_test::serial] fn missing() { std::env::set_current_dir(\".\").unwrap(); }",
        "#[test] #[serial_test::serial] fn after() { std::env::set_current_dir(\".\").unwrap(); let _guard = CwdGuard::new(); }",
        "#[test] #[serial_test::serial] fn discarded() { CwdGuard::new(); std::env::set_current_dir(\".\").unwrap(); }",
        "#[test] #[serial_test::serial] fn dropped() { let _guard = CwdGuard::new(); std::env::set_current_dir(\".\").unwrap(); drop(_guard); std::env::set_current_dir(\".\").unwrap(); }",
        "use std::env::set_current_dir as cd; #[test] #[serial_test::serial] fn renamed() { cd(\".\").unwrap(); }",
    ] {
        let findings = cwd_findings(&cwd_fixture(source));
        assert!(
            findings
                .iter()
                .any(|finding| finding.contains("live CwdGuard")),
            "expected guard finding for {source}: {findings:?}"
        );
    }

    let non_serial = cwd_fixture(
        r#"
        fn run_in() { let _guard = CwdGuard::new(); std::env::set_current_dir(".").unwrap(); }
        #[test] fn foo() { assert!({ run_in(); true }); }
        "#,
    );
    let findings = cwd_findings(&non_serial);
    assert!(
        findings
            .iter()
            .any(|finding| finding.contains("foo -> run_in")),
        "{findings:?}"
    );

    let environment = cwd_fixture(
        r#"
        #[test] fn environment() { std::env::set_var("A", "B"); std::env::remove_var("A"); }
        "#,
    );
    let findings = cwd_findings(&environment);
    assert_eq!(
        findings
            .iter()
            .filter(|finding| finding.contains("environment mutation"))
            .count(),
        2,
        "{findings:?}"
    );

    let mixed_keys = cwd_fixture(
        r#"
        #[test] #[serial_test::serial] fn default_key() { let _guard = CwdGuard::new(); std::env::set_current_dir(".").unwrap(); }
        #[test] #[serial_test::serial(cwd)] fn named_key() { let _guard = CwdGuard::new(); std::env::set_current_dir(".").unwrap(); }
        "#,
    );
    assert!(
        cwd_findings(&mixed_keys)
            .iter()
            .any(|finding| finding.contains("mixed or missing"))
    );
}

#[test]
fn cwd_policy_loader_includes_external_test_modules() {
    let temp = tempfile::tempdir().unwrap();
    fs::create_dir(temp.path().join("src")).unwrap();
    fs::write(temp.path().join("src/main.rs"), "#[cfg(test)] mod tests;").unwrap();
    fs::write(
        temp.path().join("src/tests.rs"),
        "#[test] #[serial_test::serial] fn external() { let _guard = CwdGuard::new(); std::env::set_current_dir(\".\").unwrap(); }",
    ).unwrap();
    let functions = cwd_inventory(temp.path()).unwrap();
    assert!(
        functions.iter().any(|function| function.name == "external"),
        "{functions:#?}"
    );
}

#[test]
fn cwd_guard_drop_restoration_is_the_only_test_helper_exception() {
    let functions = cwd_inventory(Path::new(env!("CARGO_MANIFEST_DIR"))).unwrap();
    let drop = functions
        .iter()
        .find(|function| function.identity == "src/test_helpers.rs::test_helpers::CwdGuard::drop")
        .expect("CwdGuard::drop inventory entry");
    assert_eq!(drop.mutations.len(), 1);
    let findings = cwd_findings(&functions);
    assert!(findings.is_empty(), "{findings:#?}");
}

fn fixture(source: &str, path: &str, module: &[&str]) -> BTreeSet<Finding> {
    let source = SourceModule {
        path: path.to_string(),
        module: module.iter().map(|part| (*part).to_string()).collect(),
        syntax: syn::parse_file(source).unwrap(),
    };
    inspect_module(&source)
}

#[test]
fn production_validators_do_not_control_process_or_mutate_files() {
    let mut exception_paths = HashSet::new();
    for (path, reason) in VALIDATOR_IO_WRITE_IMPORT_EXCEPTIONS {
        assert!(
            exception_paths.insert(*path),
            "duplicate I/O Write exception: {path}"
        );
        assert!(
            !reason.trim().is_empty(),
            "I/O Write exception {path} needs a reason"
        );
    }
    let findings = inspect_tree(Path::new(env!("CARGO_MANIFEST_DIR"))).unwrap();
    let failures = findings
        .iter()
        .filter(|finding| finding.kind == "validator-purity")
        .map(Finding::render)
        .collect::<Vec<_>>();
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn walkdir_is_owned_only_by_shared_traversal() {
    let findings = inspect_tree(Path::new(env!("CARGO_MANIFEST_DIR"))).unwrap();
    let failures = findings
        .iter()
        .filter(|finding| finding.kind == "traversal-ownership")
        .map(Finding::render)
        .collect::<Vec<_>>();
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn validator_peer_dependencies_match_the_reasoned_inventory() {
    let edges = validator_peer_edges(Path::new(env!("CARGO_MANIFEST_DIR"))).unwrap();
    let errors = inventory_errors(&edges);
    assert!(errors.is_empty(), "{}", errors.join("\n"));
}

#[test]
fn purity_fixture_rejects_every_forbidden_spelling_and_ignores_test_code() {
    let source = r#"
        use std::env::remove_var;
        use std::fs::{write as save, *};
        use std::io::Write;
        use std::process::Command as Cmd;
        fn production(mut file: std::fs::File) {
            // println!("ignored comment");
            let _ = "std::fs::remove_file(ignored string)";
            println!("x"); print!("x"); std::eprintln!("x"); eprint!("x"); dbg!(1); format!("ok");
            std::process::exit(1); std::process::abort();
            std::process::Command::new("x"); Cmd::new("x");
            std::env::set_current_dir("."); std::env::set_var("A", "B"); remove_var("A");
            std::fs::write("a", "b"); save("a", "b"); std::fs::copy("a", "b");
            std::fs::hard_link("a", "b"); std::fs::rename("a", "b"); std::fs::remove_file("a");
            std::fs::remove_dir("a"); std::fs::remove_dir_all("a"); std::fs::set_permissions("a", p);
            std::fs::File::create("a"); std::fs::File::options(); std::fs::OpenOptions::new();
            file.set_len(0); file.set_permissions(p); file.write_all(b"x"); file.write_fmt(args);
        }
        #[cfg(test)] fn test_function() { println!("ignored"); std::fs::write("a", "b"); }
        #[cfg(any(test, feature = "x"))] mod test_branch { fn f() { std::process::exit(1); } }
        #[cfg(not(test))] fn production_branch() { std::fs::remove_file("a"); }
        #[cfg(test)] mod tests { fn f() { std::fs::write("a", "b"); } }
        struct Example;
        impl Example {
            #[cfg(test)] fn test_method() { std::fs::write("a", "b"); }
        }
    "#;
    let findings = fixture(
        source,
        "src/validators/example.rs",
        &["validators", "example"],
    );
    let rendered = findings
        .iter()
        .map(|finding| finding.rendered_path.as_str())
        .collect::<Vec<_>>();
    for expected in [
        "println",
        "print",
        "std::eprintln",
        "eprint",
        "dbg",
        "std::process::exit",
        "std::process::abort",
        "std::process::Command::new",
        "std::env::set_current_dir",
        "std::env::set_var",
        "std::env::remove_var",
        "std::fs::write",
        "std::fs::copy",
        "std::fs::hard_link",
        "std::fs::rename",
        "std::fs::remove_file",
        "std::fs::remove_dir",
        "std::fs::remove_dir_all",
        "std::fs::set_permissions",
        "std::fs::File::create",
        "std::fs::File::options",
        "std::fs::OpenOptions::new",
        "set_len",
        "set_permissions",
        "write_all",
        "write_fmt",
        "std::io::Write",
        "std::fs",
    ] {
        assert!(
            rendered.contains(&expected),
            "missing {expected}: {rendered:?}"
        );
    }
    assert_eq!(
        rendered
            .iter()
            .filter(|path| **path == "std::fs::remove_file")
            .count(),
        2
    );
}

#[test]
fn walkdir_fixture_covers_import_alias_path_and_owner() {
    let forbidden = fixture(
        r#"
            use walkdir::WalkDir;
            use walkdir::WalkDir as Walker;
            fn f(_: walkdir::DirEntry) { WalkDir::new("."); Walker::new("."); }
            // walkdir::WalkDir::new("ignored");
        "#,
        "src/other.rs",
        &["other"],
    );
    assert!(forbidden.len() >= 4, "{forbidden:#?}");
    let owner = fixture(
        "use walkdir::WalkDir; fn f() { walkdir::WalkDir::new(\".\"); }",
        "src/traversal.rs",
        &["traversal"],
    );
    assert!(owner.is_empty(), "{owner:#?}");
}

#[test]
fn peer_dependency_fixture_resolves_import_aliases_and_super_paths() {
    let source = SourceModule {
        path: "src/validators/source.rs".to_string(),
        module: vec!["validators".to_string(), "source".to_string()],
        syntax: syn::parse_file(
            r#"
                use crate::validators::target::Thing as Alias;
                use crate::validators::common::Helper;
                fn production(_: Alias) {
                    super::sibling::check();
                    let _ = Helper::default();
                }
                #[cfg(test)] fn test_only() { crate::validators::ignored::check(); }
            "#,
        )
        .unwrap(),
    };
    let edges = peer_edges(&source);
    assert_eq!(
        edges
            .keys()
            .map(|(source, target)| (source.as_str(), target.as_str()))
            .collect::<Vec<_>>(),
        [("source", "sibling"), ("source", "target")]
    );
    assert_eq!(
        edges
            .get(&("source".to_string(), "target".to_string()))
            .unwrap()
            .line,
        2
    );
}

#[test]
fn peer_dependency_inventory_reports_new_stale_and_invalid_rows() {
    let edges = BTreeMap::from([(
        ("source".to_string(), "target".to_string()),
        PeerEdge {
            source: "source".to_string(),
            target: "target".to_string(),
            path: "src/validators/source.rs".to_string(),
            line: 42,
        },
    )]);
    let errors = inventory_errors_for(
        &edges,
        &[("common", ""), ("common", "duplicate")],
        &[
            ("source", "source", ""),
            ("source", "common", "must be primitive"),
            ("source", "common", "duplicate row"),
        ],
    );
    assert!(
        errors.iter().any(|error| error == "new validator peer edge: source -> target (first seen at src/validators/source.rs:42); route through validators/mod.rs or add a narrowly justified edge"),
        "{errors:#?}"
    );
    assert!(
        errors.iter().any(|error| error
            == "stale validator peer edge: source -> common; remove its allowlist row"),
        "{errors:#?}"
    );
    for expected in [
        "duplicate validator primitive: common",
        "validator primitive common needs a reason",
        "validator peer edge source -> source needs a reason",
        "validator peer edge cannot be self-referential: source",
        "validator peer edge source -> common targets a shared primitive",
        "duplicate validator peer edge: source -> common",
    ] {
        assert!(errors.iter().any(|error| error == expected), "{errors:#?}");
    }
}

#[test]
fn module_loader_skips_external_test_modules_inherited_from_the_declaration() {
    let temp = tempfile::tempdir().unwrap();
    fs::create_dir(temp.path().join("src")).unwrap();
    fs::write(
        temp.path().join("src/main.rs"),
        "mod production; #[cfg(test)] mod hidden;",
    )
    .unwrap();
    fs::write(temp.path().join("src/production.rs"), "fn f() {} ").unwrap();
    fs::write(
        temp.path().join("src/hidden.rs"),
        "fn f() { println!(\"not parsed as production\"); }",
    )
    .unwrap();
    let loaded = load_production_modules(temp.path()).unwrap();
    assert_eq!(
        loaded
            .iter()
            .map(|module| module.path.as_str())
            .collect::<Vec<_>>(),
        ["src/main.rs", "src/production.rs"]
    );
}
