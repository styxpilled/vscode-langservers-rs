//! Port of `microsoft/node-jsonc-parser`.
//!
//! All offsets, lengths and character positions are UTF-8 byte offsets.
//! Upstream uses UTF-16 code units.

mod edit;
mod format;
mod parser;
mod scanner;

use serde_json::{Number, Value};

pub use edit::{EditError, InsertionIndex, ModificationOptions, apply_edits, modify};
pub use format::{FormattingOptions, format};
pub use parser::{
    Location, Node, NodeId, Tree, Visitor, get_location, parse, parse_tree, strip_comments, visit,
};
pub use scanner::Scanner;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum ScanError {
    None = 0,
    UnexpectedEndOfComment = 1,
    UnexpectedEndOfString = 2,
    UnexpectedEndOfNumber = 3,
    InvalidUnicode = 4,
    InvalidEscapeCharacter = 5,
    InvalidCharacter = 6,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum SyntaxKind {
    OpenBraceToken = 1,
    CloseBraceToken = 2,
    OpenBracketToken = 3,
    CloseBracketToken = 4,
    CommaToken = 5,
    ColonToken = 6,
    NullKeyword = 7,
    TrueKeyword = 8,
    FalseKeyword = 9,
    StringLiteral = 10,
    NumericLiteral = 11,
    LineCommentTrivia = 12,
    BlockCommentTrivia = 13,
    LineBreakTrivia = 14,
    Trivia = 15,
    Unknown = 16,
    EOF = 17,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum ParseErrorCode {
    InvalidSymbol = 1,
    InvalidNumberFormat = 2,
    PropertyNameExpected = 3,
    ValueExpected = 4,
    ColonExpected = 5,
    CommaExpected = 6,
    CloseBraceExpected = 7,
    CloseBracketExpected = 8,
    EndOfFileExpected = 9,
    InvalidCommentToken = 10,
    UnexpectedEndOfComment = 11,
    UnexpectedEndOfString = 12,
    UnexpectedEndOfNumber = 13,
    InvalidUnicode = 14,
    InvalidEscapeCharacter = 15,
    InvalidCharacter = 16,
    /// Not in upstream. An array or object deeper than `MAX_DEPTH`.
    NestingTooDeep = 17,
}

/// Deepest nesting of arrays and objects that the parsers read. A deeper
/// value is read as an empty array or object, so recursive walks of the result
/// have a bounded depth.
pub const MAX_DEPTH: usize = 1000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParseError {
    pub error: ParseErrorCode,
    pub offset: usize,
    pub length: usize,
    pub start_line: usize,
    pub start_character: usize,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ParseOptions {
    pub disallow_comments: bool,
    pub allow_trailing_comma: bool,
    pub allow_empty_content: bool,
}

/// Location of a token, as given to [`Visitor`] callbacks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span {
    pub offset: usize,
    pub length: usize,
    pub start_line: usize,
    pub start_character: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Segment {
    Property(String),
    Index(usize),
}

impl From<&str> for Segment {
    fn from(name: &str) -> Self {
        Segment::Property(name.to_owned())
    }
}

impl From<String> for Segment {
    fn from(name: String) -> Self {
        Segment::Property(name)
    }
}

impl From<usize> for Segment {
    fn from(index: usize) -> Self {
        Segment::Index(index)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NodeType {
    Object,
    Array,
    Property,
    String,
    Number,
    Boolean,
    Null,
}

impl NodeType {
    pub fn as_str(self) -> &'static str {
        match self {
            NodeType::Object => "object",
            NodeType::Array => "array",
            NodeType::Property => "property",
            NodeType::String => "string",
            NodeType::Number => "number",
            NodeType::Boolean => "boolean",
            NodeType::Null => "null",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Literal {
    Null,
    Boolean(bool),
    Number(f64),
    String(String),
}

impl Literal {
    pub fn node_type(&self) -> NodeType {
        match self {
            Literal::Null => NodeType::Null,
            Literal::Boolean(_) => NodeType::Boolean,
            Literal::Number(_) => NodeType::Number,
            Literal::String(_) => NodeType::String,
        }
    }

    pub fn to_json(&self) -> Value {
        match self {
            Literal::Null => Value::Null,
            Literal::Boolean(b) => Value::Bool(*b),
            Literal::Number(n) => number_to_json(*n),
            Literal::String(s) => Value::String(s.clone()),
        }
    }
}

/// Whole numbers in the exact `f64` range become integers, as `JSON.stringify` writes them.
/// Infinite values become `null`, as `JSON.stringify` writes them.
pub fn number_to_json(n: f64) -> Value {
    const MAX_SAFE: f64 = 9_007_199_254_740_992.0;
    if n.fract() == 0.0 && n.abs() < MAX_SAFE {
        Value::from(n as i64)
    } else {
        Number::from_f64(n).map_or(Value::Null, Value::Number)
    }
}

/// A text replacement. An empty `length` is an insertion; an empty `content` is a removal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Edit {
    pub offset: usize,
    pub length: usize,
    pub content: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Range {
    pub offset: usize,
    pub length: usize,
}
