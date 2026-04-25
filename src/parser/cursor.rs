//! Line cursor over preprocessed lines.

use crate::ast::Location;
use crate::preprocessor::PreprocessedLine;

pub struct Cursor<'a> {
    lines: &'a [PreprocessedLine],
    pos: usize,
}

impl<'a> Cursor<'a> {
    pub fn new(lines: &'a [PreprocessedLine]) -> Self {
        Self { lines, pos: 0 }
    }

    pub fn at_end(&self) -> bool {
        self.pos >= self.lines.len()
    }

    /// Current position; pair with [`seek`] to roll back speculative reads
    /// (e.g. block metadata that turned out to belong to an outer scope).
    pub fn pos(&self) -> usize {
        self.pos
    }

    pub fn seek(&mut self, pos: usize) {
        self.pos = pos.min(self.lines.len());
    }

    pub fn peek(&self) -> Option<&'a PreprocessedLine> {
        self.lines.get(self.pos)
    }

    pub fn peek_text(&self) -> Option<&'a str> {
        self.peek().map(|l| l.text.as_str())
    }

    pub fn advance(&mut self) -> Option<&'a PreprocessedLine> {
        let out = self.lines.get(self.pos);
        if out.is_some() {
            self.pos += 1;
        }
        out
    }

    pub fn skip_blank_lines(&mut self) {
        while let Some(line) = self.peek() {
            if line.text.trim().is_empty() {
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    pub fn current_location(&self) -> Location {
        match self.peek() {
            Some(line) => line.location.clone(),
            None => self
                .lines
                .last()
                .map(|l| l.location.clone())
                .unwrap_or_else(Location::synthetic),
        }
    }
}
