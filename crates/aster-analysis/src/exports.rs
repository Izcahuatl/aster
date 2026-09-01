//! Module export shapes: what a module returns from its top-level
//! `return` statement. Computed from the module's own AST alone — no
//! cross-file recursion, so require cycles cannot loop this analysis.

use std::collections::{HashMap, HashSet};

use full_moon::ast::Ast;

/// What a module exports. Members are flat: a member that is itself a
/// table is `Member::Value` (nested shapes are out of scope for v3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ExportShape {
    pub members: HashMap<String, Member>,
    /// Static class/metatable facts recognized for this exported table.
    pub class: Option<ClassInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Member {
    /// A function with known return arity.
    Function {
        arity: usize,
        returns_instance: bool,
    },
    /// A non-function value, or a function of unknown arity.
    Value,
}

/// The statically-known object model attached to one exported table.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ClassInfo {
    pub index_self: bool,
    pub index_other: Option<String>,
    pub index_unknown: bool,
    pub metatable_name: Option<String>,
    pub metatable_unknown: bool,
    pub instance_members: HashSet<String>,
    pub parent_constructor: Option<String>,
}

use full_moon::ast::{
    Block, Expression, Field, FunctionName, LastStmt, Stmt, TableConstructor, Var,
};
use full_moon::ast::{Index, Prefix, Suffix};

use crate::ast_util::{identifier_name, string_literal};
use crate::checks::{has_nested_returns, return_arity};

/// Compute a module's export shape, or None when unknown or ambiguous
/// (no top-level return, non-table return, conditional returns, or the
/// exported local is ever rebound wholesale).
pub(crate) fn module_export(ast: &Ast) -> Option<ExportShape> {
    let block = ast.nodes();
    if has_nested_returns(block) {
        return None;
    }
    let Some(LastStmt::Return(ret)) = block.last_stmt() else {
        return None;
    };
    let exprs: Vec<&Expression> = ret.returns().iter().collect();
    let [single] = exprs.as_slice() else {
        return None;
    };
    match single {
        Expression::TableConstructor(table) => shape_from_constructor(table),
        Expression::Var(Var::Name(name_token)) => {
            let name = identifier_name(name_token)?;
            shape_from_builder(block, &name)
        }
        _ => None,
    }
}

/// Shape of `return { ... }`.
fn shape_from_constructor(table: &TableConstructor) -> Option<ExportShape> {
    let mut members = HashMap::new();
    for field in table.fields() {
        match field {
            Field::NameKey { key, value, .. } => {
                if let Some(name) = identifier_name(key) {
                    members.insert(name, member_from_value(value, false));
                }
            }
            Field::ExpressionKey { key, value, .. } => {
                // String-literal keys only: `{ ["f"] = ... }`.
                let Expression::String(token) = key else {
                    continue;
                };
                if let Some(name) = string_literal(token) {
                    members.insert(name, member_from_value(value, false));
                }
            }
            // Positional members aren't name-addressable; ignored.
            _ => {}
        }
    }
    Some(ExportShape {
        members,
        class: None,
    })
}

