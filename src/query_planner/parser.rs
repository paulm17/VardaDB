//! Textual expression parser for GraphQL-facing expression syntax (M-D).
//!
//! Turns strings like `"age + 10 > 40"`, `"upper(name)"`, or
//! `"tags[0] contains \"rust\""` into planner [`LogicalExpr`] IR so clients
//! can express computed predicates, computed output fields, and sorts over
//! computed aliases without the fixed per-op filter objects.
//!
//! Grammar (precedence low -> high):
//! `or` < `and` < comparison (`== != > >= < <= in contains`) < `+ -` < `* / %` < unary (`- not`) < primary.
//!
//! The surface syntax cannot express subqueries, so parsed expressions never
//! contain [`LogicalExpr::Subquery`].

use crate::query_planner::ir::{
    BinaryOp, FieldPath, FieldSegment, LogicalExpr, QueryValue, UnaryOp,
};

#[derive(Debug)]
pub struct ParseError {
    pub message: String,
    pub position: usize,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "expression parse error at {}: {}", self.position, self.message)
    }
}

impl std::error::Error for ParseError {}

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Int(i64),
    Float(f64),
    Str(String),
    Ident(String),
    Op(String),
    LParen,
    RParen,
    LBracket,
    RBracket,
    Comma,
    Dot,
    Eof,
}

struct Lexer<'a> {
    src: &'a [u8],
    pos: usize,
}

impl<'a> Lexer<'a> {
    fn new(src: &'a str) -> Self {
        Lexer { src: src.as_bytes(), pos: 0 }
    }

    fn error(&self, message: &str) -> ParseError {
        ParseError { message: message.to_string(), position: self.pos }
    }

    fn next_token(&mut self) -> Result<Token, ParseError> {
        while self.pos < self.src.len() && self.src[self.pos].is_ascii_whitespace() {
            self.pos += 1;
        }
        if self.pos >= self.src.len() {
            return Ok(Token::Eof);
        }
        let b = self.src[self.pos];
        match b {
            b'(' => {
                self.pos += 1;
                Ok(Token::LParen)
            }
            b')' => {
                self.pos += 1;
                Ok(Token::RParen)
            }
            b'[' => {
                self.pos += 1;
                Ok(Token::LBracket)
            }
            b']' => {
                self.pos += 1;
                Ok(Token::RBracket)
            }
            b',' => {
                self.pos += 1;
                Ok(Token::Comma)
            }
            b'.' => {
                self.pos += 1;
                Ok(Token::Dot)
            }
            b'"' | b'\'' => {
                let quote = b;
                self.pos += 1;
                let mut out = String::new();
                loop {
                    if self.pos >= self.src.len() {
                        return Err(self.error("unterminated string literal"));
                    }
                    let c = self.src[self.pos];
                    if c == quote {
                        self.pos += 1;
                        break;
                    }
                    if c == b'\\' && self.pos + 1 < self.src.len() {
                        let esc = self.src[self.pos + 1];
                        out.push(match esc {
                            b'n' => '\n',
                            b't' => '\t',
                            b'r' => '\r',
                            other => other as char,
                        });
                        self.pos += 2;
                    } else {
                        out.push(c as char);
                        self.pos += 1;
                    }
                }
                Ok(Token::Str(out))
            }
            b'0'..=b'9' => {
                let start = self.pos;
                while self.pos < self.src.len() && self.src[self.pos].is_ascii_digit() {
                    self.pos += 1;
                }
                if self.pos + 1 < self.src.len()
                    && self.src[self.pos] == b'.'
                    && self.src[self.pos + 1].is_ascii_digit()
                {
                    self.pos += 1;
                    while self.pos < self.src.len() && self.src[self.pos].is_ascii_digit() {
                        self.pos += 1;
                    }
                    let text = std::str::from_utf8(&self.src[start..self.pos]).unwrap();
                    text.parse::<f64>()
                        .map(Token::Float)
                        .map_err(|_| self.error("invalid float literal"))
                } else {
                    let text = std::str::from_utf8(&self.src[start..self.pos]).unwrap();
                    text.parse::<i64>()
                        .map(Token::Int)
                        .map_err(|_| self.error("invalid integer literal"))
                }
            }
            c if c.is_ascii_alphabetic() || c == b'_' => {
                let start = self.pos;
                while self.pos < self.src.len()
                    && (self.src[self.pos].is_ascii_alphanumeric()
                        || self.src[self.pos] == b'_'
                        || self.src[self.pos] == b':')
                {
                    // '::' allowed inside idents for namespaced functions (math::sum)
                    if self.src[self.pos] == b':' {
                        if self.pos + 1 >= self.src.len() || self.src[self.pos + 1] != b':' {
                            return Err(self.error("expected '::' inside identifier"));
                        }
                        self.pos += 2;
                    } else {
                        self.pos += 1;
                    }
                }
                Ok(Token::Ident(
                    std::str::from_utf8(&self.src[start..self.pos]).unwrap().to_string(),
                ))
            }
            _ => {
                let two = self.src.get(self.pos..self.pos + 2);
                let op = match two {
                    Some(b"==") => "==",
                    Some(b"!=") => "!=",
                    Some(b">=") => ">=",
                    Some(b"<=") => "<=",
                    _ => match b {
                        b'>' => ">",
                        b'<' => "<",
                        b'+' => "+",
                        b'-' => "-",
                        b'*' => "*",
                        b'/' => "/",
                        b'%' => "%",
                        _ => return Err(self.error("unexpected character")),
                    },
                };
                self.pos += op.len();
                Ok(Token::Op(op.to_string()))
            }
        }
    }
}

