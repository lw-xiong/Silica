//! Parsing a statement into Obsidian's own tree.
//!
//! The grammar, lowest binding first. Every level is left-associative, which
//! was read off the trees rather than assumed: `1 - 2 + 3` parses as
//! `(1-2)+3`, `a == b == c` as `(a==b)==c`.
//!
//! ```text
//! expr        := or
//! or          := and ( "||" and )*
//! and         := comparison ( "&&" comparison )*
//! comparison  := additive ( ( "<" | "<=" | ">" | ">=" | "==" | "!=" ) additive )*
//! additive    := multiplicative ( ( "+" | "-" ) multiplicative )*
//! multiplicative := unary ( ( "*" | "/" | "%" ) unary )*
//! unary       := ( "-" | "!" ) unary | postfix
//! postfix     := primary ( "." name [ "(" args ")" ] | "[" expr "]" )*
//! primary     := number | string | "true" | "false" | "null" | regexp
//!              | ident [ "(" args ")" ] | "(" expr ")" | "[" [ expr ( "," expr )* ] "]"
//! args        := [ expr ( "," expr )* ]
//! ```
//!
//! Two things the grammar does not show, both observed:
//!
//! - `&&` and `||` build a [`Expr::Comparison`] node, not a node of their own.
//!   Obsidian's tree carries all eight operators under one `comparison` type.
//! - A `-` in front of a number literal is folded into the literal, so
//!   `-1 + 2` has a `primitive -1` on its left and only `- a` becomes a
//!   [`Expr::Negate`].
//!
//! There is no lambda. `filter`, `map` and `reduce` take an ordinary
//! expression evaluated per element, with the idents `value`, `index` and (for
//! `reduce`) `acc` bound around it — `=>` does not parse in any form. That is
//! a fact about evaluation, not syntax, so nothing here treats those names
//! specially; it is recorded because the shape of the call is the reason the
//! grammar needs no lambda production.

use std::fmt;

/// A literal value. Numbers are IEEE doubles, as they are in Obsidian.
#[derive(Debug, Clone, PartialEq)]
pub enum Literal {
    Number(f64),
    String(String),
    Bool(bool),
    Null,
}

/// The binary operators, all of which Obsidian stores on one of two node
/// types: the first three on `arithmetic`, the rest on `comparison`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOp {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
    Ne,
    And,
    Or,
}

impl BinaryOp {
    /// How Obsidian writes this operator.
    pub fn as_str(self) -> &'static str {
        match self {
            BinaryOp::Add => "+",
            BinaryOp::Sub => "-",
            BinaryOp::Mul => "*",
            BinaryOp::Div => "/",
            BinaryOp::Rem => "%",
            BinaryOp::Lt => "<",
            BinaryOp::Le => "<=",
            BinaryOp::Gt => ">",
            BinaryOp::Ge => ">=",
            BinaryOp::Eq => "==",
            BinaryOp::Ne => "!=",
            BinaryOp::And => "&&",
            BinaryOp::Or => "||",
        }
    }

    /// Whether Obsidian files this operator under `arithmetic` rather than
    /// `comparison`.
    pub fn is_arithmetic(self) -> bool {
        matches!(
            self,
            BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div | BinaryOp::Rem
        )
    }
}

/// A parsed statement. The variants are Obsidian's thirteen node types, with
/// its own names given where they differ.
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    /// `primitive`
    Primitive(Literal),
    /// `ident`. `this` is one of these and not a node of its own.
    Ident(String),
    /// `array`
    Array(Vec<Expr>),
    /// `regexp`. Obsidian holds a live RegExp here, so only the literal's own
    /// two parts survive into anything observable.
    Regexp { pattern: String, flags: String },
    /// `paren_expr`. Kept rather than flattened, because Obsidian keeps it.
    Paren(Box<Expr>),
    /// `arithmetic` when the operator is one, `comparison` otherwise.
    Binary {
        op: BinaryOp,
        left: Box<Expr>,
        right: Box<Expr>,
    },
    /// `negate`
    Negate(Box<Expr>),
    /// `not`
    Not(Box<Expr>),
    /// `function`. `subject` is `None` for a global call, which is what
    /// Obsidian's `subject: null` means.
    Function {
        name: String,
        subject: Option<Box<Expr>>,
        args: Vec<Expr>,
    },
    /// `object_access`: a `.name` that is not a call. The index is the name as
    /// written — resolution is case-insensitive, but that is the evaluator's
    /// business and the tree keeps what the author typed.
    ObjectAccess { object: Box<Expr>, index: String },
    /// `array_access`: a `[expr]`.
    ArrayAccess { array: Box<Expr>, index: Box<Expr> },
    /// `invalid`. A failed parse is a node in Obsidian, not an exception: the
    /// statement is kept with its message and the file is written back intact.
    Invalid { value: String, error: String },
}

