//! A message's text as the apps draw it: blocks of styled runs, flat (no
//! recursive records), each formula to typeset with
//! [`crate::typeset_formula`].

use mtrx_core::{self as core, UserId};
use mtrx_format as format;

/// A block of a message's text, top to bottom. Nesting is flat: lay the
/// blocks out in one column.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct TextBlock {
    pub kind: BlockKind,
    /// Quote bars to its left.
    pub quotes: u8,
    /// List indents.
    pub lists: u8,
    /// "•" or "3." on the first block of a list item, in a column before it.
    pub marker: Option<String>,
}

#[derive(Clone, Debug, PartialEq, uniffi::Enum)]
pub enum BlockKind {
    Paragraph {
        runs: Vec<TextRun>,
    },
    /// Every level in the same size.
    Heading {
        level: u8,
        runs: Vec<TextRun>,
    },
    /// Monospace, wrapped; trailing newlines trimmed.
    Code {
        code: String,
        language: Option<String>,
    },
    /// Display math, centred.
    Math {
        formula: Formula,
    },
    Rule,
}

/// Inline content in one style. Runs in a link carry its URL.
#[derive(Clone, Debug, PartialEq, uniffi::Enum)]
pub enum TextRun {
    /// Line breaks are "\n".
    Text {
        text: String,
        style: TextStyle,
        link: Option<String>,
    },
    /// Inline code: monospace on a tinted background.
    Code {
        code: String,
        style: TextStyle,
        link: Option<String>,
    },
    /// Inline math, on the line's baseline.
    Math {
        formula: Formula,
        link: Option<String>,
    },
    /// Tap: show the person. In the link colour; `you`: drawn bold.
    Mention {
        user_id: String,
        name: String,
        style: TextStyle,
        you: bool,
    },
}

/// Superscript and subscript: raised or lowered, smaller. A spoiler: a
/// filled box until tapped.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, uniffi::Record)]
pub struct TextStyle {
    pub bold: bool,
    pub italic: bool,
    pub strikethrough: bool,
    pub underline: bool,
    pub superscript: bool,
    pub subscript: bool,
    pub spoiler: bool,
}

/// A formula, as LaTeX: displayed (a block of its own) or inline. Until
/// typeset, or if it cannot be, show `tex` as `$tex$` in monospace.
#[derive(Clone, Debug, PartialEq, Eq, Hash, uniffi::Record)]
pub struct Formula {
    pub tex: String,
    pub display: bool,
}

/// What `message`, sent by `sender`, shows as text, with `own` our user ID.
/// Empty for a file without a caption. An emote starts with the sender's
/// name, and all its runs are italic.
pub(crate) fn message_text(
    message: &core::Message,
    sender: &core::User,
    own: &UserId,
) -> Vec<TextBlock> {
    let Some(document) = message.text() else {
        return Vec::new();
    };
    let mut blocks = document.text_blocks();
    if message.kind == core::MessageKind::Emote {
        emote(&mut blocks, &sender.display_name());
    }
    blocks
        .into_iter()
        .map(|block| TextBlock::new(block, own))
        .collect()
}

/// The sender's `name` before the text (on a line of its own if the text
/// starts with no paragraph), all of it italic.
fn emote(blocks: &mut Vec<format::TextBlock>, name: &str) {
    let starts_paragraph = matches!(
        blocks.first(),
        Some(format::TextBlock {
            kind: format::BlockKind::Paragraph(_),
            quotes: 0,
            lists: 0,
            ..
        })
    );
    if !starts_paragraph {
        blocks.insert(
            0,
            format::TextBlock {
                kind: format::BlockKind::Paragraph(Vec::new()),
                quotes: 0,
                lists: 0,
                marker: None,
            },
        );
    }
    let italic = format::TextStyle {
        italic: true,
        ..format::TextStyle::default()
    };
    for block in blocks.iter_mut() {
        if let format::BlockKind::Paragraph(runs) | format::BlockKind::Heading { runs, .. } =
            &mut block.kind
        {
            for run in runs {
                match run {
                    format::Run::Text { style, .. }
                    | format::Run::Code { style, .. }
                    | format::Run::Mention { style, .. } => style.italic = true,
                    format::Run::Math { .. } => {}
                }
            }
        }
    }
    let Some(format::TextBlock {
        kind: format::BlockKind::Paragraph(runs),
        ..
    }) = blocks.first_mut()
    else {
        unreachable!("a paragraph was put first")
    };
    let separator = if runs.is_empty() { "" } else { " " };
    let named = format!("{name}{separator}");
    match runs.first_mut() {
        // Joined to text in the same style.
        Some(format::Run::Text { text, style, link }) if *style == italic && link.is_none() => {
            text.insert_str(0, &named)
        }
        _ => runs.insert(
            0,
            format::Run::Text {
                text: named,
                style: italic,
                link: None,
            },
        ),
    }
}

impl TextBlock {
    fn new(block: format::TextBlock, own: &UserId) -> Self {
        let runs =
            |runs: Vec<format::Run>| runs.into_iter().map(|run| TextRun::new(run, own)).collect();
        Self {
            kind: match block.kind {
                format::BlockKind::Paragraph(content) => BlockKind::Paragraph {
                    runs: runs(content),
                },
                format::BlockKind::Heading {
                    level,
                    runs: content,
                } => BlockKind::Heading {
                    level,
                    runs: runs(content),
                },
                format::BlockKind::Code { code, language } => BlockKind::Code { code, language },
                format::BlockKind::Math(formula) => BlockKind::Math {
                    formula: formula.into(),
                },
                format::BlockKind::Rule => BlockKind::Rule,
            },
            quotes: block.quotes,
            lists: block.lists,
            marker: block.marker,
        }
    }
}

impl TextRun {
    fn new(run: format::Run, own: &UserId) -> Self {
        match run {
            format::Run::Text { text, style, link } => Self::Text {
                text,
                style: style.into(),
                link,
            },
            format::Run::Code { code, style, link } => Self::Code {
                code,
                style: style.into(),
                link,
            },
            format::Run::Math { formula, link } => Self::Math {
                formula: formula.into(),
                link,
            },
            format::Run::Mention {
                user_id,
                name,
                style,
            } => Self::Mention {
                you: user_id == own.as_str(),
                user_id,
                name,
                style: style.into(),
            },
        }
    }
}

impl From<format::TextStyle> for TextStyle {
    fn from(style: format::TextStyle) -> Self {
        let format::TextStyle {
            bold,
            italic,
            strikethrough,
            underline,
            superscript,
            subscript,
            spoiler,
        } = style;
        Self {
            bold,
            italic,
            strikethrough,
            underline,
            superscript,
            subscript,
            spoiler,
        }
    }
}

impl From<format::Formula> for Formula {
    fn from(formula: format::Formula) -> Self {
        Self {
            tex: formula.tex,
            display: formula.display,
        }
    }
}

impl From<Formula> for format::Formula {
    fn from(formula: Formula) -> Self {
        Self {
            tex: formula.tex,
            display: formula.display,
        }
    }
}
