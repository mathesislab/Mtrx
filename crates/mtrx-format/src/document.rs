//! A small document model for message bodies, parsed from Matrix HTML.
//!
//! Frontends render this model; math keeps its LaTeX source so it can be
//! copied back out.

use linkify::{LinkFinder, LinkKind};
use ruma_common::matrix_uri::MatrixId;
use ruma_html::{
    Html, NodeData, NodeRef,
    matrix::{AnchorUri, MatrixElement},
};

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Document {
    pub blocks: Vec<Block>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Block {
    Paragraph(Vec<Inline>),
    Heading {
        level: u8,
        content: Vec<Inline>,
    },
    Quote(Vec<Block>),
    /// `start` is `None` for bullet lists.
    List {
        start: Option<u64>,
        items: Vec<Vec<Block>>,
    },
    CodeBlock {
        language: Option<String>,
        code: String,
    },
    /// Display math, as LaTeX.
    Math(String),
    Rule,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Inline {
    Text(String),
    Styled {
        style: Style,
        content: Vec<Inline>,
    },
    Code(String),
    Link {
        url: String,
        content: Vec<Inline>,
    },
    /// Inline math, as LaTeX.
    Math(String),
    /// A mention of a user: a `matrix.to` (or `matrix:`) link to them.
    Mention {
        user_id: String,
        name: String,
    },
    LineBreak,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Style {
    Bold,
    Italic,
    Strikethrough,
    Underline,
    Superscript,
    Subscript,
    Spoiler,
}

impl Document {
    /// Parse a Matrix `org.matrix.custom.html` body. Unknown elements keep
    /// their text; reply fallbacks (`<mx-reply>`) are dropped.
    pub fn from_html(html: &str) -> Self {
        let html = Html::parse(html);
        let mut builder = Builder::default();
        for node in html.children() {
            builder.node(&node);
        }
        Self {
            blocks: builder.finish(),
        }
        .linkified()
    }

    /// A plain-text body: one paragraph per line block, line breaks kept.
    pub fn from_plain(text: &str) -> Self {
        let blocks = text
            .split("\n\n")
            .filter(|p| !p.trim().is_empty())
            .map(|paragraph| {
                let mut inlines = Vec::new();
                for (i, line) in paragraph.lines().enumerate() {
                    if i > 0 {
                        inlines.push(Inline::LineBreak);
                    }
                    inlines.push(Inline::Text(line.to_owned()));
                }
                Block::Paragraph(inlines)
            })
            .collect();
        Self { blocks }.linkified()
    }

    /// Bare web addresses in text become links, as most clients show them:
    /// `http://` and `https://` URLs, and `www.` addresses (opened over
    /// https).
    fn linkified(mut self) -> Self {
        let finder = {
            let mut finder = LinkFinder::new();
            finder.kinds(&[LinkKind::Url]).url_must_have_scheme(false);
            finder
        };
        for_each_inlines(&mut self.blocks, &mut |inlines| linkify(inlines, &finder));
        self
    }

    /// Link targets, in order of appearance.
    pub fn links(&self) -> Vec<&str> {
        fn inlines<'a>(list: &'a [Inline], out: &mut Vec<&'a str>) {
            for inline in list {
                match inline {
                    Inline::Link { url, .. } => out.push(url),
                    Inline::Styled { content, .. } => inlines(content, out),
                    _ => {}
                }
            }
        }
        fn blocks<'a>(list: &'a [Block], out: &mut Vec<&'a str>) {
            for block in list {
                match block {
                    Block::Paragraph(content) | Block::Heading { content, .. } => {
                        inlines(content, out)
                    }
                    Block::Quote(inner) => blocks(inner, out),
                    Block::List { items, .. } => items.iter().for_each(|item| blocks(item, out)),
                    Block::CodeBlock { .. } | Block::Math(_) | Block::Rule => {}
                }
            }
        }
        let mut out = Vec::new();
        blocks(&self.blocks, &mut out);
        out
    }

    /// The name the first mention of `user_id` shows, if there is one.
    pub fn mention_name(&self, user_id: &str) -> Option<&str> {
        fn inlines<'a>(list: &'a [Inline], user_id: &str) -> Option<&'a str> {
            list.iter().find_map(|inline| match inline {
                Inline::Mention { user_id: id, name } if id == user_id => Some(name.as_str()),
                Inline::Styled { content, .. } | Inline::Link { content, .. } => {
                    inlines(content, user_id)
                }
                _ => None,
            })
        }
        fn blocks<'a>(list: &'a [Block], user_id: &str) -> Option<&'a str> {
            list.iter().find_map(|block| match block {
                Block::Paragraph(content) | Block::Heading { content, .. } => {
                    inlines(content, user_id)
                }
                Block::Quote(inner) => blocks(inner, user_id),
                Block::List { items, .. } => items.iter().find_map(|item| blocks(item, user_id)),
                Block::CodeBlock { .. } | Block::Math(_) | Block::Rule => None,
            })
        }
        blocks(&self.blocks, user_id)
    }

    pub fn has_math(&self) -> bool {
        !self.formulas().is_empty()
    }

    /// The LaTeX of each formula, inline or displayed, in order of
    /// appearance.
    pub fn formulas(&self) -> Vec<&str> {
        fn inlines<'a>(list: &'a [Inline], out: &mut Vec<&'a str>) {
            for inline in list {
                match inline {
                    Inline::Math(tex) => out.push(tex),
                    Inline::Styled { content, .. } | Inline::Link { content, .. } => {
                        inlines(content, out)
                    }
                    _ => {}
                }
            }
        }
        fn blocks<'a>(list: &'a [Block], out: &mut Vec<&'a str>) {
            for block in list {
                match block {
                    Block::Math(tex) => out.push(tex),
                    Block::Paragraph(content) | Block::Heading { content, .. } => {
                        inlines(content, out)
                    }
                    Block::Quote(inner) => blocks(inner, out),
                    Block::List { items, .. } => items.iter().for_each(|item| blocks(item, out)),
                    Block::CodeBlock { .. } | Block::Rule => {}
                }
            }
        }
        let mut out = Vec::new();
        blocks(&self.blocks, &mut out);
        out
    }

    /// Plain text for copying: Markdown-like, with math as `$…$` / `$$…$$`.
    pub fn to_plain_text(&self) -> String {
        let mut out = String::new();
        blocks_text(&self.blocks, &mut out, "");
        out.trim_end().to_owned()
    }
}

