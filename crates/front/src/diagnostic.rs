//! Problems shown to the person running a program, in Radium's style made friendlier (D31): only what's certain,
//! the place twice, the code line with a pointer, and lined-up `what:`, `why:` and `fix:` labels.

use std::fmt::Write;

use unicode_segmentation::UnicodeSegmentation;

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

/// What kind of problem a diagnostic reports; the message's first word says it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// The program has a mistake: `stopped:`.
    Mistake,
    /// The program uses something whose design is still open in `DECISIONS.md` (rule 2): `not decided yet:`.
    NotDecided,
    /// Something went wrong inside Redstone itself (rule 6).
    Bug,
}

/// One problem: where it is, what is wrong, why, and its fix when there is exactly one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    /// What kind of problem it is.
    pub kind: Kind,
    /// Where.
    pub span: Span,
    /// What is wrong, in plain words.
    pub what: String,
    /// The rule behind it, in plain words.
    pub why: Option<String>,
    /// The one fix, as code: `` `is not` → `is_not` ``. Shown in place of `what`.
    pub fix: Option<String>,
}

impl Diagnostic {
    /// A mistake in the program.
    #[must_use]
    pub fn mistake(what: impl Into<String>, span: Span) -> Self {
        Self {
            kind: Kind::Mistake,
            span,
            what: what.into(),
            why: None,
            fix: None,
        }
    }

    /// Something the program uses whose design is still open; `decision` names the entry it belongs to, if any.
    #[must_use]
    pub fn not_decided(what: impl Into<String>, decision: Option<&str>, span: Span) -> Self {
        let why = match decision {
            Some(decision) => format!("this part of Redstone's design is still open ({decision})"),
            None => "Redstone's design doesn't cover this yet".to_owned(),
        };
        Self {
            kind: Kind::NotDecided,
            span,
            what: what.into(),
            why: Some(why),
            fix: None,
        }
    }

    /// A failure inside Redstone that a correct program can still reach.
    #[must_use]
    pub fn bug(what: impl Into<String>, span: Span) -> Self {
        Self {
            kind: Kind::Bug,
            span,
            what: what.into(),
            why: Some("please report it".to_owned()),
            fix: None,
        }
    }

    /// The same diagnostic with the rule behind it.
    #[must_use]
    pub fn because(mut self, why: impl Into<String>) -> Self {
        self.why = Some(why.into());
        self
    }

    /// The same diagnostic with its one certain fix: `from` becomes `to`.
    #[must_use]
    pub fn fix(mut self, from: &str, to: &str) -> Self {
        self.fix = Some(format!("`{from}` → `{to}`"));
        self
    }

    /// The diagnostic as it is shown, pointing into `source`, which was read from `path`.
    #[must_use]
    pub fn render(&self, source: &str, path: &str) -> String {
        let label = match self.kind {
            Kind::Mistake => "stopped",
            Kind::NotDecided => "not decided yet",
            Kind::Bug => "Redstone's bug, not yours",
        };
        let start = floor_char_boundary(source, self.span.start);
        let line_start = source[..start].rfind('\n').map_or(0, |index| index + 1);
        let line_end = source[start..]
            .find('\n')
            .map_or(source.len(), |index| start + index);
        let line_number = source[..start].matches('\n').count() + 1;
        let before = &source[line_start..start];
        // D31: the software column counts UTF-8 bytes; the human one counts grapheme clusters.
        let byte_column = before.len() + 1;
        let human_column = before.graphemes(true).count() + 1;
        let end = floor_char_boundary(source, self.span.end.clamp(start, line_end));
        let width = source[start..end].graphemes(true).count().max(1);
        // Tabs stay tabs under the line, so the pointer lines up however the terminal shows them.
        let padding: String = before
            .graphemes(true)
            .map(|grapheme| if grapheme == "\t" { '\t' } else { ' ' })
            .collect();
        let margin = " ".repeat(line_number.to_string().len());

        let mut text = String::new();
        let _ = writeln!(
            text,
            "{label}: {path}:{line_number}:{byte_column}, line {line_number} column {human_column}"
        );
        let _ = writeln!(text, " {line_number} | {}", &source[line_start..line_end]);
        let _ = writeln!(text, " {margin} | {padding}{}", "^".repeat(width));
        match &self.fix {
            Some(fix) => {
                let _ = writeln!(text, " fix:  {fix}");
            }
            None => {
                let _ = writeln!(text, " what: {}", self.what);
            }
        }
        if let Some(why) = &self.why {
            let _ = writeln!(text, " why:  {why}");
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
