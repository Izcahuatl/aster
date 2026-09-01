//! Performance and inefficiency heuristics (concept.md §5).

use std::path::Path;

use full_moon::ast::{
    Ast, BinOp, Block, Expression, FunctionBody, FunctionCall, GenericFor, Index, LocalAssignment,
    LocalFunction, NumericFor, Parameter, Prefix, Repeat, Suffix, TableConstructor, While,
};
use full_moon::node::Node;
use full_moon::tokenizer::{Position, TokenReference};
use full_moon::visitors::Visitor;

use crate::ast_util::identifier_name;
use crate::diagnostic::{Diagnostic, DiagnosticKind};

const GLOBAL_LIBRARIES: &[&str] = &[
    "math",
    "string",
    "table",
    "bit",
    "io",
    "os",
    "debug",
    "package",
    "coroutine",
];

const GLOBAL_BUILTINS: &[&str] = &[
    "pairs",
    "ipairs",
    "type",
    "tostring",
    "tonumber",
    "pcall",
    "xpcall",
    "rawget",
    "rawset",
    "rawequal",
    "setmetatable",
    "getmetatable",
];

pub(crate) fn check_file(path: &Path, ast: &Ast) -> Vec<Diagnostic> {
    let mut context = ContextCollector::new();
    context.visit_ast(ast);
    let mut checker = PerformanceChecker {
        path,
        bindings: context.bindings,
        loops: context.loops,
        functions: context.functions,
        diagnostics: Vec::new(),
    };
    checker.visit_ast(ast);
    checker.diagnostics
}

#[derive(Debug)]
struct LocalBinding {
    name: String,
    visible_from: usize,
    scope_end: usize,
}

#[derive(Debug)]
struct LoopRange {
    start: usize,
    end: usize,
    function_id: usize,
}

#[derive(Debug)]
struct FunctionRange {
    id: usize,
    start: usize,
    end: usize,
}

struct ContextCollector {
    bindings: Vec<LocalBinding>,
    loops: Vec<LoopRange>,
    functions: Vec<FunctionRange>,
    block_ends: Vec<usize>,
    function_stack: Vec<usize>,
    next_function_id: usize,
}

impl ContextCollector {
    fn new() -> Self {
        Self {
            bindings: Vec::new(),
            loops: Vec::new(),
            functions: Vec::new(),
            block_ends: Vec::new(),
            function_stack: vec![0],
            next_function_id: 1,
        }
    }

    fn add_binding(&mut self, name: String, visible_from: usize, scope_end: usize) {
        self.bindings.push(LocalBinding {
            name,
            visible_from,
            scope_end,
        });
    }

    fn add_loop(&mut self, block: &Block) {
        if let Some((start, end)) = block.range() {
            self.loops.push(LoopRange {
                start: start.bytes(),
                end: end.bytes(),
                function_id: *self.function_stack.last().unwrap_or(&0),
            });
        }
    }

    fn add_loop_binding(&mut self, token: &TokenReference, block: &Block) {
        let (Some(name), Some((start, end))) = (identifier_name(token), block.range()) else {
            return;
        };
        self.add_binding(name, start.bytes(), end.bytes());
    }
}

impl Visitor for ContextCollector {
    fn visit_block(&mut self, block: &Block) {
        self.block_ends.push(
            block
                .end_position()
                .map(Position::bytes)
                .unwrap_or(usize::MAX),
        );
    }

    fn visit_block_end(&mut self, _block: &Block) {
        self.block_ends.pop();
    }

    fn visit_local_assignment_end(&mut self, node: &LocalAssignment) {
        let visible_from = node.end_position().map(Position::bytes).unwrap_or(0);
        let scope_end = *self.block_ends.last().unwrap_or(&usize::MAX);
        for token in node.names() {
            if let Some(name) = identifier_name(token) {
                self.add_binding(name, visible_from, scope_end);
            }
        }
    }

    fn visit_local_function(&mut self, node: &LocalFunction) {
        let visible_from = node.start_position().map(Position::bytes).unwrap_or(0);
        let scope_end = *self.block_ends.last().unwrap_or(&usize::MAX);
        if let Some(name) = identifier_name(node.name()) {
            self.add_binding(name, visible_from, scope_end);
        }
    }

    fn visit_function_body(&mut self, body: &FunctionBody) {
        let id = self.next_function_id;
        self.next_function_id += 1;
        if let Some((start, end)) = body.block().range() {
            self.functions.push(FunctionRange {
                id,
                start: start.bytes(),
                end: end.bytes(),
            });
            for parameter in body.parameters() {
                if let Parameter::Name(token) = parameter
                    && let Some(name) = identifier_name(token)
                {
                    self.add_binding(name, start.bytes(), end.bytes());
                }
            }
        }
        self.function_stack.push(id);
    }

    fn visit_function_body_end(&mut self, _body: &FunctionBody) {
        self.function_stack.pop();
    }

    fn visit_numeric_for(&mut self, node: &NumericFor) {
        self.add_loop(node.block());
        self.add_loop_binding(node.index_variable(), node.block());
    }

    fn visit_generic_for(&mut self, node: &GenericFor) {
        self.add_loop(node.block());
        for token in node.names() {
            self.add_loop_binding(token, node.block());
        }
    }

