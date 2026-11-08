//! Copying a document, or the selected part of one, as HTML for the
//! clipboard: what other apps (Mail, a word processor) keep of its
//! formatting, links and lists. Math stays `$…$`, as in its plain text.
//! The same HTML, made safe, for a page of its own (an export).

use std::ops::Range;

use crate::{
    document::{Block, Document, Inline, Style},
    markdown::escape,
};

impl Document {
    /// The part of the document selected by `keep`, which gets each leaf's
    /// index and gives the bytes of it kept, if any. Leaves are, in reading
    /// order, each text, inline code, mention and formula, and each code
    /// block and displayed formula, as shown: code blocks without trailing
    /// whitespace, formulas as `$…$` and `$$…$$`, mentions by name. Part of
    /// a formula or mention is kept as text. Blocks and styles left empty
    /// go, and rules with them.
    pub fn clipped(&self, keep: impl FnMut(usize) -> Option<Range<usize>>) -> Self {
        let mut clip = Clip { keep, leaf: 0 };
        Self {
            blocks: clip.blocks(&self.blocks),
        }
    }

    /// HTML for the clipboard: math as `$…$` and `$$…$$` text, mentions as
    /// `matrix.to` links.
    pub fn to_html(&self) -> String {
        let mut out = String::new();
        blocks_html(&self.blocks, &mut out, Target::Clipboard);
        out
    }

    /// HTML for a web page of its own, such as an export: only `http`,
    /// `https` and `mailto` links (others are their text), math as `$…$`
    /// and `$$…$$` text in `math` spans and paragraphs, spoilers in
    /// `spoiler` spans.
    pub fn to_page_html(&self) -> String {
        let mut out = String::new();
        blocks_html(&self.blocks, &mut out, Target::Page);
        out
    }
}

/// Where HTML goes: the clipboard, or a page of its own.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Target {
    Clipboard,
    Page,
}

/// Whether a page may link to `url`: the web and mail only, never
/// `javascript:` or a local file.
fn is_safe_link(url: &str) -> bool {
    let Some((scheme, _)) = url.split_once(':') else {
        return false;
    };
    ["https", "http", "mailto"]
        .iter()
        .any(|safe| scheme.eq_ignore_ascii_case(safe))
}

struct Clip<F> {
    keep: F,
    /// The index of the next leaf.
    leaf: usize,
}