fn blocks_text(blocks: &[Block], out: &mut String, prefix: &str) {
    for (i, block) in blocks.iter().enumerate() {
        if i > 0 {
            out.push_str(prefix.trim_end());
            out.push('\n');
        }
        match block {
            Block::Paragraph(content) => {
                out.push_str(prefix);
                inlines_text(content, out, prefix);
            }
            Block::Heading { level, content } => {
                out.push_str(prefix);
                out.push_str(&"#".repeat(*level as usize));
                out.push(' ');
                inlines_text(content, out, prefix);
            }
            Block::Quote(inner) => blocks_text(inner, out, &format!("{prefix}> ")),
            Block::List { start, items } => {
                for (n, item) in items.iter().enumerate() {
                    let marker = match start {
                        Some(s) => format!("{}. ", s + n as u64),
                        None => "- ".to_owned(),
                    };
                    out.push_str(prefix);
                    out.push_str(&marker);
                    let indent = format!("{prefix}{}", " ".repeat(marker.len()));
                    let mut item_text = String::new();
                    blocks_text(item, &mut item_text, &indent);
                    out.push_str(item_text.trim_start());
                    if !out.ends_with('\n') {
                        out.push('\n');
                    }
                }
                continue;
            }
            Block::CodeBlock { language, code } => {
                out.push_str(prefix);
                out.push_str("```");
                out.push_str(language.as_deref().unwrap_or(""));
                out.push('\n');
                for line in code.trim_end_matches('\n').lines() {
                    out.push_str(prefix);
                    out.push_str(line);
                    out.push('\n');
                }
                out.push_str(prefix);
                out.push_str("```");
            }
            Block::Math(tex) => {
                out.push_str(prefix);
                out.push_str("$$");
                out.push_str(tex);
                out.push_str("$$");
            }
            Block::Rule => {
                out.push_str(prefix);
                out.push_str("---");
            }
        }
        if !out.ends_with('\n') {
            out.push('\n');
        }
    }
}

