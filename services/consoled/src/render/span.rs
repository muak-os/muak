//! Styled text primitives for terminal rendering.

use std::io::{self, Write};

use crossterm::cursor::MoveTo;
use crossterm::queue;
use crossterm::style::{Attribute, Color, Print, ResetColor, SetAttribute, SetForegroundColor};

/// A single styled segment within a line.
#[derive(Debug, Clone)]
pub(super) struct Span {
    pub(super) color: Option<Color>,
    pub(super) bold: bool,
    pub(super) text: String,
}

impl Span {
    pub(super) fn new(color: Color, text: impl Into<String>) -> Self {
        Self {
            color: Some(color),
            bold: false,
            text: text.into(),
        }
    }

    pub(super) fn bold(color: Color, text: impl Into<String>) -> Self {
        Self {
            color: Some(color),
            bold: true,
            text: text.into(),
        }
    }

    pub(super) fn reset(text: impl Into<String>) -> Self {
        Self {
            color: None,
            bold: false,
            text: text.into(),
        }
    }
}

/// A logical line composed of styled spans, with a starting column.
#[derive(Debug, Clone, Default)]
pub(super) struct Line {
    pub(super) col: u16,
    pub(super) spans: Vec<Span>,
}

impl Line {
    pub(super) fn new(col: u16) -> Self {
        Self {
            col,
            spans: Vec::new(),
        }
    }

    pub(super) fn push(&mut self, span: Span) {
        self.spans.push(span);
    }

    pub(super) fn write_to(&self, w: &mut impl Write, row: u16) -> io::Result<()> {
        queue!(w, MoveTo(self.col, row))?;
        for span in &self.spans {
            write_span(w, span)?;
        }

        Ok(())
    }
}

fn write_span(w: &mut impl Write, span: &Span) -> io::Result<()> {
    match span.color {
        Some(color) => queue!(w, SetForegroundColor(color))?,
        None => queue!(w, ResetColor)?,
    }
    if span.bold {
        queue!(w, SetAttribute(Attribute::Bold))?;
    }
    queue!(w, Print(&span.text))?;
    if span.bold {
        queue!(w, SetAttribute(Attribute::Reset))?;
    }

    Ok(())
}