/// Shape of `local M = {}; function M.f() ... end; M.x = ...; return M`.
/// Only top-level statements populate the shape.
fn shape_from_builder(block: &Block, name: &str) -> Option<ExportShape> {
    // The exported local must be declared exactly once, as a table
    // constructor, and never rebound wholesale.
    let mut declared = false;
    for stmt in block.stmts() {
        match stmt {
            Stmt::LocalAssignment(local) => {
                for (name_token, expr) in local.names().iter().zip(local.expressions().iter()) {
                    if identifier_name(name_token).as_deref() == Some(name) {
                        if declared || !matches!(expr, Expression::TableConstructor(_)) {
                            return None;
                        }
                        declared = true;
                    }
                }
            }
            Stmt::Assignment(assignment) => {
                for var in assignment.variables() {
                    if matches!(var, Var::Name(token) if identifier_name(token).as_deref() == Some(name))
                    {
                        return None; // `M = ...` rebinds the whole table
                    }
                }
            }
            _ => {}
        }
    }
    if !declared {
        return None;
    }

    let mut members = HashMap::new();
    let mut class = ClassInfo::default();
    let mut has_constructor = false;
    for stmt in block.stmts() {
        match stmt {
            Stmt::FunctionDeclaration(decl) => {
                let Some(member_name) = declared_member(decl.name(), name) else {
                    continue;
                };
                let returns_instance = returns_instance(decl.body(), name);
                let member = function_member(decl.body(), returns_instance);
                if returns_instance {
                    has_constructor = true;
                    collect_constructor_facts(decl.body(), &mut class);
                }
                members.insert(member_name, member);
            }
            Stmt::Assignment(assignment) => {
                for (var, expr) in assignment
                    .variables()
                    .iter()
                    .zip(assignment.expressions().iter())
                {
                    if let Some((table, member_name)) = dot_target(var) {
                        if table == name {
                            if member_name == "__index" {
                                match expr {
                                    Expression::Var(Var::Name(target)) => {
                                        let target = identifier_name(target)?;
                                        if target == name {
                                            class.index_self = true;
                                        } else {
                                            class.index_other = Some(target);
                                        }
                                    }
                                    _ => class.index_unknown = true,
                                }
                            } else {
                                let returns_instance = match expr {
                                    Expression::Function(function) => {
                                        returns_instance(function.body(), name)
                                    }
                                    _ => false,
                                };
                                if returns_instance {
                                    has_constructor = true;
                                    if let Expression::Function(function) = expr {
                                        collect_constructor_facts(function.body(), &mut class);
                                    }
                                }
                                members
                                    .insert(member_name, member_from_value(expr, returns_instance));
                            }
                        }
                    } else if targets_table(var, name) {
                        // `M["x"] = ...`, `M.a.b = ...` — members we can't
                        // model; the shape is no longer fully known.
                        return None;
                    }
                }
            }
            Stmt::FunctionCall(call) => collect_metatable_fact(call, name, &mut class),
            _ => {}
        }
    }
    let class = (class.index_self
        || class.index_other.is_some()
        || class.index_unknown
        || class.metatable_name.is_some()
        || class.metatable_unknown
        || !class.instance_members.is_empty()
        || class.parent_constructor.is_some()
        || has_constructor)
        .then_some(class);
    Some(ExportShape { members, class })
}

/// The member name of `function M.f()` / `function M:f()` declarations on
/// the exported table `name` — or None for any other declaration shape.
fn declared_member(function_name: &FunctionName, name: &str) -> Option<String> {
    let parts: Vec<String> = function_name
        .names()
        .iter()
        .filter_map(identifier_name)
        .collect();
    if let Some(method) = function_name.method_name() {
        // `function M:f()` — dotted prefix must be exactly `M`.
        if parts.len() == 1 && parts[0] == name {
            return identifier_name(method);
        }
        return None;
    }
    // `function M.f()` — dotted prefix must be exactly `M.f`.
    if parts.len() == 2 && parts[0] == name {
        return parts.into_iter().nth(1);
    }
    None
}

/// If `var` is exactly `name.member` (a single Dot suffix), return both names.
fn dot_target(var: &Var) -> Option<(String, String)> {
    let Var::Expression(var_expr) = var else {
        return None;
    };
    let Prefix::Name(prefix) = var_expr.prefix() else {
        return None;
    };
    let table = identifier_name(prefix)?;
    let mut suffixes = var_expr.suffixes();
    let Some(Suffix::Index(Index::Dot { name: member, .. })) = suffixes.next() else {
        return None;
    };
    if suffixes.next().is_some() {
        return None; // longer chains are out of scope
    }
    Some((table, identifier_name(member)?))
}

/// Does this assignment target write into table `name` (any suffix shape)?
fn targets_table(var: &Var, name: &str) -> bool {
    let Var::Expression(var_expr) = var else {
        return false;
    };
    matches!(var_expr.prefix(), Prefix::Name(t) if identifier_name(t).as_deref() == Some(name))
}