struct Parser {
    tokens: Vec<(Token, usize)>,
    idx: usize,
}

impl Parser {
    fn new(src: &str) -> Result<Self, ParseError> {
        let mut lexer = Lexer::new(src);
        let mut tokens = Vec::new();
        loop {
            let pos = lexer.pos;
            let tok = lexer.next_token()?;
            let eof = tok == Token::Eof;
            tokens.push((tok, pos));
            if eof {
                break;
            }
        }
        Ok(Parser { tokens, idx: 0 })
    }

    fn peek(&self) -> &Token {
        &self.tokens[self.idx].0
    }

    fn position(&self) -> usize {
        self.tokens[self.idx].1
    }

    fn bump(&mut self) -> Token {
        let tok = self.tokens[self.idx].0.clone();
        if self.idx + 1 < self.tokens.len() {
            self.idx += 1;
        }
        tok
    }

    fn eat_op(&mut self, ops: &[&str]) -> Option<String> {
        if let Token::Op(op) = self.peek() {
            if ops.contains(&op.as_str()) {
                return Some(self.bump().expect_op());
            }
        }
        None
    }

    fn eat_keyword(&mut self, kw: &str) -> bool {
        if let Token::Ident(id) = self.peek() {
            if id == kw {
                self.bump();
                return true;
            }
        }
        false
    }

    fn expect_ident(&mut self) -> Result<String, ParseError> {
        match self.peek() {
            Token::Ident(_) => {
                let tok = self.bump();
                Ok(match tok {
                    Token::Ident(s) => s,
                    _ => unreachable!(),
                })
            }
            _ => Err(ParseError {
                message: format!("expected identifier, found {:?}", self.peek()),
                position: self.position(),
            }),
        }
    }
}

impl Token {
    fn expect_op(self) -> String {
        match self {
            Token::Op(op) => op,
            other => panic!("expected op token, got {:?}", other),
        }
    }
}

/// Parse a textual expression into planner IR.
pub fn parse_expression(src: &str) -> Result<LogicalExpr, ParseError> {
    let mut parser = Parser::new(src)?;
    let expr = parser.parse_or()?;
    if !matches!(parser.peek(), Token::Eof) {
        return Err(ParseError {
            message: format!("unexpected trailing input near {:?}", parser.peek()),
            position: parser.position(),
        });
    }
    Ok(expr)
}

