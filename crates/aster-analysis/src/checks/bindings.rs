//! Require bindings: `local name = require("mod")` connects a local name
//! to the required module's export shape (when fully known).

use std::collections::HashMap;
use std::path::PathBuf;

use full_moon::ast::{
    Ast, Call, Expression, FunctionArgs, FunctionCall, LocalAssignment, Prefix, Suffix,
};
use full_moon::visitors::Visitor;

use crate::ast_util::{identifier_name, string_literal};
use crate::exports::ExportShape;
use crate::resolve::Resolver;

/// Local names bound to modules and instances. Scope-blind (documented v2/v3
/// limitation): last binding in document order wins.
#[derive(Debug, Default)]
pub(crate) struct Bindings {
    modules: HashMap<String, PathBuf>,
    instances: HashMap<String, PathBuf>,
}

impl Bindings {
    pub(crate) fn get_module(&self, name: &str) -> Option<&PathBuf> {
        self.modules.get(name)
    }

    pub(crate) fn get_instance(&self, name: &str) -> Option<&PathBuf> {
        self.instances.get(name)
    }

    /// Collect bindings from `local name = require("mod")` assignments.
    /// Unresolvable modules and modules with unknown export shape simply
    /// produce no binding.
    pub(crate) fn collect(
        ast: &Ast,
        resolver: &Resolver,
        shapes: &HashMap<PathBuf, ExportShape>,
    ) -> Bindings {
        let mut visitor = BindingVisitor {
            resolver,
            shapes,
            modules: HashMap::new(),
            instances: HashMap::new(),
        };
        visitor.visit_ast(ast);
        Bindings {
            modules: visitor.modules,
            instances: visitor.instances,
        }
    }
}

struct BindingVisitor<'a, 'b> {
    resolver: &'b Resolver,
    shapes: &'a HashMap<PathBuf, ExportShape>,
    modules: HashMap<String, PathBuf>,
    instances: HashMap<String, PathBuf>,
}

impl Visitor for BindingVisitor<'_, '_> {
    fn visit_local_assignment(&mut self, node: &LocalAssignment) {
        for (name_token, expr) in node.names().iter().zip(node.expressions().iter()) {
            let Expression::FunctionCall(call) = expr else {
                continue;
            };
            let Some(module) = require_module_name(call) else {
                continue;
            };
            let Some(path) = self.resolver.resolve(&module) else {
                continue;
            };
            let Some(name) = identifier_name(name_token) else {
                continue;
            };
            if self.shapes.contains_key(&path) {
                self.modules.insert(name, path);
            }
        }
        for (name_token, expr) in node.names().iter().zip(node.expressions().iter()) {
            let Some(class_path) = instance_class(expr, &self.modules, self.shapes) else {
                continue;
            };
            let Some(name) = identifier_name(name_token) else {
                continue;
            };
            // `self` is method-local and scope-sensitive. The checker is
            // deliberately scope-blind, so class extraction handles it
            // instead of publishing a binding that would leak into callers.
            if name == "self" {
                continue;
            }
            self.instances.insert(name, class_path);
        }
    }
}

/// `local instance = Class.new(...)` where `Class` is a known module and
/// `new` is recognized as an instance-returning constructor.
fn instance_class(
    expr: &Expression,
    modules: &HashMap<String, PathBuf>,
    shapes: &HashMap<PathBuf, ExportShape>,
) -> Option<PathBuf> {
    let Expression::FunctionCall(call) = expr else {
        return None;
    };
    let Prefix::Name(prefix) = call.prefix() else {
        return None;
    };
    let class_name = identifier_name(prefix)?;
    let class_path = modules.get(&class_name)?;
    let mut suffixes = call.suffixes();
    let Some(Suffix::Index(full_moon::ast::Index::Dot { name: method, .. })) = suffixes.next()
    else {
        return None;
    };
    let method = identifier_name(method)?;
    if !matches!(suffixes.next(), Some(Suffix::Call(Call::AnonymousCall(_))))
        || suffixes.next().is_some()
    {
        return None;
    }
    let shape = shapes.get(class_path)?;
    matches!(
        shape.members.get(&method),
        Some(crate::exports::Member::Function {
            returns_instance: true,
            ..
        })
    )
    .then(|| class_path.clone())
}

/// If `call` is a bare `require("m")` / `require "m"` call, return the
/// module name.
fn require_module_name(call: &FunctionCall) -> Option<String> {
    let Prefix::Name(name_ref) = call.prefix() else {
        return None;
    };
    if identifier_name(name_ref).as_deref() != Some("require") {
        return None;
    }
    let mut suffixes = call.suffixes();
    let Some(Suffix::Call(Call::AnonymousCall(args))) = suffixes.next() else {
        return None;
    };
    // Chained calls (`require("m"):new()`, `require("m")()`) yield the call
    // result, not the module — no binding.
    if suffixes.next().is_some() {
        return None;
    }
    match args {
        FunctionArgs::String(token) => string_literal(token),
        FunctionArgs::Parentheses { arguments, .. } => {
            let Some(Expression::String(token)) = arguments.iter().next() else {
                return None;
            };
            string_literal(token)
        }
        _ => None,
    }
}
