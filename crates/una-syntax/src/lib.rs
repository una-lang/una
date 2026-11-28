//! Source files, spans, the lexer, the parser and the AST with trivia (`spec/01`, `spec/02`).
#![forbid(unsafe_code)]
#![deny(missing_docs)]

pub mod ast;
mod dump;
mod error;
mod intern;
pub mod ita;
pub mod keywords;
mod lexer;
mod parser;
mod rewrite;
mod source;
mod token;

pub use dump::dump;
pub use error::{SyntaxError, SyntaxRepair};
pub use intern::{Interner, Symbol};
pub use lexer::{Lexed, Rename, lex, lex_in, lex_project, rename_across};
pub use parser::{MAX_NESTING, Parsed, parse, parse_uses_clause};
pub use rewrite::{Declaration, DeclarationKind, declarations, declarations_in};
pub use source::{FileId, SourceDb, SourceFile, Span, Unread};
pub use token::{Comment, Keyword, KeywordSet, Token, TokenKind};