impl Parser {
    fn parse_or(&mut self) -> Result<LogicalExpr, ParseError> {
        let mut left = self.parse_and()?;
        while self.eat_keyword("or") {
            let right = self.parse_and()?;
            left = LogicalExpr::Binary {
                left: Box::new(left),
                op: BinaryOp::Or,
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn parse_and(&mut self) -> Result<LogicalExpr, ParseError> {
        let mut left = self.parse_comparison()?;
        while self.eat_keyword("and") {
            let right = self.parse_comparison()?;
            left = LogicalExpr::Binary {
                left: Box::new(left),
                op: BinaryOp::And,
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn parse_comparison(&mut self) -> Result<LogicalExpr, ParseError> {
        let left = self.parse_additive()?;
        let op = if let Some(op) = self.eat_op(&["==", "!=", ">=", "<=", ">", "<"]) {
            match op.as_str() {
                "==" => BinaryOp::Eq,
                "!=" => BinaryOp::Ne,
                ">=" => BinaryOp::Ge,
                "<=" => BinaryOp::Le,
                ">" => BinaryOp::Gt,
                _ => BinaryOp::Lt,
            }
        } else if self.eat_keyword("in") {
            BinaryOp::In
        } else if self.eat_keyword("contains") {
            BinaryOp::Contains
        } else {
            return Ok(left);
        };
        let right = self.parse_additive()?;
        Ok(LogicalExpr::Binary {
            left: Box::new(left),
            op,
            right: Box::new(right),
        })
    }

    fn parse_additive(&mut self) -> Result<LogicalExpr, ParseError> {
        let mut left = self.parse_multiplicative()?;
        while let Some(op) = self.eat_op(&["+", "-"]) {
            let right = self.parse_multiplicative()?;
            left = LogicalExpr::Binary {
                left: Box::new(left),
                op: if op == "+" { BinaryOp::Add } else { BinaryOp::Sub },
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn parse_multiplicative(&mut self) -> Result<LogicalExpr, ParseError> {
        let mut left = self.parse_unary()?;
        while let Some(op) = self.eat_op(&["*", "/", "%"]) {
            let right = self.parse_unary()?;
            left = LogicalExpr::Binary {
                left: Box::new(left),
                op: match op.as_str() {
                    "*" => BinaryOp::Mul,
                    "/" => BinaryOp::Div,
                    _ => BinaryOp::Mod,
                },
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn parse_unary(&mut self) -> Result<LogicalExpr, ParseError> {
        if self.eat_keyword("not") {
            let inner = self.parse_unary()?;
            return Ok(LogicalExpr::Unary {
                op: UnaryOp::Not,
                expr: Box::new(inner),
            });
        }
        if self.eat_op(&["-"]).is_some() {
            let inner = self.parse_unary()?;
            return Ok(LogicalExpr::Unary {
                op: UnaryOp::Neg,
                expr: Box::new(inner),
            });
        }
        self.parse_primary()
    }

    fn parse_primary(&mut self) -> Result<LogicalExpr, ParseError> {
        let pos = self.position();
        match self.peek().clone() {
            Token::Int(v) => {
                self.bump();
                Ok(LogicalExpr::Value(QueryValue::Int(v)))
            }
            Token::Float(v) => {
                self.bump();
                Ok(LogicalExpr::Value(QueryValue::Float(v)))
            }
            Token::Str(s) => {
                self.bump();
                Ok(LogicalExpr::Value(QueryValue::String(s)))
            }
            Token::LParen => {
                self.bump();
                let inner = self.parse_or()?;
                match self.peek() {
                    Token::RParen => {
                        self.bump();
                        Ok(inner)
                    }
                    other => Err(ParseError {
                        message: format!("expected ')', found {:?}", other),
                        position: self.position(),
                    }),
                }
            }
            Token::Ident(name) => {
                self.bump();
                // Function call
                if matches!(self.peek(), Token::LParen) {
                    self.bump();
                    let mut args = Vec::new();
                    if !matches!(self.peek(), Token::RParen) {
                        loop {
                            args.push(self.parse_or()?);
                            match self.peek() {
                                Token::Comma => {
                                    self.bump();
                                }
                                Token::RParen => break,
                                other => {
                                    return Err(ParseError {
                                        message: format!(
                                            "expected ',' or ')' in argument list, found {:?}",
                                            other
                                        ),
                                        position: self.position(),
                                    })
                                }
                            }
                        }
                    }
                    match self.peek() {
                        Token::RParen => {
                            self.bump();
                            Ok(LogicalExpr::Function { name, args })
                        }
                        other => Err(ParseError {
                            message: format!("expected ')' after arguments, found {:?}", other),
                            position: self.position(),
                        }),
                    }
                } else {
                    Ok(LogicalExpr::Field(Self::parse_path(pos, &name, self)?))
                }
            }
            other => Err(ParseError {
                message: format!("unexpected token {:?}", other),
                position: pos,
            }),
        }
    }

    /// Dotted/indexed path continuation: `profile.age`, `tags[0]`.
    /// The leading identifier is already consumed; index brackets may also
    /// follow nested segments (`a.b[2]`).
    fn parse_path(
        _pos: usize,
        head: &str,
        parser: &mut Parser,
    ) -> Result<FieldPath, ParseError> {
        let mut segments = vec![FieldSegment::Field(head.to_string())];
        loop {
            match parser.peek() {
                Token::Dot => {
                    parser.bump();
                    let seg = parser.expect_ident()?;
                    segments.push(FieldSegment::Field(seg));
                }
                Token::LBracket => {
                    parser.bump();
                    match parser.peek().clone() {
                        Token::Int(i) if i >= 0 => {
                            parser.bump();
                            match parser.peek() {
                                Token::RBracket => {
                                    parser.bump();
                                    segments.push(FieldSegment::Index(i as usize));
                                }
                                other => {
                                    return Err(ParseError {
                                        message: format!("expected ']', found {:?}", other),
                                        position: parser.position(),
                                    })
                                }
                            }
                        }
                        other => {
                            return Err(ParseError {
                                message: format!("expected list index, found {:?}", other),
                                position: parser.position(),
                            })
                        }
                    }
                }
                _ => break,
            }
        }
        Ok(FieldPath { segments })
    }
}

/// Collect the distinct root field names referenced by an expression.
/// Used by resolvers to prefetch exactly the stored fields an expression
/// needs before evaluating it against a row.
pub fn root_fields(expr: &LogicalExpr, out: &mut Vec<String>) {
    match expr {
        LogicalExpr::Value(_) => {}
        LogicalExpr::Field(path) => {
            if let Some(root) = path.segments.first() {
                if let FieldSegment::Field(name) = root {
                    if !out.iter().any(|existing| existing == name) {
                        out.push(name.clone());
                    }
                }
            }
        }
        LogicalExpr::Binary { left, right, .. } => {
            root_fields(left, out);
            root_fields(right, out);
        }
        LogicalExpr::Unary { expr: inner, .. } => root_fields(inner, out),
        LogicalExpr::Function { args, .. } => {
            for arg in args {
                root_fields(arg, out);
            }
        }
        LogicalExpr::Subquery(_) => {}
    }
}