    fn visit_while(&mut self, node: &While) {
        self.add_loop(node.block());
    }

    fn visit_repeat(&mut self, node: &Repeat) {
        self.add_loop(node.block());
    }
}

struct PerformanceChecker<'a> {
    path: &'a Path,
    bindings: Vec<LocalBinding>,
    loops: Vec<LoopRange>,
    functions: Vec<FunctionRange>,
    diagnostics: Vec<Diagnostic>,
}

impl PerformanceChecker<'_> {
    fn function_at(&self, byte: usize) -> usize {
        self.functions
            .iter()
            .filter(|range| byte >= range.start && byte <= range.end)
            .min_by_key(|range| range.end - range.start)
            .map(|range| range.id)
            .unwrap_or(0)
    }

    fn is_in_loop(&self, byte: usize) -> bool {
        let function_id = self.function_at(byte);
        self.loops.iter().any(|range| {
            range.function_id == function_id && byte >= range.start && byte <= range.end
        })
    }

    fn is_local(&self, name: &str, byte: usize) -> bool {
        self.bindings.iter().any(|binding| {
            binding.name == name && byte >= binding.visible_from && byte <= binding.scope_end
        })
    }

    fn check_global_access(&mut self, base: &str, member: Option<&str>, token: &TokenReference) {
        let Some(pos) = token.start_position() else {
            return;
        };
        if !self.is_in_loop(pos.bytes()) || self.is_local(base, pos.bytes()) {
            return;
        }
        if let Some(member) = member {
            if GLOBAL_LIBRARIES.contains(&base) {
                self.diagnostics.push(
                    Diagnostic::new(
                        DiagnosticKind::GlobalInLoop,
                        format!(
                            "repeated global lookup `{base}.{member}` in loop; consider caching `local {member} = {base}.{member}` outside the loop"
                        ),
                    )
                    .at(self.path.to_path_buf(), pos.line(), pos.character()),
                );
            }
        } else if GLOBAL_BUILTINS.contains(&base) {
            self.diagnostics.push(
                Diagnostic::new(
                    DiagnosticKind::GlobalInLoop,
                    format!(
                        "repeated global call `{base}()` in loop; consider caching `local {base} = {base}` outside the loop"
                    ),
                )
                .at(self.path.to_path_buf(), pos.line(), pos.character()),
            );
        }
    }
}

impl Visitor for PerformanceChecker<'_> {
    fn visit_function_call(&mut self, call: &FunctionCall) {
        let Prefix::Name(prefix) = call.prefix() else {
            return;
        };
        let Some(base) = identifier_name(prefix) else {
            return;
        };
        match call.suffixes().next() {
            Some(Suffix::Index(Index::Dot { name, .. })) => {
                if let Some(member) = identifier_name(name) {
                    self.check_global_access(&base, Some(&member), prefix);
                }
            }
            Some(Suffix::Call(_)) => self.check_global_access(&base, None, prefix),
            _ => {}
        }
    }

    fn visit_expression(&mut self, expression: &Expression) {
        let Expression::BinaryOperator {
            binop: BinOp::TwoDots(token),
            ..
        } = expression
        else {
            return;
        };
        let Some(pos) = token.start_position() else {
            return;
        };
        if self.is_in_loop(pos.bytes()) {
            self.diagnostics.push(
                Diagnostic::new(
                    DiagnosticKind::StringConcatInLoop,
                    "string concatenation `..` in loop causes repeated GC allocations; consider using `table.concat`",
                )
                .at(self.path.to_path_buf(), pos.line(), pos.character()),
            );
        }
    }

    fn visit_table_constructor(&mut self, node: &TableConstructor) {
        let (start_brace, _) = node.braces().tokens();
        let Some(pos) = start_brace.start_position() else {
            return;
        };
        if self.is_in_loop(pos.bytes()) {
            self.diagnostics.push(
                Diagnostic::new(
                    DiagnosticKind::TableAllocationInLoop,
                    "table constructor `{}` allocated in loop creates GC pressure; consider reusing a table across iterations",
                )
                .at(self.path.to_path_buf(), pos.line(), pos.character()),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn diagnostics(source: &str) -> Vec<Diagnostic> {
        let ast = full_moon::parse(source).expect("fixture should parse");
        check_file(Path::new("main.lua"), &ast)
    }

    #[test]
    fn later_local_does_not_hide_earlier_global() {
        let found = diagnostics("for i = 1, 2 do math.sqrt(i) end\nlocal math = {}\n");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].kind, DiagnosticKind::GlobalInLoop);
    }

    #[test]
    fn local_visible_before_loop_hides_global() {
        let found = diagnostics("local math = math\nfor i = 1, 2 do math.sqrt(i) end\n");
        assert!(found.is_empty(), "diagnostics: {found:?}");
    }

    #[test]
    fn generic_for_iterator_is_not_repeated_loop_work() {
        let found = diagnostics("for item in pairs(items) do print(item) end\n");
        assert!(found.is_empty(), "diagnostics: {found:?}");
    }

    #[test]
    fn nested_function_body_is_not_part_of_enclosing_loop_execution() {
        let found =
            diagnostics("for i = 1, 2 do\n  local f = function() return math.sqrt(i) end\nend\n");
        assert!(found.is_empty(), "diagnostics: {found:?}");
    }
}
