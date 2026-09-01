use full_moon::ast::{Call, Expression, FunctionArgs, Prefix, Suffix};
use full_moon::tokenizer::{TokenReference, TokenType};
use full_moon::visitors::Visitor;

/// A static `require("...")` call site.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequireCall {
    /// Module name exactly as written, e.g. `"a.b.c"`. Escape sequences are
    /// NOT unescaped in v1.
    pub name: String,
    /// 1-based line of the `require` identifier.
    pub line: usize,
    /// 1-based column of the `require` identifier.
    pub column: usize,
}

/// All `require` calls found in one Lua source file.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct FileRequires {
    pub static_requires: Vec<RequireCall>,
    /// `(line, column)` of `require(expr)` calls whose argument is not a
    /// string literal.
    pub dynamic_requires: Vec<(usize, usize)>,
}

/// Extract all `require` calls from Lua source.
///
/// Static requires (string-literal argument: `require("m")`, `require "m"`,
/// `require[[m]]`) are recorded with the 1-based position of the `require`
/// identifier. Requires with any other argument are recorded as dynamic.
// Kept as the source-based wrapper per the v2 plan; since the `requires`
// module is crate-private and `analyze()` now uses `extract_requires_ast`,
// its only callers are the unit tests below.
#[allow(dead_code)]
pub fn extract_requires(source: &str) -> Result<FileRequires, Box<full_moon::Error>> {
    // full_moon 2.2 reports all parse errors at once; surface the first one.
    let ast = full_moon::parse(source).map_err(|mut errors| Box::new(errors.swap_remove(0)))?;
    Ok(extract_requires_ast(&ast))
}

/// Extract all `require` calls from an already-parsed AST.
pub(crate) fn extract_requires_ast(ast: &full_moon::ast::Ast) -> FileRequires {
    let mut visitor = RequireVisitor::default();
    visitor.visit_ast(ast);
    visitor.result
}

#[derive(Default)]
struct RequireVisitor {
    result: FileRequires,
}

impl Visitor for RequireVisitor {
    fn visit_function_call(&mut self, call: &full_moon::ast::FunctionCall) {
        // Only the FIRST suffix belongs to the prefix (`require`): later
        // suffixes like `require("a").setup({})` call methods on the result.
        if let Some(Suffix::Call(Call::AnonymousCall(args))) = call.suffixes().next() {
            self.handle_call(call.prefix(), args);
        }
    }
}

impl RequireVisitor {
    fn handle_call(&mut self, prefix: &Prefix, args: &FunctionArgs) {
        // Only bare `require(...)` — `foo.require(...)` has prefix `foo`.
        let Prefix::Name(name_ref) = prefix else {
            return;
        };
        let name_token = name_ref.token();
        let TokenType::Identifier { identifier } = name_token.token_type() else {
            return;
        };
        if identifier.as_str() != "require" {
            return;
        }

        let position = name_token.start_position();
        let (line, column) = (position.line(), position.character());

        match args {
            FunctionArgs::String(token_ref) => {
                if let Some(name) = string_literal_value(token_ref) {
                    self.result
                        .static_requires
                        .push(RequireCall { name, line, column });
                }
            }
            FunctionArgs::Parentheses { arguments, .. } => match arguments.iter().next() {
                Some(Expression::String(token_ref)) => {
                    if let Some(name) = string_literal_value(token_ref) {
                        self.result
                            .static_requires
                            .push(RequireCall { name, line, column });
                    }
                }
                _ => self.result.dynamic_requires.push((line, column)),
            },
            FunctionArgs::TableConstructor(_) => {
                self.result.dynamic_requires.push((line, column));
            }
            _ => {}
        }
    }
}

/// Extract the text of a string literal token. full_moon stores the literal
/// without its quotes / long brackets; escape sequences are left as-is (v1).
fn string_literal_value(token_ref: &TokenReference) -> Option<String> {
    let TokenType::StringLiteral { literal, .. } = token_ref.token().token_type() else {
        return None;
    };
    Some(literal.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_parenthesized_string_require() {
        let result = extract_requires("local p = require(\"player\")").unwrap();
        assert_eq!(
            result.static_requires,
            vec![RequireCall {
                name: "player".to_string(),
                line: 1,
                column: 11
            }]
        );
        assert!(result.dynamic_requires.is_empty());
    }

    #[test]
    fn extracts_bare_string_require() {
        let result = extract_requires("require \"network\"").unwrap();
        assert_eq!(result.static_requires[0].name, "network");
        assert_eq!(
            (
                result.static_requires[0].line,
                result.static_requires[0].column
            ),
            (1, 1)
        );
    }

    #[test]
    fn extracts_long_bracket_require() {
        let result = extract_requires("require[[save]]").unwrap();
        assert_eq!(result.static_requires[0].name, "save");
    }

    #[test]
    fn flags_dynamic_require() {
        let result = extract_requires("local m = \"x\"\nrequire(m)").unwrap();
        assert!(result.static_requires.is_empty());
        assert_eq!(result.dynamic_requires, vec![(2, 1)]);
    }

    #[test]
    fn ignores_non_require_calls() {
        let result = extract_requires("print(\"x\")\nfoo.require(\"y\")").unwrap();
        assert!(result.static_requires.is_empty());
        assert!(result.dynamic_requires.is_empty());
    }

    #[test]
    fn ignores_chained_calls_after_require() {
        let result = extract_requires("require(\"a\").setup({})").unwrap();
        assert_eq!(result.static_requires.len(), 1);
        assert_eq!(result.static_requires[0].name, "a");
        assert!(result.dynamic_requires.is_empty());
    }
}