fn inlines_text(inlines: &[Inline], out: &mut String, prefix: &str) {
    for inline in inlines {
        match inline {
            Inline::Text(t) => out.push_str(t),
            Inline::Styled { content, .. } => inlines_text(content, out, prefix),
            Inline::Code(c) => {
                out.push('`');
                out.push_str(c);
                out.push('`');
            }
            Inline::Link { url, content } => {
                let mut text = String::new();
                inlines_text(content, &mut text, prefix);
                out.push_str(&text);
                if text != *url {
                    out.push_str(" (");
                    out.push_str(url);
                    out.push(')');
                }
            }
            Inline::Math(tex) => {
                out.push('$');
                out.push_str(tex);
                out.push('$');
            }
            Inline::Mention { name, .. } => out.push_str(name),
            Inline::LineBreak => {
                out.push('\n');
                out.push_str(prefix);
            }
        }
    }
}

/// Calls `f` on every inline sequence of `blocks`.
fn for_each_inlines(blocks: &mut [Block], f: &mut impl FnMut(&mut Vec<Inline>)) {
    for block in blocks {
        match block {
            Block::Paragraph(content) | Block::Heading { content, .. } => f(content),
            Block::Quote(inner) => for_each_inlines(inner, f),
            Block::List { items, .. } => {
                for item in items {
                    for_each_inlines(item, f);
                }
            }
            Block::CodeBlock { .. } | Block::Math(_) | Block::Rule => {}
        }
    }
}

/// Split text inlines at URLs found by `finder`; text inside links stays as is.
fn linkify(inlines: &mut Vec<Inline>, finder: &LinkFinder) {
    let mut out = Vec::with_capacity(inlines.len());
    for inline in inlines.drain(..) {
        match inline {
            Inline::Text(text) => {
                // Text and links; what was found but is no web address is
                // text, joined to the text around it.
                let mut text_at = None;
                for span in finder.spans(&text) {
                    let piece = span.as_str();
                    match span.kind().and_then(|_| web_address(piece)) {
                        Some(url) => {
                            text_at = None;
                            out.push(Inline::Link {
                                url,
                                content: vec![Inline::Text(piece.to_owned())],
                            });
                        }
                        None => match text_at.and_then(|at| out.get_mut(at)) {
                            Some(Inline::Text(before)) => before.push_str(piece),
                            _ => {
                                text_at = Some(out.len());
                                out.push(Inline::Text(piece.to_owned()));
                            }
                        },
                    }
                }
            }
            Inline::Styled { style, mut content } => {
                linkify(&mut content, finder);
                out.push(Inline::Styled { style, content });
            }
            other => out.push(other),
        }
    }
    *inlines = out;
}

/// The URL a bare address found in text opens, if it is a web address:
/// `http://` or `https://` (any case), or `www.` (over https). Others, such
/// as `example.org` or `ftp://…`, stay text.
fn web_address(found: &str) -> Option<String> {
    let starts = |prefix: &str| {
        found
            .get(..prefix.len())
            .is_some_and(|start| start.eq_ignore_ascii_case(prefix))
    };
    if starts("http://") || starts("https://") {
        Some(found.to_owned())
    } else if starts("www.") {
        Some(format!("https://{found}"))
    } else {
        None
    }
}

