//! AST-backed architectural checks over production Rust source.
//!
//! This module is test-only so `syn` does not enter the shipped dependency
//! graph. The loader follows the same ordinary module-file layout as rustc and
//! deliberately excludes every item that is positively gated on `test`.

use proc_macro2::Span;
use std::collections::{BTreeSet, HashMap, HashSet};
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

struct SourceModule {
    path: String,
    module: Vec<String>,
    syntax: syn::File,
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
