//! Shared project loading: discover, read, and parse each Lua file once,
//! so `analyze()` and `check()` never parse the same file twice.

use std::path::PathBuf;

use crate::AnalysisOptions;
use crate::diagnostic::{Diagnostic, DiagnosticKind};

/// A discovered Lua file and its parse result.
pub(crate) struct ParsedFile {
    /// Path relative to the project root.
    pub path: PathBuf,
    /// The parsed AST, or `None` if the file could not be read or parsed
    /// (a diagnostic is emitted either way).
    pub ast: Option<full_moon::ast::Ast>,
}

/// Discover, read, and parse all Lua files under `options.root`.
/// Unreadable and unparseable files are still listed (with `ast: None`)
/// and produce `IoError` / `ParseError` diagnostics.
pub(crate) fn load_project(options: &AnalysisOptions) -> (Vec<ParsedFile>, Vec<Diagnostic>) {
    let mut files = Vec::new();
    let mut diagnostics = Vec::new();
    for path in crate::discover::discover_lua_files(&options.root) {
        let source = match std::fs::read_to_string(options.root.join(&path)) {
            Ok(source) => source,
            Err(error) => {
                diagnostics.push(
                    Diagnostic::new(
                        DiagnosticKind::IoError,
                        format!("cannot read file: {error}"),
                    )
                    .in_file(path.clone()),
                );
                files.push(ParsedFile { path, ast: None });
                continue;
            }
        };
        match full_moon::parse(&source) {
            Ok(ast) => files.push(ParsedFile {
                path,
                ast: Some(ast),
            }),
            Err(errors) => {
                let message = errors
                    .into_iter()
                    .next()
                    .map(|error| error.to_string())
                    .unwrap_or_else(|| "parse error".to_string());
                diagnostics.push(
                    Diagnostic::new(DiagnosticKind::ParseError, message).in_file(path.clone()),
                );
                files.push(ParsedFile { path, ast: None });
            }
        }
    }
    (files, diagnostics)
}