/// Why a statement did not parse.
#[derive(Debug, Clone, PartialEq)]
pub struct ParseError {
    pub message: String,
    /// Byte offset into the statement.
    pub position: usize,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} (position {})", self.message, self.position)
    }
}

impl std::error::Error for ParseError {}

impl ParseError {
    fn new(message: impl Into<String>, position: usize) -> ParseError {
        ParseError {
            message: message.into(),
            position,
        }
    }

    fn unparseable(position: usize) -> ParseError {
        ParseError::new("Unable to parse formula.", position)
    }
}

/// Parses a statement.
pub fn parse(source: &str) -> Result<Expr, ParseError> {
    let mut p = Parser::new(source);
    p.skip_space();
    let expr = p.parse_expr()?;
    p.skip_space();
    if p.pos < p.bytes.len() {
        return Err(ParseError::unparseable(0));
    }
    Ok(expr)
}

/// Parses a statement the way Obsidian stores one: a failure is an
/// [`Expr::Invalid`] carrying the original text, not an error.
pub fn parse_node(source: &str) -> Expr {
    match parse(source) {
        Ok(expr) => expr,
        Err(e) => Expr::Invalid {
            value: source.to_string(),
            error: e.to_string(),
        },
    }
}

// ---------------------------------------------------------------------------

struct Parser<'a> {
    bytes: &'a [u8],
    src: &'a str,
    pos: usize,
}

