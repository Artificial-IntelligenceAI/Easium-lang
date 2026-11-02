//! Splits source text into tokens. A space always separates two things, and one thing never holds a space, except
//! text inside quotes.

use redstone_runtime::{Rational, RationalError};

use crate::diagnostic::{Diagnostic, Span};

/// A word Redstone reserves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Keyword {
    /// `show`
    Show,
    /// `whether`
    Whether,
    /// `ask`
    Ask,
    /// `if`
    If,
    /// `else_if`
    ElseIf,
    /// `else`
    Else,
    /// `end_of`
    EndOf,
    /// `is_now`
    IsNow,
    /// `is`
    Is,
    /// `is_not`
    IsNot,
    /// `plus`
    Plus,
    /// `minus`
    Minus,
    /// `times`
    Times,
    /// `divided_by`
    DividedBy,
    /// `remainder`
    Remainder,
    /// `at_least`
    AtLeast,
    /// `at_most`
    AtMost,
    /// `greater_than`
    GreaterThan,
    /// `less_than`
    LessThan,
    /// `and`
    And,
    /// `or`
    Or,
    /// `not`
    Not,
    /// `true`
    True,
    /// `false`
    False,
}

// D5, D12, D14, D16, D18, D19, D20, D21, D22, D24, D25, D26, D28
const KEYWORDS: [(&str, Keyword); 24] = [
    ("show", Keyword::Show),
    ("whether", Keyword::Whether),
    ("ask", Keyword::Ask),
    ("if", Keyword::If),
    ("else_if", Keyword::ElseIf),
    ("else", Keyword::Else),
    ("end_of", Keyword::EndOf),
    ("is_now", Keyword::IsNow),
    ("is", Keyword::Is),
    ("is_not", Keyword::IsNot),
    ("plus", Keyword::Plus),
    ("minus", Keyword::Minus),
    ("times", Keyword::Times),
    ("divided_by", Keyword::DividedBy),
    ("remainder", Keyword::Remainder),
    ("at_least", Keyword::AtLeast),
    ("at_most", Keyword::AtMost),
    ("greater_than", Keyword::GreaterThan),
    ("less_than", Keyword::LessThan),
    ("and", Keyword::And),
    ("or", Keyword::Or),
    ("not", Keyword::Not),
    ("true", Keyword::True),
    ("false", Keyword::False),
];

impl Keyword {
    fn from_word(word: &str) -> Option<Self> {
        KEYWORDS
            .iter()
            .find(|(spelling, _)| *spelling == word)
            .map(|(_, keyword)| *keyword)
    }

    /// How the keyword is written.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        KEYWORDS
            .iter()
            .find(|(_, keyword)| *keyword == self)
            .map_or("", |(spelling, _)| spelling)
    }
}

/// What a token is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TokenKind {
    /// A name the program chose, such as `age`.
    Name(String),
    /// A reserved word.
    Keyword(Keyword),
    /// A whole number, such as `18`.
    Int(i64),
    /// A decimal, such as `0.1`, which is a rational.
    Rational(Rational),
    /// Text in quotes, without the quotes.
    Text(String),
    /// `=`
    Equals,
    /// `:`
    Colon,
    /// `(`
    OpenParen,
    /// `)`
    CloseParen,
    /// The end of a line.
    Newline,
    /// The end of the source.
    End,
}

/// One thing in the source, with where it is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Token {
    /// What it is.
    pub kind: TokenKind,
    /// Where it is.
    pub span: Span,
}

/// Splits `source` into tokens, ending with [`TokenKind::End`].
///
/// # Errors
///
/// A character or spelling Redstone doesn't use, or one whose design is still open.
pub fn lex(source: &str) -> Result<Vec<Token>, Diagnostic> {
    let mut lexer = Lexer {
        source,
        characters: source.char_indices().collect(),
        position: 0,
        tokens: Vec::new(),
    };
    lexer.run()?;
    Ok(lexer.tokens)
}

struct Lexer<'source> {
    source: &'source str,
    characters: Vec<(usize, char)>,
    position: usize,
    tokens: Vec<Token>,
}