impl<F: FnMut(usize) -> Option<Range<usize>>> Clip<F> {
    /// What is kept of the next leaf, shown as `shown`, and whether that is
    /// all of it.
    fn leaf<'s>(&mut self, shown: &'s str) -> Option<(&'s str, bool)> {
        let index = self.leaf;
        self.leaf += 1;
        let range = (self.keep)(index)?;
        let whole = range.start == 0 && range.end >= shown.len();
        let kept = shown.get(range.start..range.end.min(shown.len()))?;
        (!kept.is_empty()).then_some((kept, whole))
    }

    fn blocks(&mut self, blocks: &[Block]) -> Vec<Block> {
        blocks
            .iter()
            .filter_map(|block| self.block(block))
            .collect()
    }

    fn block(&mut self, block: &Block) -> Option<Block> {
        match block {
            Block::Paragraph(content) => self.paragraph(content).map(Block::Paragraph),
            Block::Heading { level, content } => {
                self.paragraph(content).map(|content| Block::Heading {
                    level: *level,
                    content,
                })
            }
            Block::Quote(inner) => {
                let inner = self.blocks(inner);
                (!inner.is_empty()).then_some(Block::Quote(inner))
            }
            Block::List { start, items } => {
                // Numbered from the first item kept.
                let mut skipped = 0;
                let mut kept = Vec::new();
                for item in items {
                    let item = self.blocks(item);
                    if !item.is_empty() {
                        kept.push(item);
                    } else if kept.is_empty() {
                        skipped += 1;
                    }
                }
                (!kept.is_empty()).then(|| Block::List {
                    start: start.map(|start| start + skipped),
                    items: kept,
                })
            }
            Block::CodeBlock { language, code } => {
                let (kept, _) = self.leaf(code.trim_end())?;
                Some(Block::CodeBlock {
                    language: language.clone(),
                    code: kept.to_owned(),
                })
            }
            Block::Math(tex) => {
                let shown = format!("$${tex}$$");
                let (kept, whole) = self.leaf(&shown)?;
                Some(if whole {
                    Block::Math(tex.clone())
                } else {
                    Block::Paragraph(vec![Inline::Text(kept.to_owned())])
                })
            }
            Block::Rule => None,
        }
    }

    /// The inlines kept; line breaks at their edges go with what is not.
    fn paragraph(&mut self, content: &[Inline]) -> Option<Vec<Inline>> {
        let mut kept = self.inlines(content);
        let start = kept
            .iter()
            .position(|inline| *inline != Inline::LineBreak)?;
        kept.drain(..start);
        while kept.last() == Some(&Inline::LineBreak) {
            kept.pop();
        }
        Some(kept)
    }

    fn inlines(&mut self, content: &[Inline]) -> Vec<Inline> {
        let mut out = Vec::new();
        for inline in content {
            let kept = match inline {
                Inline::Text(text) => self
                    .leaf(text)
                    .map(|(kept, _)| Inline::Text(kept.to_owned())),
                Inline::Code(code) => self
                    .leaf(code)
                    .map(|(kept, _)| Inline::Code(kept.to_owned())),
                Inline::Mention { name, .. } => self.leaf(name).map(|(kept, whole)| {
                    if whole {
                        inline.clone()
                    } else {
                        Inline::Text(kept.to_owned())
                    }
                }),
                Inline::Math(tex) => {
                    let shown = format!("${tex}$");
                    self.leaf(&shown).map(|(kept, whole)| {
                        if whole {
                            inline.clone()
                        } else {
                            Inline::Text(kept.to_owned())
                        }
                    })
                }
                Inline::Styled { style, content } => {
                    let content = self.inlines(content);
                    has_text(&content).then_some(Inline::Styled {
                        style: *style,
                        content,
                    })
                }
                Inline::Link { url, content } => {
                    let content = self.inlines(content);
                    has_text(&content).then(|| Inline::Link {
                        url: url.clone(),
                        content,
                    })
                }
                Inline::LineBreak => Some(Inline::LineBreak),
            };
            out.extend(kept);
        }
        out
    }
}

fn has_text(inlines: &[Inline]) -> bool {
    inlines.iter().any(|inline| *inline != Inline::LineBreak)
}

fn blocks_html(blocks: &[Block], out: &mut String, target: Target) {
    for block in blocks {
        match block {
            Block::Paragraph(content) => {
                out.push_str("<p>");
                inlines_html(content, out, target);
                out.push_str("</p>");
            }
            Block::Heading { level, content } => {
                out.push_str(&format!("<h{level}>"));
                inlines_html(content, out, target);
                out.push_str(&format!("</h{level}>"));
            }
            Block::Quote(inner) => {
                out.push_str("<blockquote>");
                blocks_html(inner, out, target);
                out.push_str("</blockquote>");
            }
            Block::List { start, items } => {
                let tag = if start.is_some() { "ol" } else { "ul" };
                match start {
                    Some(start) if *start != 1 => out.push_str(&format!("<ol start=\"{start}\">")),
                    _ => out.push_str(&format!("<{tag}>")),
                }
                for item in items {
                    out.push_str("<li>");
                    // A one-paragraph item, as most are, without the <p>.
                    match &item[..] {
                        [Block::Paragraph(content)] => inlines_html(content, out, target),
                        blocks => blocks_html(blocks, out, target),
                    }
                    out.push_str("</li>");
                }
                out.push_str(&format!("</{tag}>"));
            }
            Block::CodeBlock { language, code } => {
                match language {
                    Some(language) => out.push_str(&format!(
                        "<pre><code class=\"language-{}\">",
                        escape(language, true)
                    )),
                    None => out.push_str("<pre><code>"),
                }
                out.push_str(&escape(code, false));
                out.push_str("</code></pre>");
            }
            Block::Math(tex) => {
                out.push_str(match target {
                    Target::Clipboard => "<p>$$",
                    Target::Page => "<p class=\"math\">$$",
                });
                out.push_str(&escape(tex, false));
                out.push_str("$$</p>");
            }
            Block::Rule => out.push_str("<hr>"),
        }
    }
}

