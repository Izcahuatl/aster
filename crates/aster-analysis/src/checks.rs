//! Lua-specific static checks (concept.md §5), run per file over its AST.

pub(crate) mod bindings;
pub(crate) mod classes;
pub(crate) mod members;
pub(crate) mod performance;
pub(crate) mod returns;
pub(crate) mod sequence;

use full_moon::ast::{
    Block, Do, Expression, FunctionBody, GenericFor, If, LastStmt, NumericFor, Repeat, Return,
    While,
};
use full_moon::tokenizer::{Symbol, TokenReference, TokenType};
use full_moon::visitors::{Visit, Visitor};

/// Return arity of a function body, or None when unknown: conditional or
/// nested returns, a tail call (`return f()`), or varargs in the return
/// list all make the arity unknowable to this analysis.
pub(crate) fn return_arity(body: &FunctionBody) -> Option<usize> {
    if has_nested_returns(body.block()) {
        return None;
    }
    match body.block().last_stmt() {
        Some(LastStmt::Return(ret)) => {
            let exprs: Vec<&Expression> = ret.returns().iter().collect();
            match exprs.last() {
                None => Some(0),
                Some(Expression::FunctionCall(_)) => None,
                Some(Expression::Symbol(token)) if is_vararg(token) => None,
                Some(_) => Some(exprs.len()),
            }
        }
        _ => Some(0),
    }
}

pub(crate) fn is_vararg(token: &TokenReference) -> bool {
    matches!(token.token().token_type(), TokenType::Symbol { symbol } if *symbol == Symbol::Ellipsis)
}

/// Detect `return` statements inside nested blocks (if/loops/do) while
/// ignoring returns inside nested function bodies.
pub(crate) fn has_nested_returns(block: &Block) -> bool {
    #[derive(Default)]
    struct NestedReturns {
        block_depth: usize,
        function_depth: usize,
        found: bool,
    }

    impl Visitor for NestedReturns {
        fn visit_function_body(&mut self, _body: &FunctionBody) {
            self.function_depth += 1;
        }
        fn visit_function_body_end(&mut self, _body: &FunctionBody) {
            self.function_depth -= 1;
        }
        fn visit_if(&mut self, _node: &If) {
            self.block_depth += 1;
        }
        fn visit_if_end(&mut self, _node: &If) {
            self.block_depth -= 1;
        }
        fn visit_while(&mut self, _node: &While) {
            self.block_depth += 1;
        }
        fn visit_while_end(&mut self, _node: &While) {
            self.block_depth -= 1;
        }
        fn visit_repeat(&mut self, _node: &Repeat) {
            self.block_depth += 1;
        }
        fn visit_repeat_end(&mut self, _node: &Repeat) {
            self.block_depth -= 1;
        }
        fn visit_numeric_for(&mut self, _node: &NumericFor) {
            self.block_depth += 1;
        }
        fn visit_numeric_for_end(&mut self, _node: &NumericFor) {
            self.block_depth -= 1;
        }
        fn visit_generic_for(&mut self, _node: &GenericFor) {
            self.block_depth += 1;
        }
        fn visit_generic_for_end(&mut self, _node: &GenericFor) {
            self.block_depth -= 1;
        }
        fn visit_do(&mut self, _node: &Do) {
            self.block_depth += 1;
        }
        fn visit_do_end(&mut self, _node: &Do) {
            self.block_depth -= 1;
        }
        fn visit_return(&mut self, _ret: &Return) {
            if self.function_depth == 0 && self.block_depth > 0 {
                self.found = true;
            }
        }
    }

    let mut check = NestedReturns::default();
    // Note: `Visitor::visit_block`'s default body is empty; `Visit::visit`
    // is what actually drives the traversal into nested statements.
    block.visit(&mut check);
    check.found
}
