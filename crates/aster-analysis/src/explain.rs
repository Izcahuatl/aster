//! Human-readable lookup traces for the metatable inspector.

use std::collections::HashMap;

use full_moon::ast::{Ast, Call, Expression, FunctionCall, Index, LastStmt, Prefix, Suffix, Var};
use full_moon::tokenizer::TokenReference;
use full_moon::visitors::Visitor;

use crate::ast_util::{identifier_name, string_literal};
use crate::checks::bindings::Bindings;
use crate::checks::classes::{LookupContext, LookupTarget};
use crate::exports::ExportShape;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LookupResult {
    Found(String),
    NotFound,
    Unknown(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LookupExplanation {
    pub expression: String,
    pub line: usize,
    pub column: usize,
    pub start_column: usize,
    pub end_column: usize,
    pub steps: Vec<String>,
    pub result: LookupResult,
}

pub(crate) fn explain_file(
    ast: &Ast,
    path: &std::path::Path,
    line: usize,
    bindings: &Bindings,
    shapes: &HashMap<std::path::PathBuf, ExportShape>,
    all_bindings: &HashMap<std::path::PathBuf, Bindings>,
) -> Vec<LookupExplanation> {
    let context = LookupContext::new(shapes, all_bindings);
    let mut visitor = ExplainVisitor {
        line,
        bindings,
        context,
        current_path: path.to_path_buf(),
        local_class_name: exported_table_name(ast),
        explanations: Vec::new(),
    };
    visitor.visit_ast(ast);
    visitor.explanations
}

struct ExplainVisitor<'a> {
    line: usize,
    bindings: &'a Bindings,
    context: LookupContext<'a>,
    current_path: std::path::PathBuf,
    local_class_name: Option<String>,
    explanations: Vec<LookupExplanation>,
}

fn exported_table_name(ast: &Ast) -> Option<String> {
    let LastStmt::Return(return_stmt) = ast.nodes().last_stmt()? else {
        return None;
    };
    let expressions = return_stmt.returns().iter().collect::<Vec<_>>();
    let [Expression::Var(Var::Name(name))] = expressions.as_slice() else {
        return None;
    };
    identifier_name(name)
}

impl ExplainVisitor<'_> {
    fn explain(
        &mut self,
        binding: String,
        member: String,
        prefix: &TokenReference,
        member_token: &TokenReference,
    ) {
        let position = member_token.start_position();
        if position.line() != self.line {
            return;
        }
        let target = if let Some(path) = self.bindings.get_instance(&binding) {
            Some(LookupTarget::Instance(path.clone()))
        } else if let Some(path) = self.bindings.get_module(&binding) {
            Some(LookupTarget::Module(path.clone()))
        } else if self.local_class_name.as_deref() == Some(binding.as_str()) {
            Some(LookupTarget::Module(self.current_path.clone()))
        } else if binding == "self" && self.local_class_name.is_some() {
            Some(LookupTarget::Instance(self.current_path.clone()))
        } else {
            None
        };
        let resolved = if let Some(target) = target {
            self.context.resolve(target, &member)
        } else {
            crate::checks::classes::LookupResolution {
                steps: vec![format!("receiver `{binding}` has no known class binding")],
                result: LookupResult::Unknown(format!(
                    "Aster cannot infer the class or module behind `{binding}`"
                )),
                member: None,
            }
        };
        self.explanations.push(LookupExplanation {
            expression: format!("{binding}.{member}"),
            line: position.line(),
            column: position.character(),
            start_column: prefix.start_position().character(),
            end_column: member_token.end_position().character(),
            steps: resolved.steps,
            result: resolved.result,
        });
    }
}

impl Visitor for ExplainVisitor<'_> {
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
                    self.explain(binding, member, prefix, name);
                }
            }
            Some(Suffix::Index(Index::Brackets {
                expression: Expression::String(token),
                ..
            })) => {
                if let Some(member) = string_literal(token) {
                    self.explain(binding, member, prefix, token);
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
                    self.explain(binding, member, prefix, name);
                }
            }
            Some(Suffix::Call(Call::MethodCall(method))) => {
                if let Some(member) = identifier_name(method.name()) {
                    self.explain(binding, member, prefix, method.name());
                }
            }
            _ => {}
        }
    }
}
