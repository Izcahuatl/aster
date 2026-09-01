//! UI-agnostic Lua project analysis: module graph, require resolution,
//! and Lua-specific diagnostics.

mod ast_util;
mod checks;
mod diagnostic;
mod discover;
mod explain;
mod exports;
mod graph;
mod project;
mod requires;
mod resolve;

use std::path::PathBuf;

pub use diagnostic::{Diagnostic, DiagnosticKind};
pub use explain::{LookupExplanation, LookupResult};
pub use graph::ModuleGraph;

/// Options for [`analyze`].
#[derive(Debug, Clone)]
pub struct AnalysisOptions {
    /// Project root directory.
    pub root: PathBuf,
    /// `package.path`-style patterns with `?` placeholders, relative to `root`.
    pub search_path: Vec<String>,
}

impl AnalysisOptions {
    /// Options for the project at `root` with the default search path
    /// `./?.lua;./?/init.lua`.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            search_path: vec!["./?.lua".to_string(), "./?/init.lua".to_string()],
        }
    }
}

/// Run Lua-specific checks (sequence diagnostics, multiple-returns
/// visibility, and unknown-member access on known module shapes) over the
/// project at `options.root`.
///
/// Returns diagnostics sorted by file, line, and column. IO and parse
/// failures are included as diagnostics.
pub fn check(options: &AnalysisOptions) -> Vec<Diagnostic> {
    let (files, mut diagnostics) = project::load_project(options);

    // Phase 1: per-module export shapes (each from its own AST alone).
    let mut shapes = std::collections::HashMap::new();
    for file in &files {
        if let Some(ast) = &file.ast
            && let Some(shape) = exports::module_export(ast)
        {
            shapes.insert(file.path.clone(), shape);
        }
    }
    let resolver = resolve::Resolver::new(options.root.clone(), options.search_path.clone());

    // Phase 2: resolve every file's module and instance bindings. Class
    // links are expressed in the owning module's local namespace, so lookup
    // needs this project-wide binding index.
    let mut all_bindings = std::collections::HashMap::new();
    for file in &files {
        if let Some(ast) = &file.ast {
            all_bindings.insert(
                file.path.clone(),
                checks::bindings::Bindings::collect(ast, &resolver, &shapes),
            );
        }
    }

    // Phase 3: per-file checks.
    for file in &files {
        let Some(ast) = &file.ast else { continue };
        diagnostics.extend(checks::sequence::check_file(&file.path, ast));
        diagnostics.extend(checks::performance::check_file(&file.path, ast));
        let Some(bindings) = all_bindings.get(&file.path) else {
            continue;
        };
        diagnostics.extend(checks::members::check_file(
            &file.path,
            ast,
            bindings,
            &shapes,
            &all_bindings,
        ));
        diagnostics.extend(checks::returns::check_file(
            &file.path,
            ast,
            bindings,
            &shapes,
            &all_bindings,
        ));
    }
    diagnostics.sort_by(|a, b| (&a.file, a.line, a.column).cmp(&(&b.file, b.line, b.column)));
    diagnostics
}

/// Explain each known module or instance member access on `line` in `file`.
/// `file` may be relative to `options.root` (the CLI form) or an absolute
/// path under that root. Unknown/dynamic links are returned as explanations,
/// never as errors.
pub fn explain(
    options: &AnalysisOptions,
    file: impl AsRef<std::path::Path>,
    line: usize,
) -> Vec<LookupExplanation> {
    let (files, _) = project::load_project(options);
    let mut shapes = std::collections::HashMap::new();
    for parsed in &files {
        if let Some(ast) = &parsed.ast
            && let Some(shape) = exports::module_export(ast)
        {
            shapes.insert(parsed.path.clone(), shape);
        }
    }
    let resolver = resolve::Resolver::new(options.root.clone(), options.search_path.clone());
    let mut all_bindings = std::collections::HashMap::new();
    for parsed in &files {
        if let Some(ast) = &parsed.ast {
            all_bindings.insert(
                parsed.path.clone(),
                checks::bindings::Bindings::collect(ast, &resolver, &shapes),
            );
        }
    }
    let requested_raw = file.as_ref().to_string_lossy().replace('\\', "/");
    let root_raw = options.root.to_string_lossy().replace('\\', "/");
    let relative_str = requested_raw
        .strip_prefix(&root_raw)
        .unwrap_or(&requested_raw)
        .trim_start_matches('/');

    let Some(parsed) = files.iter().find(|parsed| {
        let p_str = parsed.path.to_string_lossy().replace('\\', "/");
        p_str == relative_str
            || p_str == requested_raw
            || requested_raw.ends_with(&format!("/{}", p_str))
            || relative_str.ends_with(&p_str)
    }) else {
        return Vec::new();
    };
    let (Some(ast), Some(bindings)) = (&parsed.ast, all_bindings.get(&parsed.path)) else {
        return Vec::new();
    };
    explain::explain_file(ast, &parsed.path, line, bindings, &shapes, &all_bindings)
}

/// The result of analyzing a project.
pub struct AnalysisResult {
    pub graph: ModuleGraph,
    pub diagnostics: Vec<Diagnostic>,
}

/// Analyze the Lua project at `options.root`: discover files, extract
/// `require` calls, resolve them, and build the module graph with
/// diagnostics.
///
/// Never fails on malformed input — unreadable files, parse errors, and
/// unresolvable modules all become [`Diagnostic`]s, and the graph is built
/// from whatever succeeded. Entry points (any file named `main.lua` or
/// `init.lua`, at any depth) are exempt from the unused-module diagnostic.
pub fn analyze(options: &AnalysisOptions) -> AnalysisResult {
    let (files, mut diagnostics) = project::load_project(options);
    let resolver = resolve::Resolver::new(options.root.clone(), options.search_path.clone());

    let mut edges = Vec::new();

    for file in &files {
        let Some(ast) = &file.ast else { continue };
        let file_requires = requires::extract_requires_ast(ast);
        for call in file_requires.static_requires {
            match resolver.resolve(&call.name) {
                Some(target) => edges.push((file.path.clone(), target)),
                None => diagnostics.push(
                    Diagnostic::new(
                        DiagnosticKind::UnresolvedRequire,
                        format!("unresolved require '{}'", call.name),
                    )
                    .at(file.path.clone(), call.line, call.column),
                ),
            }
        }
        for (line, column) in file_requires.dynamic_requires {
            diagnostics.push(
                Diagnostic::new(
                    DiagnosticKind::DynamicRequire,
                    "dynamic require with non-literal argument",
                )
                .at(file.path.clone(), line, column),
            );
        }
    }

    let paths: Vec<PathBuf> = files.iter().map(|file| file.path.clone()).collect();
    let graph = ModuleGraph::build(&paths, &edges);

    for cycle in graph.cycles() {
        let mut names: Vec<String> = cycle.iter().map(|p| p.display().to_string()).collect();
        if let Some(first) = names.first().cloned() {
            names.push(first);
            diagnostics.push(Diagnostic::new(
                DiagnosticKind::CircularDependency,
                format!("circular dependency: {}", names.join(" -> ")),
            ));
        }
    }

    for entry in graph.entry_points() {
        let is_entry_point = matches!(
            entry.file_name().and_then(|name| name.to_str()),
            Some("main.lua" | "init.lua")
        );
        if !is_entry_point {
            diagnostics.push(
                Diagnostic::new(DiagnosticKind::UnusedModule, "module is never required")
                    .in_file(entry.to_path_buf()),
            );
        }
    }

    AnalysisResult { graph, diagnostics }
}