/// Turns the HTML tree into blocks. Inline content outside any block element
/// (common in Matrix HTML, e.g. `hello <b>world</b>`) is gathered into
/// implicit paragraphs.
#[derive(Default)]
struct Builder {
    blocks: Vec<Block>,
    inlines: Vec<Inline>,
}

impl Builder {
    fn finish(mut self) -> Vec<Block> {
        self.flush();
        self.blocks
    }

    fn flush(&mut self) {
        // Trim breaks and whitespace-only text at paragraph edges.
        while matches!(self.inlines.last(), Some(Inline::LineBreak)) {
            self.inlines.pop();
        }
        let blank = self
            .inlines
            .iter()
            .all(|i| matches!(i, Inline::Text(t) if t.trim().is_empty()));
        if !blank {
            self.blocks
                .push(Block::Paragraph(std::mem::take(&mut self.inlines)));
        }
        self.inlines.clear();
    }

    fn block(&mut self, block: Block) {
        self.flush();
        self.blocks.push(block);
    }

    fn node(&mut self, node: &NodeRef) {
        let element = match node.data() {
            NodeData::Text(text) => {
                let text = text.borrow();
                // Newlines between block elements are formatting, not content.
                if !(text.trim().is_empty() && text.contains('\n')) {
                    self.inlines.push(Inline::Text(collapse_whitespace(&text)));
                }
                return;
            }
            NodeData::Element(element) => element.to_matrix().element,
            NodeData::Document | NodeData::Other => return,
        };
        match element {
            MatrixElement::P => {
                self.flush();
                self.children(node);
                self.flush();
            }
            MatrixElement::H(data) => {
                let content = inlines(node);
                self.block(Block::Heading {
                    level: data.level.value(),
                    content,
                });
            }
            MatrixElement::Blockquote => {
                let blocks = child_blocks(node);
                self.block(Block::Quote(blocks));
            }
            MatrixElement::Ul | MatrixElement::Ol(_) => {
                let start = match &element {
                    MatrixElement::Ol(data) => Some(data.start.unwrap_or(1).max(0) as u64),
                    _ => None,
                };
                let items = node
                    .children()
                    .filter(|c| {
                        matches!(
                            c.as_element().map(|e| e.to_matrix().element),
                            Some(MatrixElement::Li)
                        )
                    })
                    .map(|li| child_blocks(&li))
                    .collect();
                self.block(Block::List { start, items });
            }
            MatrixElement::Pre => {
                let mut language = None;
                if let Some(code) = node.first_child()
                    && let Some(MatrixElement::Code(data)) =
                        code.as_element().map(|e| e.to_matrix().element)
                {
                    language = data.language.map(|l| l.to_string());
                }
                self.block(Block::CodeBlock {
                    language,
                    code: text_content(node),
                });
            }
            MatrixElement::Div(data) => match data.maths {
                Some(tex) => self.block(Block::Math(tex.to_string())),
                None => {
                    self.flush();
                    self.children(node);
                    self.flush();
                }
            },
            MatrixElement::Hr => self.block(Block::Rule),
            MatrixElement::Br => self.inlines.push(Inline::LineBreak),
            MatrixElement::MatrixReply => {}
            MatrixElement::Table
            | MatrixElement::Thead
            | MatrixElement::Tbody
            | MatrixElement::Caption
            | MatrixElement::Details
            | MatrixElement::Summary => {
                self.flush();
                self.children(node);
                self.flush();
            }
            MatrixElement::Tr => {
                self.children(node);
                self.flush();
            }
            MatrixElement::Th | MatrixElement::Td => {
                self.children(node);
                self.inlines.push(Inline::Text("  ".into()));
            }
            MatrixElement::Li => {
                let blocks = child_blocks(node);
                self.block(Block::List {
                    start: None,
                    items: vec![blocks],
                });
            }
            _ => self.inlines.extend(inline_element(node, element)),
        }
    }

