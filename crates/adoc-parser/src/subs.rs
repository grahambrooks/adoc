//! Substitution pipeline configuration.
//!
//! AsciiDoc defines six substitution groups applied in order:
//! specialchars, quotes, attributes, replacements, macros, post-replacements.
//! Each block type declares which apply; the `subs` attribute overrides.

#[derive(Debug, Clone, Copy)]
pub struct Subs {
    pub specialchars: bool,
    pub quotes: bool,
    pub attributes: bool,
    pub replacements: bool,
    pub macros: bool,
    pub post_replacements: bool,
}

impl Subs {
    pub const NORMAL: Subs = Subs {
        specialchars: true,
        quotes: true,
        attributes: true,
        replacements: true,
        macros: true,
        post_replacements: true,
    };

    pub const VERBATIM: Subs = Subs {
        specialchars: true,
        quotes: false,
        attributes: false,
        replacements: false,
        macros: false,
        post_replacements: false,
    };

    pub const HEADER: Subs = Subs {
        specialchars: true,
        quotes: true,
        attributes: true,
        replacements: true,
        macros: true,
        post_replacements: false,
    };

    pub const NONE: Subs = Subs {
        specialchars: false,
        quotes: false,
        attributes: false,
        replacements: false,
        macros: false,
        post_replacements: false,
    };
}
