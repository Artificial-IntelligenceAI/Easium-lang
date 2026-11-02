//! Problems shown to the person running a program: a line and column, plain words, and a suggestion where one
//! exists (rule 8).

use std::fmt::Write;

/// A byte range in the source.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span {
    /// The first byte.
    pub start: usize,
    /// One past the last byte.
    pub end: usize,
}

impl Span {
    /// The span from `start` up to, not including, `end`.
    #[must_use]
    pub fn new(start: usize, end: usize) -> Self {
        Self { start, end }
    }

    /// The span from the start of `self` to the end of `other`.
    #[must_use]
    pub fn to(self, other: Self) -> Self {
        Self::new(self.start, other.end)
    }
}

/// What kind of problem a diagnostic reports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// The program is wrong.
    Error,
    /// The program uses something whose design is still open in `DECISIONS.md` (rule 2).
    NotDecided,
    /// Something went wrong inside Redstone itself (rule 6).
    Bug,
}

/// One problem, with where it is and what to do about it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    /// What kind of problem it is.
    pub kind: Kind,
    /// What went wrong, in plain words.
    pub message: String,
    /// Where.
    pub span: Span,
    /// What to try, where there is a suggestion.
    pub hint: Option<String>,
}

impl Diagnostic {
    /// A mistake in the program.
    #[must_use]
    pub fn error(message: impl Into<String>, span: Span) -> Self {
        Self {
            kind: Kind::Error,
            message: message.into(),
            span,
            hint: None,
        }
    }

    /// Something the program uses whose design is still open; `decision` names the entry it belongs to.
    #[must_use]
    pub fn not_decided(what: impl Into<String>, decision: &str, span: Span) -> Self {
        Self {
            kind: Kind::NotDecided,
            message: format!("{} ({decision})", what.into()),
            span,
            hint: None,
        }
    }

    /// A failure inside Redstone that a correct program can still reach.
    #[must_use]
    pub fn bug(what: impl Into<String>, span: Span) -> Self {
        Self {
            kind: Kind::Bug,
            message: what.into(),
            span,
            hint: None,
        }
    }

    /// The same diagnostic with a suggestion.
    #[must_use]
    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    /// The diagnostic as it is shown, pointing into `source`, which was read from `path`.
    #[must_use]
    pub fn render(&self, source: &str, path: &str) -> String {
        let label = match self.kind {
            Kind::Error => "error",
            Kind::NotDecided => "not decided yet",
            Kind::Bug => "Redstone's bug, not yours",
        };
        let start = floor_char_boundary(source, self.span.start);
        let line_start = source[..start].rfind('\n').map_or(0, |index| index + 1);
        let line_end = source[start..]
            .find('\n')
            .map_or(source.len(), |index| start + index);
        let line_number = source[..start].matches('\n').count() + 1;
        let column = source[line_start..start].chars().count() + 1;
        let line_text = &source[line_start..line_end];
        let end = floor_char_boundary(source, self.span.end.clamp(start, line_end));
        let width = source[start..end].chars().count().max(1);
        // Tabs stay tabs under the line, so the carets line up however the terminal shows them.
        let padding: String = source[line_start..start]
            .chars()
            .map(|character| if character == '\t' { '\t' } else { ' ' })
            .collect();
        let gutter = " ".repeat(line_number.to_string().len());

        let mut text = String::new();
        let _ = writeln!(text, "{label}: {}", self.message);
        let _ = writeln!(text, "{gutter}--> {path}:{line_number}:{column}");
        let _ = writeln!(text, "{gutter} |");
        let _ = writeln!(text, "{line_number} | {line_text}");
        let _ = writeln!(text, "{gutter} | {padding}{}", "^".repeat(width));
        match (&self.hint, self.kind) {
            (Some(hint), _) => {
                let _ = writeln!(text, "{gutter} = {hint}");
            }
            (None, Kind::NotDecided) => {
                let _ = writeln!(
                    text,
                    "{gutter} = this part of Redstone's design is still open in DECISIONS.md"
                );
            }
            (None, Kind::Bug) => {
                let _ = writeln!(text, "{gutter} = please report it");
            }
            (None, Kind::Error) => {}
        }
        text
    }
}

fn floor_char_boundary(source: &str, index: usize) -> usize {
    let mut index = index.min(source.len());
    while !source.is_char_boundary(index) {
        index -= 1;
    }
    index
}
