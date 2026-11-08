//! Composer text (Markdown with `$…$` math) to a Matrix message body.

use std::ops::Range;

use pulldown_cmark::{
    CowStr, Event, LinkType, Options, Parser, Tag, TagEnd, TextMergeStream, html,
};

/// The two bodies of a Matrix text message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Formatted {
    /// Plain-text fallback: the text as typed.
    pub body: String,
    /// `org.matrix.custom.html`; `None` when the text has no formatting.
    pub html: Option<String>,
    /// The mention targets found, as indices into those given, each once,
    /// in order of first appearance.
    pub mentions: Vec<usize>,
}

/// Someone, or everyone, a message may mention as `@` + [`Self::text`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MentionTarget {
    /// What follows the `@`: a display name, a user ID without its `@`, or
    /// `room`.
    pub text: String,
    pub mention: Mention,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Mention {
    /// A user: their `matrix.to` URL, and the link text (their name).
    User { href: String, name: String },
    /// Everyone in the room; stays plain text.
    Room,
}

/// Convert composer text to Matrix bodies, like Element does: Markdown, single
/// newlines kept as line breaks, and `$…$` / `$$…$$` math sent as
/// `data-mx-maths` elements with the LaTeX as code fallback.
///
/// `@` + a target's text, at word boundaries and outside code and links,
/// is a mention; the longest target wins. Users become `matrix.to` links.
pub fn markdown_to_matrix(text: &str, targets: &[MentionTarget]) -> Formatted {
    let options = Options::ENABLE_STRIKETHROUGH | Options::ENABLE_MATH;
    let events = Parser::new_ext(text, options).map(|event| match event {
        Event::SoftBreak => Event::HardBreak,
        Event::InlineMath(tex) => Event::InlineHtml(CowStr::from(format!(
            r#"<span data-mx-maths="{0}"><code>{1}</code></span>"#,
            escape(&tex, true),
            escape(&tex, false)
        ))),
        Event::DisplayMath(tex) => Event::InlineHtml(CowStr::from(format!(
            r#"<div data-mx-maths="{0}"><code>{1}</code></div>"#,
            escape(&tex, true),
            escape(&tex, false)
        ))),
        // Raw HTML typed by the user is shown as text, not interpreted.
        Event::Html(raw) | Event::InlineHtml(raw) => Event::Text(raw),
        other => other,
    });
    let mut mentions = Mentions::new(targets);
    let events = mentions.link(TextMergeStream::new(events));
    let mut out = String::new();
    html::push_html(&mut out, events.into_iter());
    let out = out.trim_end_matches('\n');

    // A single plain paragraph needs no HTML body.
    let plain = out
        .strip_prefix("<p>")
        .and_then(|s| s.strip_suffix("</p>"))
        .is_some_and(|inner| !inner.contains('<') && unescape(inner) == text.trim());
    Formatted {
        body: text.to_owned(),
        html: (!plain).then(|| out.to_owned()),
        mentions: mentions.found,
    }
}

/// Where `text` mentions one of `targets` (what follows the `@`, see
/// [`MentionTarget::text`]): `@` + a target, not touching letters, digits
/// or `_` on either side; the longest target wins. In order, each with its
/// target's index.
pub fn find_mentions<S: AsRef<str>>(text: &str, targets: &[S]) -> Vec<(Range<usize>, usize)> {
    let mut longest_first: Vec<usize> = (0..targets.len())
        .filter(|&i| !targets[i].as_ref().is_empty())
        .collect();
    longest_first.sort_by_key(|&i| std::cmp::Reverse(targets[i].as_ref().len()));
    let mut found = Vec::new();
    let mut at = 0;
    while let Some(offset) = text[at..].find('@') {
        let sign = at + offset;
        at = sign + 1;
        if text[..sign].chars().next_back().is_some_and(is_word) {
            continue;
        }
        let after = &text[at..];
        let hit = longest_first.iter().copied().find(|&i| {
            let target = targets[i].as_ref();
            after.starts_with(target) && !after[target.len()..].chars().next().is_some_and(is_word)
        });
        if let Some(index) = hit {
            at += targets[index].as_ref().len();
            found.push((sign..at, index));
        }
    }
    found
}

/// Turns mentions in prose text into links.
struct Mentions<'t> {
    targets: &'t [MentionTarget],
    texts: Vec<&'t str>,
    found: Vec<usize>,
}

impl<'t> Mentions<'t> {
    fn new(targets: &'t [MentionTarget]) -> Self {
        Self {
            targets,
            texts: targets.iter().map(|t| t.text.as_str()).collect(),
            found: Vec::new(),
        }
    }

    /// `events` with user mentions in text turned into links. Code is a
    /// separate event; code blocks, links and image descriptions are left
    /// as they are.
    fn link<'a>(&mut self, events: impl Iterator<Item = Event<'a>>) -> Vec<Event<'a>> {
        let mut out = Vec::new();
        let mut inside = 0usize;
        for event in events {
            match &event {
                Event::Start(Tag::Link { .. } | Tag::Image { .. } | Tag::CodeBlock(_)) => {
                    inside += 1
                }
                Event::End(TagEnd::Link | TagEnd::Image | TagEnd::CodeBlock) => {
                    inside = inside.saturating_sub(1)
                }
                Event::Text(text) if inside == 0 && text.contains('@') => {
                    self.split(text, &mut out);
                    continue;
                }
                _ => {}
            }
            out.push(event);
        }
        out
    }

    fn split<'a>(&mut self, text: &str, out: &mut Vec<Event<'a>>) {
        let owned = |s: &str| CowStr::from(s.to_owned());
        // Start of the text not pushed yet.
        let mut pending = 0;
        for (range, index) in find_mentions(text, &self.texts) {
            if !self.found.contains(&index) {
                self.found.push(index);
            }
            // The room stays text.
            let Mention::User { href, name } = &self.targets[index].mention else {
                continue;
            };
            if pending < range.start {
                out.push(Event::Text(owned(&text[pending..range.start])));
            }
            out.push(Event::Start(Tag::Link {
                link_type: LinkType::Inline,
                dest_url: owned(href),
                title: CowStr::Borrowed(""),
                id: CowStr::Borrowed(""),
            }));
            out.push(Event::Text(owned(name)));
            out.push(Event::End(TagEnd::Link));
            pending = range.end;
        }
        if pending < text.len() {
            out.push(Event::Text(owned(&text[pending..])));
        }
    }
}

/// Letters, digits and `_`: a mention may not touch them.
fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// `text` safe in HTML: `&`, `<` and `>` escaped, and `"` too in an
/// `attribute` (in double quotes).
pub fn escape(text: &str, attribute: bool) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' if attribute => out.push_str("&quot;"),
            c => out.push(c),
        }
    }
    out
}

fn unescape(text: &str) -> String {
    text.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&amp;", "&")
}
