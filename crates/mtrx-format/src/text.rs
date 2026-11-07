//! A [`Document`] flattened for native text views (the phones): blocks top
//! to bottom, each with its quote and list nesting, and inline content as
//! styled runs. Math stays LaTeX, typeset one formula at a time.

use crate::{Block, Document, Inline, Style};

/// A block of a flattened document.
#[derive(Clone, Debug, PartialEq)]
pub struct TextBlock {
    pub kind: BlockKind,
    /// Quote bars to its left.
    pub quotes: u8,
    /// List indents.
    pub lists: u8,
    /// "•" or "3." on the first block of a list item.
    pub marker: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum BlockKind {
    Paragraph(Vec<Run>),
    Heading {
        level: u8,
        runs: Vec<Run>,
    },
    /// Trailing newlines trimmed.
    Code {
        code: String,
        language: Option<String>,
    },
    /// Display math.
    Math(Formula),
    Rule,
}

/// A piece of inline content in one style.
#[derive(Clone, Debug, PartialEq)]
pub enum Run {
    /// Line breaks are "\n".
    Text {
        text: String,
        style: TextStyle,
        link: Option<String>,
    },
    Code {
        code: String,
        style: TextStyle,
        link: Option<String>,
    },
    /// Inline math.
    Math {
        formula: Formula,
        link: Option<String>,
    },
    Mention {
        user_id: String,
        name: String,
        style: TextStyle,
    },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct TextStyle {
    pub bold: bool,
    pub italic: bool,
    pub strikethrough: bool,
    pub underline: bool,
    pub superscript: bool,
    pub subscript: bool,
    pub spoiler: bool,
}

impl TextStyle {
    fn with(mut self, style: Style) -> Self {
        match style {
            Style::Bold => self.bold = true,
            Style::Italic => self.italic = true,
            Style::Strikethrough => self.strikethrough = true,
            Style::Underline => self.underline = true,
            Style::Superscript => self.superscript = true,
            Style::Subscript => self.subscript = true,
            Style::Spoiler => self.spoiler = true,
        }
        self
    }
}

/// A formula: LaTeX, displayed (its own block) or inline.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Formula {
    pub tex: String,
    pub display: bool,
}

impl Document {
    /// The document as blocks of runs. Adjacent runs with the same style and
    /// link are merged.
    pub fn text_blocks(&self) -> Vec<TextBlock> {
        let mut flattener = Flattener::default();
        flattener.blocks(&self.blocks, Nesting::default());
        flattener.out
    }
}

#[derive(Clone, Copy, Default)]
struct Nesting {
    quotes: u8,
    lists: u8,
}

#[derive(Default)]
struct Flattener {
    out: Vec<TextBlock>,
    /// The marker of the list item begun, for its first block.
    marker: Option<String>,
}

impl Flattener {
    fn push(&mut self, kind: BlockKind, nesting: Nesting) {
        self.out.push(TextBlock {
            kind,
            quotes: nesting.quotes,
            lists: nesting.lists,
            marker: self.marker.take(),
        });
    }

    /// An item's marker not yet shown (the item is empty, or its first block
    /// is a list) goes on an empty line of its own.
    fn flush_marker(&mut self, nesting: Nesting) {
        if self.marker.is_some() {
            self.push(BlockKind::Paragraph(Vec::new()), nesting);
        }
    }

    fn blocks(&mut self, blocks: &[Block], nesting: Nesting) {
        for block in blocks {
            match block {
                Block::Paragraph(content) => {
                    self.push(BlockKind::Paragraph(runs(content)), nesting);
                }
                Block::Heading { level, content } => self.push(
                    BlockKind::Heading {
                        level: *level,
                        runs: runs(content),
                    },
                    nesting,
                ),
                Block::Quote(inner) => {
                    let quoted = Nesting {
                        quotes: nesting.quotes.saturating_add(1),
                        ..nesting
                    };
                    self.blocks(inner, quoted);
                }
                Block::List { start, items } => {
                    let listed = Nesting {
                        lists: nesting.lists.saturating_add(1),
                        ..nesting
                    };
                    for (n, item) in items.iter().enumerate() {
                        self.flush_marker(nesting);
                        self.marker = Some(match start {
                            Some(start) => format!("{}.", start.saturating_add(n as u64)),
                            None => "•".to_owned(),
                        });
                        self.blocks(item, listed);
                        self.flush_marker(listed);
                    }
                }
                Block::CodeBlock { language, code } => self.push(
                    BlockKind::Code {
                        code: code.trim_end_matches('\n').to_owned(),
                        language: language.clone(),
                    },
                    nesting,
                ),
                Block::Math(tex) => self.push(
                    BlockKind::Math(Formula {
                        tex: tex.clone(),
                        display: true,
                    }),
                    nesting,
                ),
                Block::Rule => self.push(BlockKind::Rule, nesting),
            }
        }
    }
}

fn runs(content: &[Inline]) -> Vec<Run> {
    let mut out = Vec::new();
    inline_runs(content, TextStyle::default(), None, &mut out);
    out
}

fn inline_runs(content: &[Inline], style: TextStyle, link: Option<&str>, out: &mut Vec<Run>) {
    let link_owned = || link.map(str::to_owned);
    for inline in content {
        match inline {
            Inline::Text(text) => push_run(
                out,
                Run::Text {
                    text: text.clone(),
                    style,
                    link: link_owned(),
                },
            ),
            Inline::Styled {
                style: added,
                content,
            } => inline_runs(content, style.with(*added), link, out),
            Inline::Code(code) => push_run(
                out,
                Run::Code {
                    code: code.clone(),
                    style,
                    link: link_owned(),
                },
            ),
            Inline::Link { url, content } => inline_runs(content, style, Some(url), out),
            Inline::Math(tex) => out.push(Run::Math {
                formula: Formula {
                    tex: tex.clone(),
                    display: false,
                },
                link: link_owned(),
            }),
            Inline::Mention { user_id, name } => out.push(Run::Mention {
                user_id: user_id.clone(),
                name: name.clone(),
                style,
            }),
            Inline::LineBreak => push_run(
                out,
                Run::Text {
                    text: "\n".to_owned(),
                    style,
                    link: link_owned(),
                },
            ),
        }
    }
}

/// Appends `run`, merged into the last run if both are text (or both code)
/// in the same style with the same link.
fn push_run(out: &mut Vec<Run>, run: Run) {
    match (out.last_mut(), run) {
        (
            Some(Run::Text { text, style, link }),
            Run::Text {
                text: more,
                style: s,
                link: l,
            },
        ) if *style == s && *link == l => text.push_str(&more),
        (
            Some(Run::Code { code, style, link }),
            Run::Code {
                code: more,
                style: s,
                link: l,
            },
        ) if *style == s && *link == l => code.push_str(&more),
        (_, run) => out.push(run),
    }
}
