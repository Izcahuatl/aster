//! Multiple-returns visibility: when a function's return arity is known —
//! either a same-file definition or a module member function resolved
//! through require bindings — report call sites that silently discard
//! values or bind more variables than the function returns.
//!
//! Conservative by design: functions with returns in nested blocks
//! (conditional returns), tail calls, or varargs in the return list are
//! unknown arity, and unknown arity never produces a diagnostic.

use std::collections::HashMap;
use std::path::Path;

use full_moon::ast::punctuated::Punctuated;
use full_moon::ast::{
    Assignment, Ast, Call, Expression, FunctionCall, FunctionDeclaration, Index, LocalAssignment,
    LocalFunction, Prefix, Suffix, Var,
};
use full_moon::tokenizer::{TokenReference, TokenType};
use full_moon::visitors::Visitor;

use super::return_arity;
use crate::checks::bindings::Bindings;
use crate::checks::classes::{LookupContext, LookupTarget};
use crate::diagnostic::{Diagnostic, DiagnosticKind};
use crate::exports::Member;

/// Run the multiple-returns analysis over one parsed file.
pub(crate) fn check_file(
    path: &Path,
    ast: &Ast,
    bindings: &Bindings,
    shapes: &HashMap<std::path::PathBuf, crate::exports::ExportShape>,
    all_bindings: &HashMap<std::path::PathBuf, Bindings>,
) -> Vec<Diagnostic> {
    let mut defs = DefCollector::default();
    defs.visit_ast(ast);
    let mut checker = CallSiteChecker {
        defs: &defs.functions,
        bindings,
        context: LookupContext::new(shapes, all_bindings),
        path,
        diagnostics: Vec::new(),
    };
    checker.visit_ast(ast);
    checker.diagnostics
}

// ---- pass 1: collect same-file function definitions ----

#[derive(Default)]
struct DefCollector {
    /// Function name -> known return arity.
    functions: HashMap<String, usize>,
}

impl Visitor for DefCollector {
    fn visit_local_function(&mut self, node: &LocalFunction) {
        if let Some(name) = token_name(node.name())
            && let Some(arity) = return_arity(node.body())
        {
            self.functions.insert(name, arity);
        }
    }

    fn visit_function_declaration(&mut self, node: &FunctionDeclaration) {
        // Only plain `function f(...)`; dotted/method names are skipped.
        let name = node.name();
        if name.method_name().is_some() || name.names().iter().count() != 1 {
            return;
        }
        if let Some(fn_name) = name.names().iter().next().and_then(token_name)
            && let Some(arity) = return_arity(node.body())
        {
            self.functions.insert(fn_name, arity);
        }
    }

    fn visit_local_assignment(&mut self, node: &LocalAssignment) {
        for (name_token, expr) in node.names().iter().zip(node.expressions().iter()) {
            let Expression::Function(function) = expr else {
                continue;
            };
            let Some(name) = token_name(name_token) else {
                continue;
            };
            if let Some(arity) = return_arity(function.body()) {
                self.functions.insert(name, arity);
            }
        }
    }

    fn visit_assignment(&mut self, node: &Assignment) {
        for (var, expr) in node.variables().iter().zip(node.expressions().iter()) {
            let (Var::Name(name_token), Expression::Function(function)) = (var, expr) else {
                continue;
            };
            let Some(name) = token_name(name_token) else {
                continue;
            };
            if let Some(arity) = return_arity(function.body()) {
                self.functions.insert(name, arity);
            }
        }
    }
}

// ---- pass 2: check call sites in binding position ----

struct CallSiteChecker<'a> {
    defs: &'a HashMap<String, usize>,
    bindings: &'a Bindings,
    context: LookupContext<'a>,
    path: &'a Path,
    diagnostics: Vec<Diagnostic>,
}

impl Visitor for CallSiteChecker<'_> {
    fn visit_local_assignment(&mut self, node: &LocalAssignment) {
        self.check_binding(node.names().iter().count(), node.expressions());
    }

    fn visit_assignment(&mut self, node: &Assignment) {
        self.check_binding(node.variables().iter().count(), node.expressions());
    }
}