/// Member kind from a right-hand-side expression.
fn member_from_value(expr: &Expression, returns_instance: bool) -> Member {
    match expr {
        Expression::Function(function) => function_member(function.body(), returns_instance),
        _ => Member::Value,
    }
}

fn function_member(body: &full_moon::ast::FunctionBody, returns_instance: bool) -> Member {
    // Lua's built-in `setmetatable` returns its first argument, so the
    // recognized constructor form has a precise one-value return even though
    // generic tail-call analysis is otherwise intentionally conservative.
    if returns_instance {
        return Member::Function {
            arity: 1,
            returns_instance: true,
        };
    }
    return_arity(body)
        .map(|arity| Member::Function {
            arity,
            returns_instance,
        })
        .unwrap_or(Member::Value)
}

/// Recognize a constructor whose final, top-level result is
/// `setmetatable(value, ExportedTable)`.
fn returns_instance(body: &full_moon::ast::FunctionBody, table: &str) -> bool {
    let Some(LastStmt::Return(ret)) = body.block().last_stmt() else {
        return false;
    };
    let exprs: Vec<&Expression> = ret.returns().iter().collect();
    let [Expression::FunctionCall(call)] = exprs.as_slice() else {
        return false;
    };
    let Some(args) = bare_call_args(call, "setmetatable") else {
        return false;
    };
    matches!(args.get(1).copied(), Some(Expression::Var(Var::Name(name))) if identifier_name(name).as_deref() == Some(table))
}

/// Collect `self.field = ...` and `self = Parent.new(...)` facts from the
/// top-level body of a recognized constructor.
fn collect_constructor_facts(body: &full_moon::ast::FunctionBody, class: &mut ClassInfo) {
    for stmt in body.block().stmts() {
        if let Stmt::LocalAssignment(local) = stmt {
            for (name, expr) in local.names().iter().zip(local.expressions().iter()) {
                if identifier_name(name).as_deref() == Some("self")
                    && let Some(parent) = constructor_parent(expr)
                {
                    class.parent_constructor = Some(parent);
                }
            }
            continue;
        }
        let Stmt::Assignment(assignment) = stmt else {
            continue;
        };
        for (var, expr) in assignment
            .variables()
            .iter()
            .zip(assignment.expressions().iter())
        {
            if let Some((table, member)) = dot_target(var) {
                if table == "self" {
                    class.instance_members.insert(member);
                }
                continue;
            }
            if matches!(var, Var::Name(name) if identifier_name(name).as_deref() == Some("self"))
                && let Some(parent) = constructor_parent(expr)
            {
                class.parent_constructor = Some(parent);
            }
        }
    }
}

fn constructor_parent(expr: &Expression) -> Option<String> {
    let Expression::FunctionCall(call) = expr else {
        return None;
    };
    let Prefix::Name(prefix) = call.prefix() else {
        return None;
    };
    let parent = identifier_name(prefix)?;
    let mut suffixes = call.suffixes();
    let Some(Suffix::Index(Index::Dot { name, .. })) = suffixes.next() else {
        return None;
    };
    if identifier_name(name).as_deref() != Some("new")
        || !matches!(suffixes.next(), Some(Suffix::Call(_)))
        || suffixes.next().is_some()
    {
        return None;
    }
    Some(parent)
}

/// Recognize `setmetatable(T, { __index = Parent })` at module top level.
fn collect_metatable_fact(call: &full_moon::ast::FunctionCall, table: &str, class: &mut ClassInfo) {
    let Some(args) = bare_call_args(call, "setmetatable") else {
        return;
    };
    if !matches!(args.first().copied(), Some(Expression::Var(Var::Name(name))) if identifier_name(name).as_deref() == Some(table))
    {
        return;
    }
    let Some(Expression::TableConstructor(meta)) = args.get(1).copied() else {
        class.metatable_unknown = true;
        return;
    };
    for field in meta.fields() {
        let Field::NameKey { key, value, .. } = field else {
            continue;
        };
        if identifier_name(key).as_deref() != Some("__index") {
            continue;
        }
        match value {
            Expression::Var(Var::Name(name)) => class.metatable_name = identifier_name(name),
            _ => class.metatable_unknown = true,
        }
    }
}

