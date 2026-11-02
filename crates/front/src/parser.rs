//! Tokens to the program as written. Arithmetic follows school maths, with `()` to group (D17); where the order of
//! two words is still open, the parser asks for parentheses instead of picking one (rule 2).

use crate::ast::{BinaryOp, Branch, Counting, Expr, ExprKind, Program, Selector, Stmt, Stop};
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

/// The rule behind every keyword spelt with `_` (D5).
const JOINED: &str = "a keyword made of several words is joined with `_`";

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
                self.show()
            }
            TokenKind::Keyword(Keyword::If) => {
                self.advance();
                self.if_block(token.span)
            }
            TokenKind::Keyword(keyword @ (Keyword::ElseIf | Keyword::Else | Keyword::EndOf)) => {
                Err(Diagnostic::mistake(
                    format!("this `{}` has no `if` to belong to", keyword.as_str()),
                    token.span,
                ))
            }
            TokenKind::Name(name) => self.name_statement(name, token.span),
            _ => self.expression_statement(),
        }
    }

    // D28: showing a comparison takes `whether`.
    fn show(&mut self) -> Result<Stmt, Diagnostic> {
        if self.next_word_is("items")
            && matches!(&self.peek_at(1).kind, TokenKind::Name(word) if word == "in")
        {
            return self.show_items();
        }
        let whether = self.peek_keyword() == Some(Keyword::Whether);
        if whether {
            self.advance();
        }
        let (value, shape) = self.logic()?;
        if !whether && matches!(shape, Shape::Comparison | Shape::Logic | Shape::Not) {
            return Err(
                Diagnostic::mistake("showing a comparison takes `whether`", value.span)
                    .fix("show", "show whether")
                    .because("a comparison after `show` would read as a claim, not a question"),
            );
        }
        // D40
        let count = if let TokenKind::Count(count) = self.peek().kind {
            self.advance();
            count
        } else {
            1
        };
        let condition = if self.peek_keyword() == Some(Keyword::If) {
            self.advance();
            Some(self.expression()?)
        } else {
            None
        };
        Ok(Stmt::Show {
            value,
            whether,
            count,
            condition,
        })
    }

    // D38: `show items in scores until …` and `until_before …`.
    fn show_items(&mut self) -> Result<Stmt, Diagnostic> {
        self.advance();
        self.advance();
        let table = self.primary()?;
        let (inclusive, stop) = self.until("`show items in`")?;
        let condition = if self.peek_keyword() == Some(Keyword::If) {
            self.advance();
            Some(self.expression()?)
        } else {
            None
        };
        Ok(Stmt::ShowItems {
            table,
            stop,
            inclusive,
            condition,
        })
    }

    // D38, D39: `until` or `until_before`, then the stop.
    fn until(&mut self, form: &str) -> Result<(bool, Stop), Diagnostic> {
        let inclusive = if self.next_word_is("until") {
            true
        } else if self.next_word_is("until_before") {
            false
        } else {
            let token = self.peek().clone();
            return Err(Diagnostic::mistake(
                format!(
                    "expected `until` or `until_before`, but found {}",
                    describe(&token.kind)
                ),
                token.span,
            )
            .because(format!("{form} says where to stop")));
        };
        self.advance();
        let stop_token = self.peek().clone();
        let stop = if self.next_word_is("last_item") {
            self.advance();
            Stop::LastItem
        } else if self.next_word_is("position") {
            self.advance();
            Stop::Position(self.additive()?.0)
        } else if self.next_word_is("value") {
            self.advance();
            Stop::Value(self.additive()?.0)
        } else if self.next_word_is("current_item") {
            Stop::Test(self.expression()?)
        } else if self.next_word_is("offset") {
            return Err(Diagnostic::not_decided(
                "stopping at an offset",
                Some("D38"),
                stop_token.span,
            ));
        } else {
            return Err(Diagnostic::mistake(
                format!(
                    "expected where to stop, but found {}",
                    describe(&stop_token.kind)
                ),
                stop_token.span,
            )
            .because("a stop is `last_item`, `position`, `value`, or a test on `current_item`"));
        };
        Ok((inclusive, stop))
    }

    // D39: `repeat 3 times:` and `repeat until …:`, closed by `end_of repeat`.
    fn repeat(&mut self, repeat_span: Span) -> Result<Stmt, Diagnostic> {
        self.advance();
        if self.next_word_is("until") {
            self.advance();
            let test = self.expression()?;
            let body = self.loop_block("repeat", repeat_span)?;
            return Ok(Stmt::RepeatUntil { test, body });
        }
        if self.next_word_is("forever") {
            return Err(Diagnostic::not_decided(
                "`repeat forever`",
                None,
                self.peek().span,
            ));
        }
        let (count, _) = self.additive()?;
        if self.peek_keyword() != Some(Keyword::Times) {
            let token = self.peek().clone();
            return Err(Diagnostic::mistake(
                format!(
                    "expected `times` after the count, but found {}",
                    describe(&token.kind)
                ),
                token.span,
            )
            .because("a counted loop is `repeat <count> times:`"));
        }
        self.advance();
        let body = self.loop_block("repeat", repeat_span)?;
        Ok(Stmt::RepeatTimes { count, body })
    }

    // D39: `for items in scores until …:`, closed by `end_of for`.
    fn for_items(&mut self, for_span: Span) -> Result<Stmt, Diagnostic> {
        self.advance();
        self.advance();
        self.advance();
        let table = self.primary()?;
        let (inclusive, stop) = self.until("`for items in`")?;
        let body = self.loop_block("for", for_span)?;
        Ok(Stmt::ForItems {
            table,
            stop,
            inclusive,
            body,
        })
    }

    fn loop_block(&mut self, word: &str, opener_span: Span) -> Result<Vec<Stmt>, Diagnostic> {
        self.block_start(word)?;
        let body = self.block_body()?;
        let token = self.peek().clone();
        match token.kind {
            TokenKind::Keyword(Keyword::EndOf) => {
                self.advance();
                self.end_of(token.span, word)?;
                Ok(body)
            }
            TokenKind::Keyword(keyword @ (Keyword::ElseIf | Keyword::Else)) => {
                Err(Diagnostic::mistake(
                    format!("this `{}` has no `if` to belong to", keyword.as_str()),
                    token.span,
                ))
            }
            _ => Err(Diagnostic::mistake(
                format!("this `{word}` has no `end_of {word}`"),
                opener_span,
            )
            .because(format!("every `{word}` block closes with `end_of {word}`"))),
        }
    }

    fn name_statement(&mut self, name: &str, name_span: Span) -> Result<Stmt, Diagnostic> {
        let next = self.peek_at(1).clone();
        let assigns = matches!(
            next.kind,
            TokenKind::Equals | TokenKind::Keyword(Keyword::IsNow)
        );
        if name == "repeat" && !assigns {
            return self.repeat(name_span);
        }
        let word_at = |offset: usize, word: &str| matches!(&self.peek_at(offset).kind, TokenKind::Name(found) if found == word);
        if name == "for" && word_at(1, "items") && word_at(2, "in") {
            return self.for_items(name_span);
        }
        match &next.kind {
            // D12
            TokenKind::Equals => {
                self.advance();
                self.advance();
                let value = self.assigned_value()?;
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
                let value = self.assigned_value()?;
                Ok(Stmt::Change {
                    name: name.to_owned(),
                    name_span,
                    value,
                })
            }
            // D25
            TokenKind::Name(of) if name == "end" && of == "of" => Err(Diagnostic::mistake(
                "`end of` is one word",
                name_span.to(next.span),
            )
            .fix("end of", "end_of")
            .because(JOINED)),
            _ => self.expression_statement(),
        }
    }

    fn expression_statement(&mut self) -> Result<Stmt, Diagnostic> {
        let expression = self.expression()?;
        if self.peek().kind == TokenKind::Keyword(Keyword::IsNow)
            && matches!(
                expression.kind,
                ExprKind::ReadPosition { .. } | ExprKind::ReadKey { .. }
            )
        {
            return self.change_item(expression);
        }
        let diagnostic = Diagnostic::mistake(
            "this line works something out but does nothing with it",
            expression.span,
        );
        Err(diagnostic
            .because("a line shows something, makes or changes a variable, or starts a block"))
    }

    // D35: `value at position 1 in scores is_now 95`.
    fn change_item(&mut self, target: Expr) -> Result<Stmt, Diagnostic> {
        let (table, selector) = match target.kind {
            ExprKind::ReadPosition {
                table,
                index,
                counting,
            } => (table, Selector::Index(*index, counting)),
            ExprKind::ReadKey { table, key } => (table, Selector::Key(*key)),
            _ => {
                return Err(Diagnostic::bug(
                    "a change of something that isn't an item",
                    target.span,
                ));
            }
        };
        let ExprKind::Name(name) = table.kind else {
            return Err(Diagnostic::not_decided(
                "changing an item in a table inside a table",
                Some("D35"),
                table.span,
            ));
        };
        self.advance();
        let value = self.assigned_value()?;
        Ok(Stmt::ChangeItem {
            name,
            name_span: table.span,
            selector,
            value,
        })
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
                Some("D20"),
                token.span,
            )),
            TokenKind::Name(word) => Err(self.stray_word(word, token.span)),
            TokenKind::Count(_) => Err(Diagnostic::not_decided(
                "a count like `50x` anywhere but right after `show`'s value",
                Some("D40"),
                token.span,
            )),
            TokenKind::Comma => Err(Diagnostic::not_decided(
                "a comma anywhere but in a table right after `=` or `is_now`",
                Some("D33"),
                token.span,
            )),
            _ => Err(Diagnostic::mistake(
                format!(
                    "expected the end of the line, but found {}",
                    describe(&token.kind)
                ),
                token.span,
            )),
        }
    }

    fn stray_word(&self, word: &str, span: Span) -> Diagnostic {
        let diagnostic = Diagnostic::mistake(
            format!("expected the end of the line, but found `{word}`"),
            span,
        );
        let next = match &self.peek_at(1).kind {
            TokenKind::Name(next) => next.as_str(),
            _ => "",
        };
        match (word, next) {
            // D20
            ("equal", _) => diagnostic
                .fix("equal", "is")
                .because("`is` compares two things"),
            ("not_equal", _) => diagnostic.fix("not_equal", "is_not"),
            // D5
            ("at", "least" | "most") => Diagnostic::mistake(
                format!("`at {next}` is one word"),
                span.to(self.peek_at(1).span),
            )
            .fix(&format!("at {next}"), &format!("at_{next}"))
            .because(JOINED),
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
                        return Err(Diagnostic::mistake(
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
                        return Err(Diagnostic::mistake(
                            "this `if` already has an `else`",
                            token.span,
                        ));
                    }
                    self.advance();
                    if self.peek_keyword() == Some(Keyword::If) {
                        return Err(Diagnostic::mistake(
                            "`else if` is one word",
                            token.span.to(self.peek().span),
                        )
                        .fix("else if", "else_if")
                        .because(JOINED));
                    }
                    self.block_start("else")?;
                    otherwise = Some(self.block_body()?);
                }
                // D25
                TokenKind::Keyword(Keyword::EndOf) => {
                    self.advance();
                    self.end_of(token.span, "if")?;
                    return Ok(Stmt::If {
                        branches,
                        otherwise,
                    });
                }
                TokenKind::End => {
                    return Err(Diagnostic::mistake("this `if` has no `end_of if`", if_span)
                        .because("every `if` block closes with `end_of if`"));
                }
                _ => return Err(Diagnostic::bug("an `if` block stopped early", token.span)),
            }
        }
    }

    // D25: `end_of` and the name of the block it closes.
    fn end_of(&mut self, end_of_span: Span, block: &str) -> Result<(), Diagnostic> {
        let token = self.peek().clone();
        let expected = format!("end_of {block}");
        let closing = match &token.kind {
            TokenKind::Keyword(keyword) => keyword.as_str().to_owned(),
            TokenKind::Name(name) => name.clone(),
            _ => {
                return Err(Diagnostic::mistake(
                    "`end_of` needs the name of the block it closes",
                    end_of_span,
                )
                .fix("end_of", &expected));
            }
        };
        if closing == block {
            self.advance();
            return Ok(());
        }
        let described = if block == "if" {
            "an `if`".to_owned()
        } else {
            format!("a `{block}`")
        };
        Err(Diagnostic::mistake(
            format!("`end_of {closing}` can't close this block: it's {described}"),
            end_of_span.to(token.span),
        )
        .fix(&format!("end_of {closing}"), &expected)
        .because(format!("the block being closed is {described}")))
    }

    // D33: the value after `=` or `is_now` may be a table.
    fn assigned_value(&mut self) -> Result<Expr, Diagnostic> {
        if self.peek_keyword() == Some(Keyword::Table) {
            return self.keyed_table();
        }
        let first = self.expression()?;
        if self.peek().kind != TokenKind::Comma {
            return Ok(first);
        }
        let start = first.span;
        let mut items = vec![first];
        while self.peek().kind == TokenKind::Comma {
            let comma_span = self.peek().span;
            self.advance();
            if matches!(self.peek().kind, TokenKind::Newline | TokenKind::End) {
                return Err(Diagnostic::not_decided(
                    "a comma at the end of a table, as for a table with one item",
                    Some("D33"),
                    comma_span,
                ));
            }
            items.push(self.expression()?);
        }
        let end = items.last().map_or(start, |item| item.span);
        Ok(Expr {
            kind: ExprKind::InOrder(items),
            span: start.to(end),
        })
    }

    // D33: `table:`, one `key: value` per line, `end_of table`.
    fn keyed_table(&mut self) -> Result<Expr, Diagnostic> {
        let table_span = self.peek().span;
        self.advance();
        self.block_start("table")?;
        let mut entries = Vec::new();
        loop {
            self.skip_newlines();
            let token = self.peek().clone();
            match token.kind {
                TokenKind::Keyword(Keyword::EndOf) => {
                    self.advance();
                    let closing_span = self.peek().span;
                    self.end_of(token.span, "table")?;
                    if entries.is_empty() {
                        return Err(Diagnostic::not_decided(
                            "an empty table",
                            Some("D33"),
                            table_span,
                        ));
                    }
                    return Ok(Expr {
                        kind: ExprKind::Keyed(entries),
                        span: table_span.to(closing_span),
                    });
                }
                TokenKind::End => {
                    return Err(Diagnostic::mistake(
                        "this `table` has no `end_of table`",
                        table_span,
                    )
                    .because("every `table` block closes with `end_of table`"));
                }
                _ => {
                    let key = self.expression()?;
                    let colon = self.peek().clone();
                    if colon.kind != TokenKind::Colon {
                        return Err(Diagnostic::mistake(
                            format!(
                                "expected `:` after the key, but found {}",
                                describe(&colon.kind)
                            ),
                            colon.span,
                        )
                        .because("each line of a `table:` block is `key: value`"));
                    }
                    self.advance();
                    let value = self.expression()?;
                    entries.push((key, value));
                    if self.peek().kind != TokenKind::End {
                        self.expect_line_end()?;
                    }
                }
            }
        }
    }

    // D22: a colon after the condition, the block on the lines after it.
    fn block_start(&mut self, opener: &str) -> Result<(), Diagnostic> {
        let token = self.peek().clone();
        if token.kind != TokenKind::Colon {
            return Err(Diagnostic::mistake(
                format!(
                    "this `{opener}` needs a `:` here, but found {}",
                    describe(&token.kind)
                ),
                token.span,
            )
            .because("a block's first line ends with `:`"));
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
                Some("D22"),
                token.span,
            )),
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
                    Some("D21"),
                    op_span,
                ));
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
                Diagnostic::mistake("`is not` is one word", op_span.to(self.peek().span))
                    .fix("is not", "is_not")
                    .because(JOINED),
            );
        }
        // D30: comparisons bind looser than all arithmetic, so the arithmetic on each side comes first.
        let (right, _) = self.additive()?;
        if self.peek_keyword().and_then(comparison_op).is_some() {
            return Err(Diagnostic::not_decided(
                "chaining comparisons",
                Some("D18"),
                self.peek().span,
            ));
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
            // D39: `repeat 3 times:` — `times` right before the colon ends the count.
            if keyword == Keyword::Times && self.peek_at(1).kind == TokenKind::Colon {
                break;
            }
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
                Some("D21"),
                expression.span,
            ));
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
            TokenKind::Name(name) if name == "value" && self.read_follows() => {
                return self.read(token.span);
            }
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
            TokenKind::Keyword(Keyword::Table) => {
                return Err(Diagnostic::not_decided(
                    "a table anywhere but right after `=` or `is_now`",
                    Some("D33"),
                    token.span,
                ));
            }
            TokenKind::Keyword(Keyword::Whether) => {
                return Err(Diagnostic::not_decided(
                    "`whether` anywhere but right after `show`",
                    Some("D28"),
                    token.span,
                ));
            }
            TokenKind::Keyword(Keyword::Minus) => {
                return Err(Diagnostic::not_decided(
                    "negative numbers",
                    Some("D16"),
                    token.span,
                ));
            }
            other => {
                return Err(Diagnostic::mistake(
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

    fn read_follows(&self) -> bool {
        let next_is = |offset: usize, word: &str| matches!(&self.peek_at(offset).kind, TokenKind::Name(name) if name == word);
        next_is(1, "of") || (next_is(1, "at") && (next_is(2, "position") || next_is(2, "offset")))
    }

    // D34: `value at position 1 in scores`, `value at offset 0 in scores`, `value of "Ada" in ages`.
    fn read(&mut self, value_span: Span) -> Result<Expr, Diagnostic> {
        self.advance();
        let counting = if self.next_word_is("of") {
            self.advance();
            None
        } else {
            self.advance();
            let counting = if self.next_word_is("position") {
                Counting::Position
            } else {
                Counting::Offset
            };
            self.advance();
            Some(counting)
        };
        let (index, _) = self.additive()?;
        let in_token = self.peek().clone();
        if !self.next_word_is("in") {
            return Err(Diagnostic::mistake(
                format!(
                    "expected `in` and the table, but found {}",
                    describe(&in_token.kind)
                ),
                in_token.span,
            )
            .because("a read names its table after `in`"));
        }
        self.advance();
        let table = self.primary()?;
        let span = value_span.to(table.span);
        let (table, index) = (Box::new(table), Box::new(index));
        let kind = match counting {
            Some(counting) => ExprKind::ReadPosition {
                table,
                index,
                counting,
            },
            None => ExprKind::ReadKey { table, key: index },
        };
        Ok(Expr { kind, span })
    }

    fn next_word_is(&self, word: &str) -> bool {
        matches!(&self.peek().kind, TokenKind::Name(name) if name == word)
    }

    // D17
    fn parenthesized(&mut self, open_span: Span) -> Result<Expr, Diagnostic> {
        self.advance();
        let inner = self.expression()?;
        let close = self.peek().clone();
        if close.kind != TokenKind::CloseParen {
            return Err(Diagnostic::mistake("this `(` is never closed", open_span)
                .because("every `(` needs a matching `)`"));
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
        Some("D17"),
        span,
    )
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
        TokenKind::Count(count) => format!("`{count}x`"),
        TokenKind::Text(_) => "text".to_owned(),
        TokenKind::Equals => "`=`".to_owned(),
        TokenKind::Colon => "`:`".to_owned(),
        TokenKind::Comma => "`,`".to_owned(),
        TokenKind::OpenParen => "`(`".to_owned(),
        TokenKind::CloseParen => "`)`".to_owned(),
        TokenKind::Newline => "the end of the line".to_owned(),
        TokenKind::End => "the end of the program".to_owned(),
    }
}
