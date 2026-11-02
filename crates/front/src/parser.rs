//! Tokens to the program as written. Arithmetic follows school maths, with `()` to group (D17); where the order of
//! two words is still open, the parser asks for parentheses instead of picking one (rule 2).

use crate::ast::{BinaryOp, Branch, Expr, ExprKind, Program, Stmt};
use crate::diagnostic::{Diagnostic, Span};
use crate::lexer::{Keyword, Token, TokenKind};

/// Parses a whole program.
///
/// # Errors
///
/// The first place the tokens don't form a program.
pub fn parse(tokens: &[Token]) -> Result<Program, Diagnostic> {
    if tokens.is_empty() {
        return Err(Diagnostic::bug("the lexer gave no tokens", Span::new(0, 0)));
    }
    let mut parser = Parser {
        tokens,
        position: 0,
    };
    let mut statements = Vec::new();
    loop {
        parser.skip_newlines();
        if parser.peek().kind == TokenKind::End {
            break;
        }
        statements.push(parser.statement()?);
        parser.expect_line_end()?;
    }
    Ok(Program { statements })
}

/// How an expression was built, for the places where mixing two words without parentheses is still open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Shape {
    Plain,
    Not,
    Arithmetic { remainder: bool },
    Comparison,
    Logic,
}

struct Parser<'tokens> {
    tokens: &'tokens [Token],
    position: usize,
}