fn bare_call_args<'a>(
    call: &'a full_moon::ast::FunctionCall,
    name: &str,
) -> Option<Vec<&'a Expression>> {
    let Prefix::Name(prefix) = call.prefix() else {
        return None;
    };
    if identifier_name(prefix).as_deref() != Some(name) {
        return None;
    }
    let mut suffixes = call.suffixes();
    let Some(Suffix::Call(full_moon::ast::Call::AnonymousCall(args))) = suffixes.next() else {
        return None;
    };
    if suffixes.next().is_some() {
        return None;
    }
    match args {
        full_moon::ast::FunctionArgs::Parentheses { arguments, .. } => {
            Some(arguments.iter().collect())
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn export(source: &str) -> Option<ExportShape> {
        let ast = full_moon::parse(source)
            .map_err(|mut e| e.swap_remove(0))
            .unwrap();
        module_export(&ast)
    }

    #[test]
    fn table_builder_pattern() {
        let shape =
            export("local M = {}\nfunction M.f() return 1 end\nM.x = 5\nreturn M\n").unwrap();
        assert_eq!(
            shape.members["f"],
            Member::Function {
                arity: 1,
                returns_instance: false
            }
        );
        assert_eq!(shape.members["x"], Member::Value);
        assert_eq!(shape.members.len(), 2);
    }

    #[test]
    fn direct_table_return() {
        let shape = export("return { f = function() return 1, 2 end, x = 1 }\n").unwrap();
        assert_eq!(
            shape.members["f"],
            Member::Function {
                arity: 2,
                returns_instance: false
            }
        );
        assert_eq!(shape.members["x"], Member::Value);
    }

    #[test]
    fn method_definition() {
        let shape = export("local M = {}\nfunction M:f() return 1 end\nreturn M\n").unwrap();
        assert_eq!(
            shape.members["f"],
            Member::Function {
                arity: 1,
                returns_instance: false
            }
        );
    }

    #[test]
    fn string_keyed_constructor_field() {
        let shape = export("return { [\"f\"] = function() end }\n").unwrap();
        assert_eq!(
            shape.members["f"],
            Member::Function {
                arity: 0,
                returns_instance: false
            }
        );
    }

    #[test]
    fn no_return_is_none() {
        assert_eq!(export("local M = {}\n"), None);
    }

    #[test]
    fn conditional_return_is_none() {
        assert_eq!(
            export("local M = {}\nif x then return M end\nreturn M\n"),
            None
        );
    }

    #[test]
    fn unknown_arity_member_becomes_value() {
        let shape = export(
            "local M = {}\nfunction M.f() if c then return 1 end return 1, 2 end\nreturn M\n",
        )
        .unwrap();
        assert_eq!(shape.members["f"], Member::Value);
    }

    #[test]
    fn non_table_return_is_none() {
        assert_eq!(export("return 5\n"), None);
    }

    #[test]
    fn wholesale_rebinding_is_none() {
        assert_eq!(export("local M = {}\nM = 5\nreturn M\n"), None);
    }

    #[test]
    fn bracket_keyed_assignment_makes_shape_unknown() {
        assert_eq!(export("local M = {}\nM[\"x\"] = 5\nreturn M\n"), None);
    }

    #[test]
    fn recognizes_class_constructor_and_metatable_facts() {
        let shape = export(
            "local Parent = require('parent')\nlocal M = {}\nM.__index = M\nsetmetatable(M, { __index = Parent })\nfunction M.new()\n  local self = Parent.new()\n  self.score = 0\n  return setmetatable(self, M)\nend\nreturn M\n",
        )
        .unwrap();
        let class = shape.class.unwrap();
        assert!(class.index_self);
        assert_eq!(class.metatable_name.as_deref(), Some("Parent"));
        assert_eq!(class.parent_constructor.as_deref(), Some("Parent"));
        assert!(class.instance_members.contains("score"));
        assert!(matches!(
            shape.members["new"],
            Member::Function {
                returns_instance: true,
                ..
            }
        ));
    }
}