impl<'a> Parser<'a> {
    fn new(src: &'a str) -> Parser<'a> {
        Parser {
            bytes: src.as_bytes(),
            src,
            pos: 0,
        }
    }

    fn skip_space(&mut self) {
        while self.pos < self.bytes.len() && self.bytes[self.pos].is_ascii_whitespace() {
            self.pos += 1;
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    /// Whether the source continues with `s` here, and consumes it if so.
    fn eat(&mut self, s: &str) -> bool {
        if self.src[self.pos..].starts_with(s) {
            self.pos += s.len();
            true
        } else {
            false
        }
    }

    fn parse_expr(&mut self) -> Result<Expr, ParseError> {
        self.parse_binary(0)
    }

    /// One precedence level, left-associative, climbing to the next.
    fn parse_binary(&mut self, level: usize) -> Result<Expr, ParseError> {
        const LEVELS: &[&[(&str, BinaryOp)]] = &[
            &[("||", BinaryOp::Or)],
            &[("&&", BinaryOp::And)],
            // The two-character forms come first, or `<=` lexes as `<`.
            &[
                ("<=", BinaryOp::Le),
                (">=", BinaryOp::Ge),
                ("==", BinaryOp::Eq),
                ("!=", BinaryOp::Ne),
                ("<", BinaryOp::Lt),
                (">", BinaryOp::Gt),
            ],
            &[("+", BinaryOp::Add), ("-", BinaryOp::Sub)],
            &[
                ("*", BinaryOp::Mul),
                ("/", BinaryOp::Div),
                ("%", BinaryOp::Rem),
            ],
        ];
        if level == LEVELS.len() {
            return self.parse_unary();
        }
        let mut left = self.parse_binary(level + 1)?;
        loop {
            self.skip_space();
            let Some((text, op)) = LEVELS[level]
                .iter()
                .find(|(text, _)| self.src[self.pos..].starts_with(text))
                .copied()
            else {
                return Ok(left);
            };
            // `/` opens a regexp only where a value may start, which is never
            // here: a division's left side has just been read.
            self.pos += text.len();
            self.skip_space();
            let right = self.parse_binary(level + 1)?;
            left = Expr::Binary {
                op,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
    }

    fn parse_unary(&mut self) -> Result<Expr, ParseError> {
        self.skip_space();
        let start = self.pos;
        if self.eat("!") {
            // `!=` is a comparison, and the level above has already had its
            // chance at it; reaching one here is a statement like `!= 1`.
            if self.peek() == Some(b'=') {
                return Err(ParseError::unparseable(start));
            }
            self.skip_space();
            return Ok(Expr::Not(Box::new(self.parse_unary()?)));
        }
        if self.eat("-") {
            self.skip_space();
            // A minus in front of a number is part of the literal, which is
            // why `-1 + 2` has a `primitive -1` on its left.
            if self.peek().is_some_and(|c| c.is_ascii_digit() || c == b'.') {
                let n = self.parse_number()?;
                return self.parse_postfix(Expr::Primitive(Literal::Number(-n)));
            }
            return Ok(Expr::Negate(Box::new(self.parse_unary()?)));
        }
        self.parse_postfix_from_primary()
    }

    fn parse_postfix_from_primary(&mut self) -> Result<Expr, ParseError> {
        let primary = self.parse_primary()?;
        self.parse_postfix(primary)
    }

    fn parse_postfix(&mut self, mut subject: Expr) -> Result<Expr, ParseError> {
        loop {
            self.skip_space();
            if self.eat(".") {
                self.skip_space();
                let start = self.pos;
                let name = self.parse_name();
                if name.is_empty() {
                    return Err(ParseError::unparseable(start));
                }
                self.skip_space();
                if self.peek() == Some(b'(') {
                    let args = self.parse_args()?;
                    subject = Expr::Function {
                        name,
                        subject: Some(Box::new(subject)),
                        args,
                    };
                } else {
                    subject = Expr::ObjectAccess {
                        object: Box::new(subject),
                        index: name,
                    };
                }
                continue;
            }
            if self.peek() == Some(b'[') {
                self.pos += 1;
                self.skip_space();
                let index = self.parse_expr()?;
                self.skip_space();
                if !self.eat("]") {
                    return Err(ParseError::unparseable(self.pos));
                }
                subject = Expr::ArrayAccess {
                    array: Box::new(subject),
                    index: Box::new(index),
                };
                continue;
            }
            return Ok(subject);
        }
    }

    fn parse_args(&mut self) -> Result<Vec<Expr>, ParseError> {
        // The caller has checked the `(`.
        self.pos += 1;
        let mut args = Vec::new();
        self.skip_space();
        if self.eat(")") {
            return Ok(args);
        }
        loop {
            self.skip_space();
            args.push(self.parse_expr()?);
            self.skip_space();
            if self.eat(",") {
                continue;
            }
            if self.eat(")") {
                return Ok(args);
            }
            return Err(ParseError::unparseable(self.pos));
        }
    }

    fn parse_primary(&mut self) -> Result<Expr, ParseError> {
        self.skip_space();
        let start = self.pos;
        let Some(c) = self.peek() else {
            return Err(ParseError::unparseable(start));
        };
        match c {
            b'(' => {
                self.pos += 1;
                self.skip_space();
                let inner = self.parse_expr()?;
                self.skip_space();
                if !self.eat(")") {
                    return Err(ParseError::unparseable(self.pos));
                }
                Ok(Expr::Paren(Box::new(inner)))
            }
            b'[' => {
                self.pos += 1;
                let mut elements = Vec::new();
                self.skip_space();
                if self.eat("]") {
                    return Ok(Expr::Array(elements));
                }
                loop {
                    self.skip_space();
                    elements.push(self.parse_expr()?);
                    self.skip_space();
                    if self.eat(",") {
                        continue;
                    }
                    if self.eat("]") {
                        return Ok(Expr::Array(elements));
                    }
                    return Err(ParseError::unparseable(self.pos));
                }
            }
            b'"' | b'\'' => Ok(Expr::Primitive(Literal::String(self.parse_string()?))),
            b'/' => self.parse_regexp(),
            c if c.is_ascii_digit() || c == b'.' => {
                let n = self.parse_number()?;
                Ok(Expr::Primitive(Literal::Number(n)))
            }
            c if is_name_start(c) => {
                let name = self.parse_name();
                self.skip_space();
                if self.peek() == Some(b'(') {
                    let args = self.parse_args()?;
                    return Ok(Expr::Function {
                        name,
                        subject: None,
                        args,
                    });
                }
                // Only these three spell a primitive, and each is
                // case-sensitive here: `TRUE` is an ident that evaluates null.
                Ok(match name.as_str() {
                    "true" => Expr::Primitive(Literal::Bool(true)),
                    "false" => Expr::Primitive(Literal::Bool(false)),
                    "null" => Expr::Primitive(Literal::Null),
                    _ => Expr::Ident(name),
                })
            }
            _ => Err(ParseError::unparseable(start)),
        }
    }

    fn parse_name(&mut self) -> String {
        let start = self.pos;
        while self.peek().is_some_and(is_name_part) {
            self.pos += 1;
        }
        self.src[start..self.pos].to_string()
    }

    /// Numbers as the lexer takes them: `1`, `1.5`, `.5`, `1.` and `01` all
    /// parse; `1e3`, `0x10`, `1_000` and a leading `+` do not.
    fn parse_number(&mut self) -> Result<f64, ParseError> {
        let start = self.pos;
        while self.peek().is_some_and(|c| c.is_ascii_digit()) {
            self.pos += 1;
        }
        if self.peek() == Some(b'.') {
            self.pos += 1;
            while self.peek().is_some_and(|c| c.is_ascii_digit()) {
                self.pos += 1;
            }
        }
        let text = &self.src[start..self.pos];
        if text.is_empty() || text == "." {
            return Err(ParseError::unparseable(start));
        }
        // An exponent or a hex digit running on from the number is not part of
        // the grammar, and leaving it for the next token would silently accept
        // `1e3` as `1` followed by `e3`.
        if self.peek().is_some_and(is_name_part) {
            return Err(ParseError::unparseable(self.pos));
        }
        text.trim_end_matches('.')
            .parse::<f64>()
            .map_err(|_| ParseError::unparseable(start))
    }

    /// A quoted string, with JSON's escapes. Obsidian hands the token to
    /// `JSON.parse`, so an unknown escape is an error rather than a literal
    /// backslash.
    fn parse_string(&mut self) -> Result<String, ParseError> {
        let quote = self.bytes[self.pos];
        let open = self.pos;
        self.pos += 1;
        let mut out = String::new();
        loop {
            let Some(c) = self.peek() else {
                return Err(ParseError::new("Unterminated string", open));
            };
            if c == quote {
                self.pos += 1;
                return Ok(out);
            }
            if c == b'\\' {
                let at = self.pos;
                self.pos += 1;
                let Some(esc) = self.peek() else {
                    return Err(ParseError::new("Unterminated string", open));
                };
                self.pos += 1;
                match esc {
                    b'"' => out.push('"'),
                    b'\'' => out.push('\''),
                    b'\\' => out.push('\\'),
                    b'/' => out.push('/'),
                    b'n' => out.push('\n'),
                    b't' => out.push('\t'),
                    b'r' => out.push('\r'),
                    b'b' => out.push('\u{8}'),
                    b'f' => out.push('\u{c}'),
                    b'u' => {
                        let hex = self
                            .src
                            .get(self.pos..self.pos + 4)
                            .ok_or_else(|| ParseError::new("Unterminated string", open))?;
                        let code = u32::from_str_radix(hex, 16).map_err(|_| {
                            ParseError::new(
                                format!("Bad escaped character in JSON at position {at}"),
                                at,
                            )
                        })?;
                        self.pos += 4;
                        out.push(char::from_u32(code).unwrap_or('\u{fffd}'));
                    }
                    _ => {
                        return Err(ParseError::new(
                            format!("Bad escaped character in JSON at position {at}"),
                            at,
                        ))
                    }
                }
                continue;
            }
            // Multi-byte characters pass through whole.
            let ch = self.src[self.pos..]
                .chars()
                .next()
                .ok_or_else(|| ParseError::new("Unterminated string", open))?;
            out.push(ch);
            self.pos += ch.len_utf8();
        }
    }

    fn parse_regexp(&mut self) -> Result<Expr, ParseError> {
        let open = self.pos;
        self.pos += 1;
        let start = self.pos;
        let mut escaped = false;
        loop {
            let Some(c) = self.peek() else {
                return Err(ParseError::unparseable(open));
            };
            if escaped {
                escaped = false;
            } else if c == b'\\' {
                escaped = true;
            } else if c == b'/' {
                break;
            }
            self.pos += 1;
        }
        let pattern = self.src[start..self.pos].to_string();
        self.pos += 1;
        let flags_start = self.pos;
        while self.peek().is_some_and(|c| c.is_ascii_alphabetic()) {
            self.pos += 1;
        }
        Ok(Expr::Regexp {
            pattern,
            flags: self.src[flags_start..self.pos].to_string(),
        })
    }
}

fn is_name_start(c: u8) -> bool {
    c.is_ascii_alphabetic() || c == b'_' || c == b'$' || c >= 0x80
}

fn is_name_part(c: u8) -> bool {
    is_name_start(c) || c.is_ascii_digit()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn num(n: f64) -> Expr {
        Expr::Primitive(Literal::Number(n))
    }
    fn ident(s: &str) -> Expr {
        Expr::Ident(s.to_string())
    }

    /// A compact rendering of the tree's shape, for asserting associativity
    /// without writing the nesting out by hand.
    fn shape(e: &Expr) -> String {
        match e {
            Expr::Primitive(Literal::Number(n)) => format!("{n}"),
            Expr::Primitive(Literal::String(s)) => format!("{s:?}"),
            Expr::Primitive(Literal::Bool(b)) => b.to_string(),
            Expr::Primitive(Literal::Null) => "null".into(),
            Expr::Ident(id) => id.clone(),
            Expr::Array(items) => format!(
                "[{}]",
                items.iter().map(shape).collect::<Vec<_>>().join(", ")
            ),
            Expr::Regexp { pattern, flags } => format!("/{pattern}/{flags}"),
            Expr::Paren(inner) => format!("({})", shape(inner)),
            Expr::Binary { op, left, right } => {
                format!("({} {} {})", shape(left), op.as_str(), shape(right))
            }
            Expr::Negate(e) => format!("(- {})", shape(e)),
            Expr::Not(e) => format!("(! {})", shape(e)),
            Expr::Function {
                name,
                subject,
                args,
            } => {
                let args = args.iter().map(shape).collect::<Vec<_>>().join(", ");
                match subject {
                    Some(s) => format!("{}.{name}({args})", shape(s)),
                    None => format!("{name}({args})"),
                }
            }
            Expr::ObjectAccess { object, index } => format!("{}.{index}", shape(object)),
            Expr::ArrayAccess { array, index } => {
                format!("{}[{}]", shape(array), shape(index))
            }
            Expr::Invalid { value, .. } => format!("<invalid {value:?}>"),
        }
    }

    fn s(src: &str) -> String {
        shape(&parse(src).unwrap_or_else(|e| panic!("{src:?} did not parse: {e}")))
    }

    /// Every row of the observed precedence table, which was read off real
    /// parse trees. `tools/obsidian-probe/results/2026-10-25-bases-expr.json`,
    /// step `e03-precedence`.
    #[test]
    fn precedence_is_the_observed_table() {
        // postfix
        assert_eq!(s("a.b.c"), "a.b.c");
        assert_eq!(s("a.b().c()"), "a.b().c()");
        assert_eq!(s("a.b[0]"), "a.b[0]");
        // unary over postfix, and under the binaries
        assert_eq!(s("!a.b"), "(! a.b)");
        assert_eq!(s("-a.b"), "(- a.b)");
        assert_eq!(s("!a && b"), "((! a) && b)");
        assert_eq!(s("!a == b"), "((! a) == b)");
        // multiplicative over additive
        assert_eq!(s("1 + 2 * 3"), "(1 + (2 * 3))");
        assert_eq!(s("1 % 2 * 3"), "((1 % 2) * 3)");
        // additive left-associative
        assert_eq!(s("1 - 2 + 3"), "((1 - 2) + 3)");
        // comparison under additive, and left-associative
        assert_eq!(s("a + b > c + d"), "((a + b) > (c + d))");
        assert_eq!(s("a == b == c"), "((a == b) == c)");
        assert_eq!(s("a < b < c"), "((a < b) < c)");
        assert_eq!(s("a >= b <= c"), "((a >= b) <= c)");
        // and under comparison, or under and
        assert_eq!(s("a && b == c"), "(a && (b == c))");
        assert_eq!(s("a && b || c"), "((a && b) || c)");
        assert_eq!(s("a || b && c"), "(a || (b && c))");
    }

    /// A minus in front of a number is part of the literal; in front of
    /// anything else it is a `negate` node.
    #[test]
    fn a_negative_number_is_folded_into_the_literal() {
        assert_eq!(
            parse("-1 + 2").unwrap(),
            Expr::Binary {
                op: BinaryOp::Add,
                left: Box::new(num(-1.0)),
                right: Box::new(num(2.0)),
            }
        );
        assert_eq!(s("- 1 * 2"), "(-1 * 2)");
        assert_eq!(s("-a"), "(- a)");
        assert_eq!(s("-(1)"), "(- (1))");
    }

    /// The node Obsidian builds for each shape, checked through the variant
    /// rather than the rendering.
    #[test]
    fn the_node_types_are_obsidians() {
        assert!(matches!(
            parse("now()").unwrap(),
            Expr::Function { subject: None, .. }
        ));
        assert!(matches!(
            parse("a.b()").unwrap(),
            Expr::Function {
                subject: Some(_),
                ..
            }
        ));
        assert!(matches!(parse("a.b").unwrap(), Expr::ObjectAccess { .. }));
        assert!(matches!(parse("a[0]").unwrap(), Expr::ArrayAccess { .. }));
        assert!(matches!(parse("(1)").unwrap(), Expr::Paren(_)));
        assert!(matches!(parse("[1, 2]").unwrap(), Expr::Array(_)));
        assert!(matches!(parse("/re/i").unwrap(), Expr::Regexp { .. }));
        // `this` is an ident, not a node type of its own.
        assert_eq!(parse("this").unwrap(), ident("this"));
        assert_eq!(s("this.file.name"), "this.file.name");
    }

    /// `&&` and `||` are comparison nodes in Obsidian's tree, and `+` and the
    /// rest are arithmetic ones.
    #[test]
    fn one_node_type_carries_both_comparison_and_boolean_operators() {
        for (src, arithmetic) in [
            ("a + b", true),
            ("a % b", true),
            ("a == b", false),
            ("a < b", false),
            ("a && b", false),
            ("a || b", false),
        ] {
            let Expr::Binary { op, .. } = parse(src).unwrap() else {
                panic!("{src} is not a binary node");
            };
            assert_eq!(op.is_arithmetic(), arithmetic, "{src}");
        }
    }

    #[test]
    fn literals_are_the_observed_set() {
        assert_eq!(s("1"), "1");
        assert_eq!(s("1.5"), "1.5");
        assert_eq!(s(".5"), "0.5");
        assert_eq!(s("1."), "1");
        assert_eq!(s("01"), "1");
        assert_eq!(s("true"), "true");
        assert_eq!(s("false"), "false");
        assert_eq!(s("null"), "null");
        assert_eq!(s(r#""a b""#), "\"a b\"");
        assert_eq!(s("'a b'"), "\"a b\"");
        // JSON escapes, and a raw multi-byte character.
        assert_eq!(
            parse(r#""a\nb""#).unwrap(),
            Expr::Primitive(Literal::String("a\nb".into()))
        );
        assert_eq!(
            parse(r#""é""#).unwrap(),
            Expr::Primitive(Literal::String("é".into()))
        );
        assert_eq!(
            parse(r#""é""#).unwrap(),
            Expr::Primitive(Literal::String("é".into()))
        );
        // `undefined`, `NaN` and `Infinity` are idents, not literals.
        for name in ["undefined", "NaN", "Infinity"] {
            assert_eq!(parse(name).unwrap(), ident(name), "{name}");
        }
    }

    /// Everything the probe tried and the parser rejected. Each of these
    /// parsing would be a divergence, not a kindness.
    #[test]
    fn what_is_not_in_the_language_does_not_parse() {
        let rejected = [
            // no ternary, no nullish coalescing, no word operators
            "a ? b : c",
            "a ?? b",
            "a in b",
            "a and b",
            "a or b",
            "not a",
            // no bitwise, no exponent, no strict equality, no assignment
            "a & b",
            "a | b",
            "a ** b",
            "a ^ b",
            "a === b",
            "a !== b",
            "a = b",
            // no object literal, no lambda
            "{}",
            "{a: 1}",
            "x => x",
            "list.filter(x => x)",
            // numbers the lexer does not take
            "+1",
            "1e3",
            "0x10",
            "1_000",
            // malformed
            "",
            "foo((((",
            r#""unterminated"#,
            "a.",
            "(1",
            "[1",
        ];
        for src in rejected {
            assert!(
                parse(src).is_err(),
                "{src:?} parsed, but Obsidian refuses it"
            );
        }
    }

    /// An unknown escape is an error, because Obsidian hands the quoted token
    /// to `JSON.parse` — the message says so.
    #[test]
    fn an_unknown_escape_is_a_json_error() {
        let err = parse(r#""a\qb""#).unwrap_err();
        assert!(
            err.message
                .starts_with("Bad escaped character in JSON at position"),
            "{}",
            err.message
        );
    }

    /// A failed parse is a node, not an exception: that is how a broken
    /// statement survives a save with its text intact.
    #[test]
    fn a_failed_parse_is_a_node_carrying_the_original_text() {
        let node = parse_node("foo((((");
        let Expr::Invalid { value, error } = node else {
            panic!("expected an invalid node");
        };
        assert_eq!(value, "foo((((");
        assert!(error.starts_with("Unable to parse formula."), "{error}");
        // ... and a statement that parses is never wrapped.
        assert_eq!(parse_node("1 + 1"), parse("1 + 1").unwrap());
    }

    /// The names are kept as written. Resolution is case-insensitive, but that
    /// is the evaluator's rule and the tree is not the place to apply it.
    #[test]
    fn names_keep_the_case_they_were_written_in() {
        assert_eq!(s("file.NAME"), "file.NAME");
        assert_eq!(s("DENSITY"), "DENSITY");
        assert_eq!(s("NOW()"), "NOW()");
        // `true` is a keyword only in lower case; `TRUE` is an ident.
        assert_eq!(parse("TRUE").unwrap(), ident("TRUE"));
    }

    /// The call shapes that make a lambda unnecessary: an ordinary expression
    /// per element, and `reduce` taking a second argument.
    #[test]
    fn the_list_functions_take_expressions_not_lambdas() {
        assert_eq!(
            s("listish.filter(value != \"beta\")"),
            "listish.filter((value != \"beta\"))"
        );
        assert_eq!(s("nums.map(value * 2)"), "nums.map((value * 2))");
        assert_eq!(
            s("nums.reduce(acc + value, 0)"),
            "nums.reduce((acc + value), 0)"
        );
    }

    /// Every statement in the probe's own fixture, which is the set of shapes
    /// a real `.base` was seen to hold.
    #[test]
    fn the_fixtures_statements_all_parse() {
        for src in [
            r#"status != "done""#,
            r#"file.hasTag("rock")"#,
            "density > 1",
            "silica < 50",
            "(density / silica).toFixed(2)",
            r#"type + " "  + file.name"#,
            r##""#" + status"##,
            r#"if(status == "a", "x: y", "z")"#,
            r#"[1, 2, 3].join(", ")"#,
            r#"status + "x" "#,
            "values.mean().round(3)",
            "values.sum()",
            r#"if(status == "todo", "a rather long string", "no")"#,
        ] {
            assert!(parse(src).is_ok(), "{src:?} did not parse");
        }
    }
}
