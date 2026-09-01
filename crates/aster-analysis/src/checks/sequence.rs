//! Sequence heuristics: zero-based indexing, off-by-one loops, sparse
//! arrays, and ambiguous `#t`. Each diagnostic requires positive, same-file
//! evidence of sequence intent — no evidence, no warning.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use full_moon::ast::{
    Assignment, Ast, BinOp, Call, Expression, Field, FunctionArgs, FunctionCall, Index,
    LocalAssignment, NumericFor, Prefix, Suffix, UnOp, Var,
};
use full_moon::tokenizer::{Position, TokenReference, TokenType};
use full_moon::visitors::Visitor;

use crate::diagnostic::{Diagnostic, DiagnosticKind};

/// Run the sequence heuristics over one parsed file.
pub(crate) fn check_file(path: &Path, ast: &Ast) -> Vec<Diagnostic> {
    let mut visitor = SequenceVisitor::default();
    visitor.visit_ast(ast);
    visitor.into_diagnostics(path)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OffByOne {
    StartsAtZero,
    MissesLast,
}

#[derive(Default)]
struct SequenceVisitor {
    /// Numeric-`for` variable names currently in scope. The visitor walks in
    /// document order, so a stack matches loop nesting.
    loop_vars: Vec<String>,
    /// Tables with observed sequence intent: name -> first evidence position.
    intent: HashMap<String, (usize, usize)>,
    /// `t[0]` occurrences per table name: (line, column) of the `[`.
    zero_uses: HashMap<String, Vec<(usize, usize)>>,
    /// `#t` occurrences per table name: (line, column) of the `#`.
    length_uses: HashMap<String, Vec<(usize, usize)>>,
    /// Explicit integer keys from constructors and `t[k] = ...` assignments,
    /// per table name: (key, line, column).
    explicit_keys: HashMap<String, Vec<(i64, usize, usize)>>,
    /// (loop variable, table, line, column, kind).
    off_by_one: Vec<(String, String, usize, usize, OffByOne)>,
}

impl SequenceVisitor {
    fn add_intent(&mut self, table: String, line: usize, column: usize) {
        self.intent.entry(table).or_insert((line, column));
    }

    fn into_diagnostics(self, path: &Path) -> Vec<Diagnostic> {
        let mut diagnostics = Vec::new();

        // zero_index_access: t[0] with sequence intent
        for (table, uses) in &self.zero_uses {
            if !self.intent.contains_key(table) {
                continue;
            }
            for &(line, column) in uses {
                diagnostics.push(
                    Diagnostic::new(
                        DiagnosticKind::ZeroIndexAccess,
                        format!("zero-based index `{table}[0]` on sequence-like table `{table}`"),
                    )
                    .at(path.to_path_buf(), line, column),
                );
            }
        }

        // sparse_array: gaps between explicit integer keys
        let mut tables_with_holes: HashSet<&String> = HashSet::new();
        for (table, keys) in &self.explicit_keys {
            let mut sorted: Vec<&(i64, usize, usize)> = keys.iter().collect();
            sorted.sort_by_key(|(key, ..)| *key);
            for pair in sorted.windows(2) {
                let (previous, ..) = pair[0];
                let (current, line, column) = pair[1];
                if previous.checked_add(1).is_some_and(|next| *current > next) {
                    tables_with_holes.insert(table);
                    diagnostics.push(
                        Diagnostic::new(
                            DiagnosticKind::SparseArray,
                            format!(
                                "sparse array `{table}`: key {current} leaves a hole after key {previous}"
                            ),
                        )
                        .at(path.to_path_buf(), *line, *column),
                    );
                }
            }
        }

        // ambiguous_length: #t on a table with known holes
        for (table, uses) in &self.length_uses {
            if !tables_with_holes.contains(table) {
                continue;
            }
            for &(line, column) in uses {
                diagnostics.push(
                    Diagnostic::new(
                        DiagnosticKind::AmbiguousLength,
                        format!(
                            "`#{table}` on a table with holes has an implementation-defined result"
                        ),
                    )
                    .at(path.to_path_buf(), line, column),
                );
            }
        }

        // off_by_one_loop
        for (var, table, line, column, kind) in &self.off_by_one {
            let message = match kind {
                OffByOne::StartsAtZero => {
                    format!("loop `for {var} = 0, #{table}` starts at 0; Lua sequences start at 1")
                }
                OffByOne::MissesLast => {
                    format!("loop `for {var} = 1, #{table} - 1` skips the last element")
                }
            };
            diagnostics.push(Diagnostic::new(DiagnosticKind::OffByOneLoop, message).at(
                path.to_path_buf(),
                *line,
                *column,
            ));
        }

        diagnostics
    }
}

impl Visitor for SequenceVisitor {
    fn visit_local_assignment(&mut self, node: &LocalAssignment) {
        for (name_token, expr) in node.names().iter().zip(node.expressions().iter()) {
            let Some(name) = identifier_name(name_token) else {
                continue;
            };
            let Expression::TableConstructor(table) = expr else {
                continue;
            };
            let mut has_array_fields = false;
            for field in table.fields() {
                match field {
                    Field::NoKey(_) => has_array_fields = true,
                    Field::ExpressionKey { key, .. } => {
                        if let Some(key_number) = number_value(key)
                            && let Some((line, column)) = expression_position(key)
                        {
                            self.explicit_keys
                                .entry(name.clone())
                                .or_default()
                                .push((key_number, line, column));
                        }
                    }
                    _ => {}
                }
            }
            if has_array_fields {
                let position = name_token.start_position();
                self.add_intent(name, position.line(), position.character());
            }
        }
    }

    fn visit_assignment(&mut self, node: &Assignment) {
        // Writes `t[k] = v` with integer k establish explicit keys.
        // (Reads are deliberately excluded — they prove nothing about
        // construction.)
        for var in node.variables() {
            let Some((table, index_expr, position)) = bracket_index(var) else {
                continue;
            };
            if let Some(key) = number_value(index_expr) {
                self.explicit_keys.entry(table).or_default().push((
                    key,
                    position.line(),
                    position.character(),
                ));
            }
        }
    }

    fn visit_var(&mut self, var: &Var) {
        let Some((table, index_expr, position)) = bracket_index(var) else {
            return;
        };
        let (line, column) = (position.line(), position.character());
        if number_value(index_expr) == Some(0) {
            self.zero_uses
                .entry(table.clone())
                .or_default()
                .push((line, column));
        }
        if number_value(index_expr) == Some(1) {
            self.add_intent(table.clone(), line, column);
        }
        if let Some(index_name) = variable_name(index_expr)
            && self.loop_vars.contains(&index_name)
        {
            self.add_intent(table.clone(), line, column);
        }
        if is_append(index_expr, &table) {
            self.add_intent(table, line, column);
        }
    }

    fn visit_expression(&mut self, expr: &Expression) {
        if let Expression::UnaryOperator {
            unop: UnOp::Hash(token),
            expression,
        } = expr
            && let Some(name) = variable_name(expression)
        {
            let position = token.start_position();
            let (line, column) = (position.line(), position.character());
            self.length_uses
                .entry(name.clone())
                .or_default()
                .push((line, column));
            self.add_intent(name, line, column);
        }
    }

    fn visit_numeric_for(&mut self, node: &NumericFor) {
        let Some(var) = identifier_name(node.index_variable()) else {
            return;
        };
        self.loop_vars.push(var.clone());
        let position = node.index_variable().start_position();
        let (line, column) = (position.line(), position.character());
        // `for i = 0, #t` — starts at 0
        if number_value(node.start()) == Some(0)
            && let Some(table) = length_target(node.end())
        {
            self.off_by_one
                .push((var, table, line, column, OffByOne::StartsAtZero));
            return;
        }
        // `for i = 1, #t - 1` — misses the last element
        if number_value(node.start()) == Some(1)
            && let Expression::BinaryOperator {
                lhs,
                binop: BinOp::Minus(_),
                rhs,
            } = node.end()
            && number_value(rhs) == Some(1)
            && let Some(table) = length_target(lhs)
        {
            self.off_by_one
                .push((var, table, line, column, OffByOne::MissesLast));
        }
    }

    fn visit_numeric_for_end(&mut self, _node: &NumericFor) {
        self.loop_vars.pop();
    }

    fn visit_function_call(&mut self, call: &FunctionCall) {
        let Prefix::Name(name_ref) = call.prefix() else {
            return;
        };
        let Some(name) = identifier_name(name_ref) else {
            return;
        };
        if name == "ipairs" {
            if let Some((table, line, column)) = first_call_arg(call.suffixes().next()) {
                self.add_intent(table, line, column);
            }
        } else if name == "table" {
            let mut suffixes = call.suffixes();
            let Some(Suffix::Index(Index::Dot { name: member, .. })) = suffixes.next() else {
                return;
            };
            if identifier_name(member).as_deref() != Some("insert") {
                return;
            }
            if let Some((table, line, column)) = first_call_arg(suffixes.next()) {
                self.add_intent(table, line, column);
            }
        }
    }
}

/// If `var` is `name[expr]`, return the table name, the index expression,
/// and the position of the `[`.
fn bracket_index(var: &Var) -> Option<(String, &Expression, Position)> {
    let Var::Expression(var_expr) = var else {
        return None;
    };
    let Prefix::Name(name_ref) = var_expr.prefix() else {
        return None;
    };
    let table = identifier_name(name_ref)?;
    let Some(Suffix::Index(Index::Brackets {
        brackets,
        expression,
    })) = var_expr.suffixes().next()
    else {
        return None;
    };
    let position = brackets.tokens().0.start_position();
    Some((table, expression, position))
}

/// If `expr` is a plain variable reference, return its name.
fn variable_name(expr: &Expression) -> Option<String> {
    if let Expression::Var(Var::Name(token)) = expr {
        identifier_name(token)
    } else {
        None
    }
}

/// If `expr` is `#name`, return the name.
fn length_target(expr: &Expression) -> Option<String> {
    if let Expression::UnaryOperator {
        unop: UnOp::Hash(_),
        expression,
    } = expr
    {
        variable_name(expression)
    } else {
        None
    }
}

/// Is `expr` the shape `#t + 1` (the idiomatic append index) for `table`?
fn is_append(expr: &Expression, table: &str) -> bool {
    let Expression::BinaryOperator {
        lhs,
        binop: BinOp::Plus(_),
        rhs,
    } = expr
    else {
        return false;
    };
    if number_value(rhs) != Some(1) {
        return false;
    }
    matches!(length_target(lhs), Some(name) if name == table)
}

/// If `suffix` is a call whose first argument is a plain variable, return
/// its name and position.
fn first_call_arg(suffix: Option<&Suffix>) -> Option<(String, usize, usize)> {
    let Some(Suffix::Call(Call::AnonymousCall(FunctionArgs::Parentheses { arguments, .. }))) =
        suffix
    else {
        return None;
    };
    let first = arguments.iter().next()?;
    let name = variable_name(first)?;
    let (line, column) = expression_position(first)?;
    Some((name, line, column))
}

fn identifier_name(token: &TokenReference) -> Option<String> {
    if let TokenType::Identifier { identifier } = token.token().token_type() {
        Some(identifier.to_string())
    } else {
        None
    }
}

/// Plain decimal integer literal value (hex/float literals return None).
fn number_value(expr: &Expression) -> Option<i64> {
    if let Expression::Number(token) = expr
        && let TokenType::Number { text } = token.token().token_type()
    {
        let text = text.to_string();
        if text.bytes().all(|b| b.is_ascii_digit()) {
            return text.parse().ok();
        }
    }
    None
}

/// 1-based (line, column) of a simple expression's token.
fn expression_position(expr: &Expression) -> Option<(usize, usize)> {
    let token = match expr {
        Expression::Number(token) | Expression::String(token) | Expression::Symbol(token) => token,
        Expression::Var(Var::Name(token)) => token,
        _ => return None,
    };
    let position = token.start_position();
    Some((position.line(), position.character()))
}