fn inlines_html(inlines: &[Inline], out: &mut String, target: Target) {
    for inline in inlines {
        match inline {
            Inline::Text(text) => out.push_str(&escape(text, false)),
            Inline::Styled { style, content } => {
                let tag = match style {
                    Style::Bold => "strong",
                    Style::Italic => "em",
                    Style::Strikethrough => "del",
                    Style::Underline => "u",
                    Style::Superscript => "sup",
                    Style::Subscript => "sub",
                    Style::Spoiler if target == Target::Page => {
                        out.push_str("<span class=\"spoiler\">");
                        inlines_html(content, out, target);
                        out.push_str("</span>");
                        continue;
                    }
                    // Copied, it is no secret.
                    Style::Spoiler => {
                        inlines_html(content, out, target);
                        continue;
                    }
                };
                out.push_str(&format!("<{tag}>"));
                inlines_html(content, out, target);
                out.push_str(&format!("</{tag}>"));
            }
            Inline::Code(code) => {
                out.push_str("<code>");
                out.push_str(&escape(code, false));
                out.push_str("</code>");
            }
            Inline::Link { content, url } if target == Target::Page && !is_safe_link(url) => {
                inlines_html(content, out, target)
            }
            Inline::Link { url, content } => {
                out.push_str(&format!("<a href=\"{}\">", escape(url, true)));
                inlines_html(content, out, target);
                out.push_str("</a>");
            }
            Inline::Math(tex) => {
                if target == Target::Page {
                    out.push_str("<span class=\"math\">");
                }
                out.push('$');
                out.push_str(&escape(tex, false));
                out.push('$');
                if target == Target::Page {
                    out.push_str("</span>");
                }
            }
            Inline::Mention { user_id, name } => out.push_str(&format!(
                "<a href=\"https://matrix.to/#/{}\">{}</a>",
                escape(user_id, true),
                escape(name, false)
            )),
            Inline::LineBreak => out.push_str("<br>"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pages_keep_safe_links_only_and_mark_math() {
        let document = Document::from_html(
            "<p><a href=\"magnet:?xt=urn:x\">get</a> <a href=\"ftp://x.org/f\">old</a> \
             <a href=\"https://x.org/?a=1&amp;b=&quot;\">web</a> \
             <a href=\"mailto:a@b.org\">mail</a> \
             <span data-mx-maths=\"x&lt;1\"><code>x&lt;1</code></span> \
             &lt;script&gt;</p>",
        );
        let html = document.to_page_html();
        assert!(
            !html.contains("magnet:") && !html.contains("ftp:"),
            "{html}"
        );
        assert!(html.contains("get") && html.contains("old"), "{html}");
        assert!(
            html.contains("<a href=\"https://x.org/?a=1&amp;b=&quot;\">web</a>"),
            "{html}"
        );
        assert!(
            html.contains("<a href=\"mailto:a@b.org\">mail</a>"),
            "{html}"
        );
        assert!(
            html.contains("<span class=\"math\">$x&lt;1$</span>"),
            "{html}"
        );
        assert!(html.contains("&lt;script&gt;"), "{html}");

        let link = |url: &str, text: &str| Inline::Link {
            url: url.into(),
            content: vec![Inline::Text(text.into())],
        };
        let document = Document {
            blocks: vec![Block::Paragraph(vec![
                link("javascript:alert(1)", "run"),
                link("magnet:?xt=urn:x", "get"),
                Inline::Styled {
                    style: Style::Spoiler,
                    content: vec![Inline::Text("secret".into())],
                },
            ])],
        };
        let html = document.to_page_html();
        assert_eq!(html, "<p>runget<span class=\"spoiler\">secret</span></p>");
        // The clipboard's keeps every link.
        assert!(document.to_html().contains("javascript:alert(1)"));
    }

    #[test]
    fn only_web_and_mail_links_are_safe() {
        for url in ["https://x.org", "HTTP://x.org", "mailto:a@b.org"] {
            assert!(is_safe_link(url), "{url}");
        }
        for url in [
            "javascript:alert(1)",
            "data:text/html,x",
            "x.html",
            "magnet:?x",
        ] {
            assert!(!is_safe_link(url), "{url}");
        }
    }
}