impl Lexer<'_> {
    fn run(&mut self) -> Result<(), Diagnostic> {
        while let Some((start, character)) = self.peek() {
            match character {
                ' ' | '\t' | '\r' => self.position += 1,
                '\n' => self.single(TokenKind::Newline, start),
                ':' => self.single(TokenKind::Colon, start),
                '(' => self.single(TokenKind::OpenParen, start),
                ')' => self.single(TokenKind::CloseParen, start),
                '=' if self.peek_at(1).is_some_and(|(_, next)| next == '=') => {
                    return Err(Diagnostic::mistake(
                        "Redstone doesn't use `==`",
                        Span::new(start, start + 2),
                    )
                    .because("`is` compares two things; `=` makes a variable"));
                }
                '=' => self.single(TokenKind::Equals, start),
                '"' => self.text(start)?,
                '0'..='9' => self.number(start)?,
                'a'..='z' | 'A'..='Z' | '_' => self.word(start),
                _ => return Err(self.unknown(start, character)),
            }
        }
        let end = self.source.len();
        self.tokens.push(Token {
            kind: TokenKind::End,
            span: Span::new(end, end),
        });
        Ok(())
    }

    fn peek(&self) -> Option<(usize, char)> {
        self.characters.get(self.position).copied()
    }

    fn peek_at(&self, offset: usize) -> Option<(usize, char)> {
        self.characters.get(self.position + offset).copied()
    }

    fn offset(&self) -> usize {
        self.peek().map_or(self.source.len(), |(offset, _)| offset)
    }

    fn single(&mut self, kind: TokenKind, start: usize) {
        self.position += 1;
        self.tokens.push(Token {
            kind,
            span: Span::new(start, start + 1),
        });
    }

    fn take_while(&mut self, keep: impl Fn(char) -> bool) {
        while self.peek().is_some_and(|(_, character)| keep(character)) {
            self.position += 1;
        }
    }

    fn word(&mut self, start: usize) {
        self.take_while(|character| character.is_ascii_alphanumeric() || character == '_');
        let end = self.offset();
        let word = &self.source[start..end];
        let kind = Keyword::from_word(word)
            .map_or_else(|| TokenKind::Name(word.to_owned()), TokenKind::Keyword);
        self.tokens.push(Token {
            kind,
            span: Span::new(start, end),
        });
    }

    fn number(&mut self, start: usize) -> Result<(), Diagnostic> {
        self.take_while(|character| character.is_ascii_digit());
        let whole_end = self.offset();
        let mut decimal = false;
        if self.peek().is_some_and(|(_, character)| character == '.') {
            if !self
                .peek_at(1)
                .is_some_and(|(_, character)| character.is_ascii_digit())
            {
                let span = Span::new(start, whole_end + 1);
                return Err(Diagnostic::mistake("this number ends with its point", span)
                    .because("a decimal has digits on both sides of its point"));
            }
            self.position += 1;
            self.take_while(|character| character.is_ascii_digit());
            decimal = true;
        }
        let end = self.offset();
        if self
            .peek()
            .is_some_and(|(_, character)| character.is_ascii_alphabetic() || character == '_')
        {
            self.take_while(|character| {
                character.is_ascii_alphanumeric() || character == '_' || character == '.'
            });
            let span = Span::new(start, self.offset());
            return Err(Diagnostic::mistake(
                format!(
                    "`{}` is neither a number nor a name",
                    &self.source[span.start..span.end]
                ),
                span,
            )
            .because("a name can't start with a digit"));
        }
        let span = Span::new(start, end);
        let text = &self.source[start..end];
        let kind = if decimal {
            TokenKind::Rational(decimal_literal(text, whole_end - start, span)?)
        } else {
            // D7: a whole number is an int.
            TokenKind::Int(text.parse().map_err(|_| {
                Diagnostic::not_decided("an int too big for 64 bits", Some("D7"), span)
            })?)
        };
        self.tokens.push(Token { kind, span });
        Ok(())
    }

    fn text(&mut self, start: usize) -> Result<(), Diagnostic> {
        self.position += 1;
        let content_start = self.offset();
        self.take_while(|character| character != '"' && character != '\n' && character != '\\');
        let content_end = self.offset();
        match self.peek() {
            Some((offset, '"')) => {
                self.position += 1;
                self.tokens.push(Token {
                    kind: TokenKind::Text(self.source[content_start..content_end].to_owned()),
                    span: Span::new(start, offset + 1),
                });
                Ok(())
            }
            Some((offset, '\\')) => Err(Diagnostic::not_decided(
                "a backslash inside text",
                None,
                Span::new(offset, offset + 1),
            )),
            _ => Err(Diagnostic::mistake(
                "this text has no closing `\"`",
                Span::new(start, content_end),
            )
            .because("text has to end on the line it starts")),
        }
    }

    fn unknown(&mut self, start: usize, character: char) -> Diagnostic {
        let symbols = "+-*/%<>!";
        match character {
            // D6: binary floats come from a friendly spelling that is still open.
            '~' => Diagnostic::not_decided(
                "how a binary float is written",
                Some("D6"),
                Span::new(start, start + 1),
            ),
            '#' => Diagnostic::not_decided("comments", None, Span::new(start, start + 1)),
            _ if symbols.contains(character) => {
                self.take_while(|next| symbols.contains(next) || next == '=');
                let span = Span::new(start, self.offset());
                let spelling = &self.source[span.start..span.end];
                // D16, D18, D20: the words are decided; symbols beside them are not.
                let word = match spelling {
                    "+" => Some("plus"),
                    "-" => Some("minus"),
                    "*" => Some("times"),
                    "/" => Some("divided_by"),
                    "%" => Some("remainder"),
                    ">=" => Some("at_least"),
                    "<=" => Some("at_most"),
                    ">" => Some("greater_than"),
                    "<" => Some("less_than"),
                    "!=" => Some("is_not"),
                    _ => None,
                };
                let diagnostic = Diagnostic::not_decided(
                    format!("symbols such as `{spelling}` beside the words"),
                    Some("D16"),
                    span,
                );
                match word {
                    Some(word) => diagnostic.fix(spelling, word),
                    None => diagnostic,
                }
            }
            '\u{201C}' | '\u{201D}' => Diagnostic::mistake(
                "Redstone doesn't use curly quotes",
                Span::new(start, start + character.len_utf8()),
            )
            .fix(&character.to_string(), "\""),
            _ => Diagnostic::mistake(
                format!("Redstone doesn't use `{character}`"),
                Span::new(start, start + character.len_utf8()),
            ),
        }
    }
}

// D7: a decimal is an exact rational, so `0.1` is 1/10.
fn decimal_literal(text: &str, whole_length: usize, span: Span) -> Result<Rational, Diagnostic> {
    let too_big =
        || Diagnostic::not_decided("a rational whose parts outgrow 64 bits", Some("D7"), span);
    let digits: String = text.chars().filter(char::is_ascii_digit).collect();
    let places = u32::try_from(text.len() - whole_length - 1).map_err(|_| too_big())?;
    let numerator: i128 = digits.parse().map_err(|_| too_big())?;
    let denominator = 10_i128.checked_pow(places).ok_or_else(too_big)?;
    Rational::new(numerator, denominator).map_err(|error| match error {
        RationalError::TooBig | RationalError::DivisionByZero => too_big(),
    })
}