impl CallSiteChecker<'_> {
    fn check_binding(&mut self, names: usize, exprs: &Punctuated<Expression>) {
        let expr_list: Vec<&Expression> = exprs.iter().collect();
        for (index, expr) in expr_list.iter().enumerate() {
            let Expression::FunctionCall(call) = expr else {
                continue;
            };
            let tail = index == expr_list.len() - 1;
            // A call in tail position propagates all its values into the
            // remaining binding slots; a non-tail call is truncated to 1.
            let bound = if tail {
                names.saturating_sub(expr_list.len() - 1)
            } else {
                1
            };
            let Some((name, returns, line, column)) = self.resolve_callee(call) else {
                continue;
            };
            let message = match returns.cmp(&bound) {
                std::cmp::Ordering::Greater => format!(
                    "`{name}()` returns {}; {bound} bound, {} discarded",
                    plural(returns, "value"),
                    returns - bound,
                ),
                std::cmp::Ordering::Less => format!(
                    "`{name}()` returns {}; {bound} bound, {} will be nil",
                    plural(returns, "value"),
                    plural(bound - returns, "variable"),
                ),
                std::cmp::Ordering::Equal => continue,
            };
            self.diagnostics.push(
                Diagnostic::new(DiagnosticKind::MultiReturnInfo, message).at(
                    self.path.to_path_buf(),
                    line,
                    column,
                ),
            );
        }
    }
    /// Resolve a call to a display name and known return arity: bare same-file
    /// definitions (`f(...)`), or module members (`mod.f(...)`, `mod:f(...)`)
    /// through require bindings. Anything else is unknown → None.
    fn resolve_callee(&self, call: &FunctionCall) -> Option<(String, usize, usize, usize)> {
        let Prefix::Name(name_ref) = call.prefix() else {
            return None;
        };
        let name = token_name(name_ref)?;
        let position = name_ref.start_position();
        let (line, column) = (position.line(), position.character());
        let mut suffixes = call.suffixes();
        match suffixes.next() {
            // Bare `f(...)`: same-file definitions only.
            Some(Suffix::Call(Call::AnonymousCall(_))) => {
                let arity = *self.defs.get(&name)?;
                Some((name, arity, line, column))
            }
            // `mod.f(...)`: module member with known arity.
            Some(Suffix::Index(Index::Dot { name: member, .. })) => {
                if !matches!(suffixes.next(), Some(Suffix::Call(Call::AnonymousCall(_)))) {
                    return None;
                }
                let member = token_name(member)?;
                let target = self.lookup_target(&name)?;
                let Some(Member::Function { arity, .. }) =
                    self.context.resolve(target, &member).member
                else {
                    return None;
                };
                Some((format!("{name}.{member}"), arity, line, column))
            }
            // `mod:f(...)`: method call; arity is return arity (self is a
            // parameter, not a return value).
            Some(Suffix::Call(Call::MethodCall(method))) => {
                let member = token_name(method.name())?;
                let target = self.lookup_target(&name)?;
                let Some(Member::Function { arity, .. }) =
                    self.context.resolve(target, &member).member
                else {
                    return None;
                };
                Some((format!("{name}:{member}"), arity, line, column))
            }
            _ => None,
        }
    }

    fn lookup_target(&self, name: &str) -> Option<LookupTarget> {
        if let Some(path) = self.bindings.get_instance(name) {
            Some(LookupTarget::Instance(path.clone()))
        } else {
            self.bindings
                .get_module(name)
                .cloned()
                .map(LookupTarget::Module)
        }
    }
}

fn token_name(token: &TokenReference) -> Option<String> {
    if let TokenType::Identifier { identifier } = token.token().token_type() {
        Some(identifier.to_string())
    } else {
        None
    }
}

fn plural(count: usize, word: &str) -> String {
    if count == 1 {
        format!("{count} {word}")
    } else {
        format!("{count} {word}s")
    }
}