    fn children(&mut self, node: &NodeRef) {
        for child in node.children() {
            self.node(&child);
        }
    }
}

fn child_blocks(node: &NodeRef) -> Vec<Block> {
    let mut builder = Builder::default();
    builder.children(node);
    builder.finish()
}

/// Inline content of `node`; block elements inside are flattened.
fn inlines(node: &NodeRef) -> Vec<Inline> {
    let mut builder = Builder::default();
    builder.children(node);
    let mut out = Vec::new();
    for block in builder.finish() {
        if !out.is_empty() {
            out.push(Inline::LineBreak);
        }
        match block {
            Block::Paragraph(c) | Block::Heading { content: c, .. } => out.extend(c),
            Block::Math(tex) => out.push(Inline::Math(tex)),
            other => out.push(Inline::Text(
                Document {
                    blocks: vec![other],
                }
                .to_plain_text(),
            )),
        }
    }
    out
}

/// Inline elements; unknown or purely presentational ones (colours) are
/// transparent and yield their content.
fn inline_element(node: &NodeRef, element: MatrixElement) -> Vec<Inline> {
    let styled = |style| {
        vec![Inline::Styled {
            style,
            content: inlines(node),
        }]
    };
    match element {
        MatrixElement::B | MatrixElement::Strong => styled(Style::Bold),
        MatrixElement::I | MatrixElement::Em => styled(Style::Italic),
        MatrixElement::S | MatrixElement::Del => styled(Style::Strikethrough),
        MatrixElement::U => styled(Style::Underline),
        MatrixElement::Sup => styled(Style::Superscript),
        MatrixElement::Sub => styled(Style::Subscript),
        MatrixElement::Code(_) => vec![Inline::Code(text_content(node))],
        MatrixElement::A(data) => match data.href {
            Some(href) => match mentioned_user(&href) {
                Some(user_id) => vec![Inline::Mention {
                    user_id,
                    name: text_content(node),
                }],
                None => vec![Inline::Link {
                    url: anchor_url(&href),
                    content: inlines(node),
                }],
            },
            None => inlines(node),
        },
        MatrixElement::Span(data) => match (data.maths, data.spoiler) {
            (Some(tex), _) => vec![Inline::Math(tex.to_string())],
            (None, Some(_)) => styled(Style::Spoiler),
            (None, None) => inlines(node),
        },
        MatrixElement::Img(data) => {
            vec![Inline::Text(
                data.alt
                    .map(|a| a.to_string())
                    .unwrap_or_else(|| "[image]".into()),
            )]
        }
        _ => inlines(node),
    }
}

/// The user a link is to, if it is a Matrix link to a user: a mention.
fn mentioned_user(href: &AnchorUri) -> Option<String> {
    let id = match href {
        AnchorUri::Matrix(uri) => uri.id(),
        AnchorUri::MatrixTo(uri) => uri.id(),
        _ => return None,
    };
    match id {
        MatrixId::User(user_id) => Some(user_id.to_string()),
        _ => None,
    }
}

fn anchor_url(href: &AnchorUri) -> String {
    match href {
        AnchorUri::Matrix(uri) => uri.to_string(),
        AnchorUri::MatrixTo(uri) => uri.to_string(),
        AnchorUri::Other(url) => url.to_string(),
        _ => String::new(),
    }
}

fn text_content(node: &NodeRef) -> String {
    let mut out = String::new();
    fn walk(node: &NodeRef, out: &mut String) {
        if let Some(text) = node.as_text() {
            out.push_str(&text.borrow());
        }
        for child in node.children() {
            walk(&child, out);
        }
    }
    walk(node, &mut out);
    out
}

/// HTML collapses runs of whitespace (outside `<pre>`).
fn collapse_whitespace(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut last_space = false;
    for c in text.chars() {
        if c.is_whitespace() {
            if !last_space {
                out.push(' ');
            }
            last_space = true;
        } else {
            out.push(c);
            last_space = false;
        }
    }
    out
}
