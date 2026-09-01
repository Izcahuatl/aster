//! Small AST helpers shared across analysis modules.

use full_moon::tokenizer::{TokenReference, TokenType};

/// The identifier text of a token, if it is an identifier.
pub(crate) fn identifier_name(token: &TokenReference) -> Option<String> {
    if let TokenType::Identifier { identifier } = token.token().token_type() {
        Some(identifier.to_string())
    } else {
        None
    }
}

/// The value of a string-literal token (full_moon 2.2: `literal` excludes
/// the quotes/brackets; escape sequences are NOT unescaped).
pub(crate) fn string_literal(token: &TokenReference) -> Option<String> {
    if let TokenType::StringLiteral { literal, .. } = token.token().token_type() {
        Some(literal.to_string())
    } else {
        None
    }
}
