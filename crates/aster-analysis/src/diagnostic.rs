use std::path::PathBuf;

use serde::Serialize;

/// The category of a [`Diagnostic`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticKind {
    /// A file could not be read.
    IoError,
    /// A file failed to parse.
    ParseError,
    /// A `require("...")` module name resolved to no file.
    UnresolvedRequire,
    /// A `require(expr)` call with a non-literal argument.
    DynamicRequire,
    /// A cycle in the require graph.
    CircularDependency,
    /// A module that no other module requires (entry points excluded).
    UnusedModule,
    /// `t[0]` on a table with sequence intent.
    ZeroIndexAccess,
    /// `for i = 0, #t` or `for i = 1, #t - 1` over a sequence.
    OffByOneLoop,
    /// Non-contiguous explicit integer keys (`t[1] = ...; t[3] = ...`).
    SparseArray,
    /// `#t` on a table known same-file to have holes.
    AmbiguousLength,
    /// Call-site return-value truncation or nil-padding made visible.
    MultiReturnInfo,
    /// `mod.member` access where the required module's export shape is
    /// fully known and has no such member.
    UnknownMember,
    /// Repeated global or library table lookup inside a loop without local caching.
    GlobalInLoop,
    /// String concatenation (`..`) inside a loop causing repeated GC allocations.
    StringConcatInLoop,
    /// Table constructor (`{}`) allocated inside a loop.
    TableAllocationInLoop,
}

/// One finding from analyzing a project.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Diagnostic {
    pub kind: DiagnosticKind,
    /// File the diagnostic points at, relative to the project root, if any.
    pub file: Option<PathBuf>,
    /// 1-based line, if any.
    pub line: Option<usize>,
    /// 1-based column, if any.
    pub column: Option<usize>,
    pub message: String,
}

impl Diagnostic {
    pub fn new(kind: DiagnosticKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            file: None,
            line: None,
            column: None,
            message: message.into(),
        }
    }

    /// Attach a file location.
    pub fn in_file(mut self, file: PathBuf) -> Self {
        self.file = Some(file);
        self
    }

    /// Attach a file and 1-based line/column location.
    pub fn at(mut self, file: PathBuf, line: usize, column: usize) -> Self {
        self.file = Some(file);
        self.line = Some(line);
        self.column = Some(column);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builders_attach_location() {
        let diagnostic = Diagnostic::new(DiagnosticKind::UnusedModule, "never required")
            .in_file(PathBuf::from("orphan.lua"));
        assert_eq!(diagnostic.file, Some(PathBuf::from("orphan.lua")));
        assert_eq!(diagnostic.line, None);

        let diagnostic = Diagnostic::new(
            DiagnosticKind::UnresolvedRequire,
            "unresolved require 'ghost'",
        )
        .at(PathBuf::from("main.lua"), 1, 15);
        assert_eq!(diagnostic.file, Some(PathBuf::from("main.lua")));
        assert_eq!(diagnostic.line, Some(1));
        assert_eq!(diagnostic.column, Some(15));
    }
}
