//! Unknown-member detection over known module and instance bindings.

use std::collections::HashMap;
use std::path::Path;

use full_moon::ast::{Ast, Call, Expression, FunctionCall, Index, Prefix, Suffix, Var};
use full_moon::tokenizer::TokenReference;
use full_moon::visitors::Visitor;

use crate::ast_util::{identifier_name, string_literal};
use crate::checks::bindings::Bindings;
use crate::checks::classes::{LookupContext, LookupTarget};
use crate::diagnostic::{Diagnostic, DiagnosticKind};
use crate::explain::LookupResult;
use crate::exports::ExportShape;

pub(crate) fn check_file(
    path: &Path,
    ast: &Ast,
    bindings: &Bindings,
    shapes: &HashMap<std::path::PathBuf, ExportShape>,
    all_bindings: &HashMap<std::path::PathBuf, Bindings>,
) -> Vec<Diagnostic> {
    let context = LookupContext::new(shapes, all_bindings);
    let mut visitor = MemberVisitor {
        bindings,
        context,
        path,
        diagnostics: Vec::new(),
    };
    visitor.visit_ast(ast);
    visitor.diagnostics
}

struct MemberVisitor<'a> {
    bindings: &'a Bindings,
    context: LookupContext<'a>,
    path: &'a Path,
    diagnostics: Vec<Diagnostic>,
}

impl MemberVisitor<'_> {
    fn check_member(&mut self, binding: String, member: String, token: &TokenReference) {
        let (target, subject) = if let Some(path) = self.bindings.get_instance(&binding) {
            (LookupTarget::Instance(path.clone()), "instance")
        } else if let Some(path) = self.bindings.get_module(&binding) {
            (LookupTarget::Module(path.clone()), "module binding")
        } else {
            return;
        };
        if !matches!(
            self.context.resolve(target, &member).result,
            LookupResult::NotFound
        ) {
            return;
        }
        let position = token.start_position();
        self.diagnostics.push(
            Diagnostic::new(
                DiagnosticKind::UnknownMember,
                format!("unknown member `{member}` on {subject} `{binding}`"),
            )
            .at(
                self.path.to_path_buf(),
                position.line(),
                position.character(),
            ),
        );
    }
}

impl Visitor for MemberVisitor<'_> {
    fn visit_var(&mut self, var: &Var) {
        let Var::Expression(var_expr) = var else {
            return;
        };
        let Prefix::Name(prefix) = var_expr.prefix() else {
            return;
        };
        let Some(binding) = identifier_name(prefix) else {
            return;
        };
        match var_expr.suffixes().next() {
            Some(Suffix::Index(Index::Dot { name, .. })) => {
                if let Some(member) = identifier_name(name) {
                    self.check_member(binding, member, name);
                }
            }
            Some(Suffix::Index(Index::Brackets {
                expression: Expression::String(token),
                ..
            })) => {
                if let Some(member) = string_literal(token) {
                    self.check_member(binding, member, token);
                }
            }
            _ => {}
        }
    }

    fn visit_function_call(&mut self, call: &FunctionCall) {
        let Prefix::Name(prefix) = call.prefix() else {
            return;
        };
        let Some(binding) = identifier_name(prefix) else {
            return;
        };
        match call.suffixes().next() {
            Some(Suffix::Index(Index::Dot { name, .. })) => {
                if let Some(member) = identifier_name(name) {
                    self.check_member(binding, member, name);
                }
            }
            Some(Suffix::Call(Call::MethodCall(method))) => {
                if let Some(member) = identifier_name(method.name()) {
                    self.check_member(binding, member, method.name());
                }
            }
            _ => {}
        }
    }
}