impl Parser<'_> {
    fn peek(&self) -> &Token {
        self.peek_at(0)
    }

    fn peek_at(&self, offset: usize) -> &Token {
        let last = self.tokens.len() - 1;
        &self.tokens[(self.position + offset).min(last)]
    }

    fn peek_keyword(&self) -> Option<Keyword> {
        match self.peek().kind {
            TokenKind::Keyword(keyword) => Some(keyword),
            _ => None,
        }
    }

    fn advance(&mut self) {
        if self.peek().kind != TokenKind::End {
            self.position += 1;
        }
    }

    fn skip_newlines(&mut self) {
        while self.peek().kind == TokenKind::Newline {
            self.advance();
        }
    }

    fn statement(&mut self) -> Result<Stmt, Diagnostic> {
        let token = self.peek().clone();
        match &token.kind {
            TokenKind::Keyword(Keyword::Show) => {
                self.advance();
                let value = self.expression()?;
                let condition = if self.peek_keyword() == Some(Keyword::If) {
                    self.advance();
                    Some(self.expression()?)
                } else {
                    None
                };
                Ok(Stmt::Show { value, condition })
            }
            TokenKind::Keyword(Keyword::If) => {
                self.advance();
                self.if_block(token.span)
            }
            TokenKind::Keyword(keyword @ (Keyword::ElseIf | Keyword::Else | Keyword::EndOf)) => {
                Err(Diagnostic::error(
                    format!("this `{}` has no `if` to belong to", keyword.as_str()),
                    token.span,
                ))
            }
            TokenKind::Name(name) => self.name_statement(name, token.span),
            _ => self.expression_statement(),
        }
    }

    fn name_statement(&mut self, name: &str, name_span: Span) -> Result<Stmt, Diagnostic> {
        let next = self.peek_at(1).clone();
        match &next.kind {
            // D12
            TokenKind::Equals => {
                self.advance();
                self.advance();
                let value = self.expression()?;
                Ok(Stmt::Create {
                    name: name.to_owned(),
                    name_span,
                    value,
                })
            }
            // D14
            TokenKind::Keyword(Keyword::IsNow) => {
                self.advance();
                self.advance();
                let value = self.expression()?;
                Ok(Stmt::Change {
                    name: name.to_owned(),
                    name_span,
                    value,
                })
            }
            // D25
            TokenKind::Name(of) if name == "end" && of == "of" => Err(Diagnostic::error(
                "`end of` is one word",
                name_span.to(next.span),
            )
            .with_hint("write `end_of if` (D25)")),
            _ => self.expression_statement(),
        }
    }

    fn expression_statement(&mut self) -> Result<Stmt, Diagnostic> {
        let expression = self.expression()?;
        let diagnostic = Diagnostic::error(
            "this line works something out but does nothing with it",
            expression.span,
        );
        if let ExprKind::Binary {
            op: BinaryOp::Is,
            left,
            ..
        } = &expression.kind
            && let ExprKind::Name(name) = &left.kind
        {
            return Err(
                diagnostic.with_hint(format!("to change `{name}`, write `{name} is_now …` (D14)"))
            );
        }
        Err(diagnostic.with_hint("to show it, put `show` in front"))
    }

    fn expect_line_end(&mut self) -> Result<(), Diagnostic> {
        let token = self.peek().clone();
        match &token.kind {
            TokenKind::Newline => {
                self.advance();
                Ok(())
            }
            TokenKind::End => Ok(()),
            TokenKind::Keyword(Keyword::If) => Err(Diagnostic::not_decided(
                "an `if` at the end of anything but `show`",
                "D20",
                token.span,
            )),
            TokenKind::Name(word) => Err(self.stray_word(word, token.span)),
            _ => Err(Diagnostic::error(
                format!(
                    "expected the end of the line, but found {}",
                    describe(&token.kind)
                ),
                token.span,
            )),
        }
    }

    fn stray_word(&self, word: &str, span: Span) -> Diagnostic {
        let diagnostic = Diagnostic::error(
            format!("expected the end of the line, but found `{word}`"),
            span,
        );
        let next = match &self.peek_at(1).kind {
            TokenKind::Name(next) => next.as_str(),
            _ => "",
        };
        match (word, next) {
            // D20
            ("equal", _) => diagnostic.with_hint("to compare, write `is` (D20)"),
            ("not_equal", _) => diagnostic.with_hint("write `is_not` (D20)"),
            // D5
            ("at", "least" | "most") => {
                diagnostic.with_hint(format!("write `at_{next}`: one thing, no space (D5)"))
            }
            _ => diagnostic,
        }
    }

    fn if_block(&mut self, if_span: Span) -> Result<Stmt, Diagnostic> {
        let condition = self.expression()?;
        self.block_start("if")?;
        let body = self.block_body()?;
        let mut branches = vec![Branch { condition, body }];
        let mut otherwise = None;
        loop {
            let token = self.peek().clone();
            match token.kind {
                // D24
                TokenKind::Keyword(Keyword::ElseIf) => {
                    if otherwise.is_some() {
                        return Err(Diagnostic::error(
                            "an `else_if` can't come after `else`",
                            token.span,
                        ));
                    }
                    self.advance();
                    let condition = self.expression()?;
                    self.block_start("else_if")?;
                    let body = self.block_body()?;
                    branches.push(Branch { condition, body });
                }
                TokenKind::Keyword(Keyword::Else) => {
                    if otherwise.is_some() {
                        return Err(Diagnostic::error(
                            "this `if` already has an `else`",
                            token.span,
                        ));
                    }
                    self.advance();
                    if self.peek_keyword() == Some(Keyword::If) {
                        return Err(Diagnostic::error(
                            "`else if` is one word",
                            token.span.to(self.peek().span),
                        )
                        .with_hint("write `else_if` (D24)"));
                    }
                    self.block_start("else")?;
                    otherwise = Some(self.block_body()?);
                }
                // D25
                TokenKind::Keyword(Keyword::EndOf) => {
                    self.advance();
                    return self.end_of(
                        token.span,
                        Stmt::If {
                            branches,
                            otherwise,
                        },
                    );
                }
                TokenKind::End => {
                    return Err(Diagnostic::error("this `if` has no `end_of if`", if_span)
                        .with_hint("add `end_of if` after its last line (D25)"));
                }
                _ => return Err(Diagnostic::bug("an `if` block stopped early", token.span)),
            }
        }
    }

    fn end_of(&mut self, end_of_span: Span, block: Stmt) -> Result<Stmt, Diagnostic> {
        let token = self.peek().clone();
        match &token.kind {
            TokenKind::Keyword(Keyword::If) => {
                self.advance();
                Ok(block)
            }
            TokenKind::Name(name) => Err(Diagnostic::error(
                format!("`end_of {name}` can't close this block: it's an `if`"),
                end_of_span.to(token.span),
            )
            .with_hint("write `end_of if` (D25)")),
            _ => Err(Diagnostic::error(
                "`end_of` needs the name of the block it closes",
                end_of_span,
            )
            .with_hint("write `end_of if` (D25)")),
        }
    }

    // D22: a colon after the condition, the block on the lines after it.
    fn block_start(&mut self, opener: &str) -> Result<(), Diagnostic> {
        let token = self.peek().clone();
        if token.kind != TokenKind::Colon {
            return Err(Diagnostic::error(
                format!(
                    "this `{opener}` needs a `:` here, but found {}",
                    describe(&token.kind)
                ),
                token.span,
            )
            .with_hint("end the line with `:` (D22)"));
        }
        self.advance();
        let token = self.peek().clone();
        match token.kind {
            TokenKind::Newline => {
                self.advance();
                Ok(())
            }
            TokenKind::End => Ok(()),
            _ => Err(Diagnostic::not_decided(
                "a block on the same line as its `:`",
                "D22",
                token.span,
            )
            .with_hint("start the block on the next line")),
        }
    }

    fn block_body(&mut self) -> Result<Vec<Stmt>, Diagnostic> {
        let mut statements = Vec::new();
        loop {
            self.skip_newlines();
            match self.peek().kind {
                TokenKind::Keyword(Keyword::ElseIf | Keyword::Else | Keyword::EndOf)
                | TokenKind::End => {
                    return Ok(statements);
                }
                _ => {
                    statements.push(self.statement()?);
                    self.expect_line_end()?;
                }
            }
        }
    }

    fn expression(&mut self) -> Result<Expr, Diagnostic> {
        Ok(self.logic()?.0)
    }

    // D21
    fn logic(&mut self) -> Result<(Expr, Shape), Diagnostic> {
        let (mut left, mut shape) = self.comparison()?;
        let mut first = None;
        while let Some(keyword @ (Keyword::And | Keyword::Or)) = self.peek_keyword() {
            let op_span = self.peek().span;
            if first.is_some_and(|first| first != keyword) {
                return Err(Diagnostic::not_decided(
                    "mixing `and` and `or` without parentheses",
                    "D21",
                    op_span,
                )
                .with_hint("add parentheses, like `(a and b) or c`"));
            }
            first = Some(keyword);
            self.advance();
            let (right, _) = self.comparison()?;
            let op = if keyword == Keyword::And {
                BinaryOp::And
            } else {
                BinaryOp::Or
            };
            left = binary(op, op_span, left, right);
            shape = Shape::Logic;
        }
        Ok((left, shape))
    }

    // D17, D18, D20
    fn comparison(&mut self) -> Result<(Expr, Shape), Diagnostic> {
        let (left, left_shape) = self.additive()?;
        let Some(op) = self.peek_keyword().and_then(comparison_op) else {
            return Ok((left, left_shape));
        };
        let op_span = self.peek().span;
        self.advance();
        if op == BinaryOp::Is && self.peek_keyword() == Some(Keyword::Not) {
            return Err(
                Diagnostic::error("`is not` is one word", op_span.to(self.peek().span))
                    .with_hint("write `is_not` (D20)"),
            );
        }
        let (right, right_shape) = self.additive()?;
        for (side, shape) in [(&left, left_shape), (&right, right_shape)] {
            if matches!(shape, Shape::Arithmetic { .. }) {
                return Err(Diagnostic::not_decided(
                    "arithmetic beside a comparison without parentheses",
                    "D17",
                    side.span,
                )
                .with_hint(
                    "add parentheses around the arithmetic, like `(age plus 1) at_least 18`",
                ));
            }
        }
        if self.peek_keyword().and_then(comparison_op).is_some() {
            return Err(
                Diagnostic::not_decided("chaining comparisons", "D18", self.peek().span)
                    .with_hint("join them with `and`, like `a less_than b and b less_than c`"),
            );
        }
        Ok((binary(op, op_span, left, right), Shape::Comparison))
    }

    // D16, D17: `plus` and `minus` bind looser than `times` and `divided_by`.
    fn additive(&mut self) -> Result<(Expr, Shape), Diagnostic> {
        let (mut left, mut shape) = self.multiplicative()?;
        while let Some(keyword @ (Keyword::Plus | Keyword::Minus)) = self.peek_keyword() {
            let op_span = self.peek().span;
            self.advance();
            let (right, right_shape) = self.multiplicative()?;
            if has_remainder(shape) || has_remainder(right_shape) {
                return Err(remainder_beside_arithmetic(op_span));
            }
            let op = if keyword == Keyword::Plus {
                BinaryOp::Plus
            } else {
                BinaryOp::Minus
            };
            left = binary(op, op_span, left, right);
            shape = Shape::Arithmetic { remainder: false };
        }
        Ok((left, shape))
    }

    fn multiplicative(&mut self) -> Result<(Expr, Shape), Diagnostic> {
        let (mut left, mut shape) = self.unary()?;
        let (mut remainder, mut other) = (false, false);
        while let Some(keyword @ (Keyword::Times | Keyword::DividedBy | Keyword::Remainder)) =
            self.peek_keyword()
        {
            let op_span = self.peek().span;
            self.advance();
            let (right, _) = self.unary()?;
            let op = match keyword {
                Keyword::Times => BinaryOp::Times,
                Keyword::DividedBy => BinaryOp::DividedBy,
                _ => BinaryOp::Remainder,
            };
            if op == BinaryOp::Remainder {
                remainder = true;
            } else {
                other = true;
            }
            if remainder && other {
                return Err(remainder_beside_arithmetic(op_span));
            }
            left = binary(op, op_span, left, right);
            shape = Shape::Arithmetic { remainder };
        }
        Ok((left, shape))
    }

    fn unary(&mut self) -> Result<(Expr, Shape), Diagnostic> {
        if self.peek_keyword() != Some(Keyword::Not) {
            return Ok((self.primary()?, Shape::Plain));
        }
        let not_span = self.peek().span;
        self.advance();
        let (operand, _) = self.unary()?;
        let expression = Expr {
            span: not_span.to(operand.span),
            kind: ExprKind::Not(Box::new(operand)),
        };
        let next_is_arithmetic_or_comparison = self.peek_keyword().is_some_and(|keyword| {
            arithmetic_op(keyword).is_some() || comparison_op(keyword).is_some()
        });
        if next_is_arithmetic_or_comparison {
            return Err(Diagnostic::not_decided(
                "`not` before arithmetic or a comparison without parentheses",
                "D21",
                expression.span,
            )
            .with_hint("add parentheses, like `not (age is 5)`"));
        }
        Ok((expression, Shape::Not))
    }

    fn primary(&mut self) -> Result<Expr, Diagnostic> {
        let token = self.peek().clone();
        let kind = match token.kind {
            TokenKind::Int(value) => ExprKind::Int(value),
            TokenKind::Rational(value) => ExprKind::Rational(value),
            TokenKind::Text(text) => ExprKind::Text(text),
            TokenKind::Keyword(Keyword::True) => ExprKind::Boolean(true),
            TokenKind::Keyword(Keyword::False) => ExprKind::Boolean(false),
            TokenKind::Name(name) => ExprKind::Name(name),
            TokenKind::OpenParen => return self.parenthesized(token.span),
            // D26
            TokenKind::Keyword(Keyword::Ask) => {
                self.advance();
                let prompt = self.primary()?;
                return Ok(Expr {
                    span: token.span.to(prompt.span),
                    kind: ExprKind::Ask(Box::new(prompt)),
                });
            }
            TokenKind::Keyword(Keyword::Minus) => {
                return Err(
                    Diagnostic::not_decided("negative numbers", "D16", token.span)
                        .with_hint("write `0 minus 5`"),
                );
            }
            other => {
                return Err(Diagnostic::error(
                    format!("expected a value here, but found {}", describe(&other)),
                    token.span,
                ));
            }
        };
        self.advance();
        Ok(Expr {
            kind,
            span: token.span,
        })
    }

    // D17
    fn parenthesized(&mut self, open_span: Span) -> Result<Expr, Diagnostic> {
        self.advance();
        let inner = self.expression()?;
        let close = self.peek().clone();
        if close.kind != TokenKind::CloseParen {
            return Err(Diagnostic::error("this `(` is never closed", open_span)
                .with_hint(format!("add `)`; found {} instead", describe(&close.kind))));
        }
        self.advance();
        Ok(Expr {
            kind: inner.kind,
            span: open_span.to(close.span),
        })
    }
}

fn binary(op: BinaryOp, op_span: Span, left: Expr, right: Expr) -> Expr {
    Expr {
        span: left.span.to(right.span),
        kind: ExprKind::Binary {
            op,
            op_span,
            left: Box::new(left),
            right: Box::new(right),
        },
    }
}

fn has_remainder(shape: Shape) -> bool {
    shape == Shape::Arithmetic { remainder: true }
}

fn remainder_beside_arithmetic(span: Span) -> Diagnostic {
    Diagnostic::not_decided(
        "`remainder` beside other arithmetic without parentheses",
        "D17",
        span,
    )
    .with_hint("add parentheses, like `(7 remainder 2) plus 1`")
}

fn arithmetic_op(keyword: Keyword) -> Option<BinaryOp> {
    match keyword {
        Keyword::Plus => Some(BinaryOp::Plus),
        Keyword::Minus => Some(BinaryOp::Minus),
        Keyword::Times => Some(BinaryOp::Times),
        Keyword::DividedBy => Some(BinaryOp::DividedBy),
        Keyword::Remainder => Some(BinaryOp::Remainder),
        _ => None,
    }
}

fn comparison_op(keyword: Keyword) -> Option<BinaryOp> {
    match keyword {
        Keyword::Is => Some(BinaryOp::Is),
        Keyword::IsNot => Some(BinaryOp::IsNot),
        Keyword::AtLeast => Some(BinaryOp::AtLeast),
        Keyword::AtMost => Some(BinaryOp::AtMost),
        Keyword::GreaterThan => Some(BinaryOp::GreaterThan),
        Keyword::LessThan => Some(BinaryOp::LessThan),
        _ => None,
    }
}

fn describe(kind: &TokenKind) -> String {
    match kind {
        TokenKind::Name(name) => format!("`{name}`"),
        TokenKind::Keyword(keyword) => format!("`{}`", keyword.as_str()),
        TokenKind::Int(_) | TokenKind::Rational(_) => "a number".to_owned(),
        TokenKind::Text(_) => "text".to_owned(),
        TokenKind::Equals => "`=`".to_owned(),
        TokenKind::Colon => "`:`".to_owned(),
        TokenKind::OpenParen => "`(`".to_owned(),
        TokenKind::CloseParen => "`)`".to_owned(),
        TokenKind::Newline => "the end of the line".to_owned(),
        TokenKind::End => "the end of the program".to_owned(),
    }
}
