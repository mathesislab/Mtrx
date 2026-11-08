//! HTML: a page per 2000 events of each room, an index and a style sheet.
//! The pages run no script: a content security policy forbids any, and
//! anything from elsewhere. Every text is escaped, and messages are written
//! from our document model, not the sender's HTML.

use std::{
    collections::{HashMap, VecDeque},
    fs::File,
    io::{self, BufWriter, Write},
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

use matrix_sdk::ruma::{EventId, OwnedEventId, OwnedUserId};
use mtrx_format::escape;

use super::{
    Line,
    event::Style,
    write::{
        self, Attached, Index, Quote, Rendered, RoomEnd, RoomHead, RoomProgress, Shown, Writer,
    },
};
use crate::{MediaKind, human_size};

/// Events on a page.
pub(super) const PAGE: usize = 2000;
/// Events whose page is remembered, for links to the messages replies
/// answer.
const REMEMBERED: usize = 2000;
/// A message this soon after the previous one of its sender goes under it,
/// without the name again.
const GROUP: Duration = Duration::from_secs(5 * 60);

/// In every page's head: nothing runs, nothing is fetched from elsewhere.
pub(super) const POLICY: &str = "default-src 'none'; img-src 'self' file:; \
    media-src 'self' file:; style-src 'self' file:; base-uri 'none'; form-action 'none'";

pub(super) struct HtmlWriter {
    root: PathBuf,
    room: Option<RoomPages>,
}

/// The room being written.
struct RoomPages {
    head: RoomHead,
    folder: PathBuf,
    out: BufWriter<File>,
    /// From 1.
    page: usize,
    on_page: usize,
    /// The day of the last event on the page.
    day: Option<String>,
    /// The sender of the last message on the page, and when.
    last: Option<(OwnedUserId, SystemTime)>,
    /// The pages of the latest events, for links to them.
    pages: HashMap<OwnedEventId, usize>,
    order: VecDeque<OwnedEventId>,
    /// Events written in the room.
    events: u64,
}

impl HtmlWriter {
    /// Writes into `root`, with its style sheet.
    pub fn new(root: &Path) -> io::Result<Self> {
        std::fs::write(root.join("style.css"), STYLE)?;
        Ok(Self {
            root: root.to_owned(),
            room: None,
        })
    }
}

impl Writer for HtmlWriter {
    fn begin_room(&mut self, room: &RoomHead) -> io::Result<()> {
        let folder = self.root.join(&room.folder);
        std::fs::create_dir_all(&folder)?;
        let mut pages = RoomPages {
            out: write::create(&folder.join(page_file(1)))?,
            folder,
            head: RoomHead {
                id: room.id.clone(),
                name: room.name.clone(),
                topic: room.topic.clone(),
                folder: room.folder.clone(),
                about: room.about.clone(),
            },
            page: 1,
            on_page: 0,
            day: None,
            last: None,
            pages: HashMap::new(),
            order: VecDeque::new(),
            events: 0,
        };
        pages.open_page()?;
        self.room = Some(pages);
        Ok(())
    }

    fn event(&mut self, event: &Rendered) -> io::Result<()> {
        let Some(room) = &mut self.room else {
            return Ok(());
        };
        if matches!(event.shown, Shown::Hidden) {
            return Ok(());
        }
        if room.on_page == PAGE {
            room.next_page()?;
        }
        room.event(event)
    }

    fn end_room(&mut self, end: &RoomEnd) -> io::Result<()> {
        let Some(mut room) = self.room.take() else {
            return Ok(());
        };
        let note = match end {
            RoomEnd::Complete if room.events == 0 => {
                "<p class=\"note\">No messages in this range.</p>".to_owned()
            }
            RoomEnd::Complete => String::new(),
            RoomEnd::Stopped => "<p class=\"note stopped\">Export stopped here.</p>".to_owned(),
            RoomEnd::Failed(why) => format!(
                "<p class=\"note stopped\">Not all of this room could be exported: {}</p>",
                escape(why, false)
            ),
        };
        room.out.write_all(note.as_bytes())?;
        room.close_page(false)
    }

    fn index(&mut self, index: &Index) -> io::Result<()> {
        let about = index.about;
        let mut html = head(&about.title, "style.css");
        html.push_str("<header><h1>");
        html.push_str(&escape(&about.title, false));
        html.push_str("</h1>");
        if let Some(topic) = index.topic.filter(|t| !t.trim().is_empty()) {
            html.push_str(&format!("<p class=\"topic\">{}</p>", text_lines(topic)));
        }
        html.push_str(&format!(
            "<p class=\"about\">{}<br>{}</p><p class=\"state\">{}</p></header>\n<main><ul class=\"tree\">\n",
            escape(&about.exported(), false),
            escape(&about.options(), false),
            escape(&index.state.describe(index.summary, index.space), false),
        ));
        for line in index.tree {
            match line {
                Line::Space { depth, name } => html.push_str(&format!(
                    "<li class=\"space d{}\">{}</li>\n",
                    depth.min(&9),
                    escape(name, false)
                )),
                Line::Room { depth, room } => {
                    let Some(room) = index.rooms.get(*room) else {
                        continue;
                    };
                    let name = escape(&room.name, false);
                    let linked = !matches!(
                        room.progress,
                        RoomProgress::NotJoined | RoomProgress::Waiting
                    );
                    let name = match &room.folder {
                        Some(folder) if linked => {
                            format!("<a href=\"{}/{}\">{name}</a>", href(folder), page_file(1))
                        }
                        _ => name,
                    };
                    html.push_str(&format!(
                        "<li class=\"room d{}\">{name} <span class=\"count\">{}</span></li>\n",
                        depth.min(&9),
                        escape(&room.status(), false)
                    ));
                }
            }
        }
        html.push_str("</ul></main>\n</body>\n</html>\n");
        write::replace(&self.root.join("index.html"), html.as_bytes())
    }
}

impl RoomPages {
    /// The page's head, with a link to the one before.
    fn open_page(&mut self) -> io::Result<()> {
        let head = &self.head;
        let title = match self.page {
            1 => head.name.clone(),
            page => format!("{} – page {page}", head.name),
        };
        let mut html = self::head(&title, "../style.css");
        html.push_str(&format!(
            "<header><p class=\"up\"><a href=\"../index.html\">{}</a></p><h1>{}</h1>",
            escape(&head.about.title, false),
            escape(&head.name, false)
        ));
        if self.page == 1 {
            if let Some(topic) = head.topic.as_deref().filter(|t| !t.trim().is_empty()) {
                html.push_str(&format!("<p class=\"topic\">{}</p>", text_lines(topic)));
            }
            html.push_str(&format!(
                "<p class=\"about\">{}<br>{}</p>",
                escape(&head.about.exported(), false),
                escape(&head.about.options(), false)
            ));
        }
        if self.page > 1 {
            html.push_str(&format!(
                "<nav class=\"pages\"><a href=\"{}\">← Earlier</a> · Page {}</nav>",
                page_file(self.page - 1),
                self.page
            ));
        }
        html.push_str("</header>\n<main>\n");
        self.out.write_all(html.as_bytes())
    }

    /// The page's end, with a link to the next if there is one.
    fn close_page(&mut self, more: bool) -> io::Result<()> {
        let mut html = String::from("</main>\n");
        if more || self.page > 1 {
            html.push_str("<footer><nav class=\"pages\">");
            if self.page > 1 {
                html.push_str(&format!(
                    "<a href=\"{}\">← Earlier</a> · ",
                    page_file(self.page - 1)
                ));
            }
            html.push_str(&format!("Page {}", self.page));
            if more {
                html.push_str(&format!(
                    " · <a href=\"{}\">Later →</a>",
                    page_file(self.page + 1)
                ));
            }
            html.push_str("</nav></footer>\n");
        }
        html.push_str("</body>\n</html>\n");
        self.out.write_all(html.as_bytes())?;
        self.out.flush()
    }

    fn next_page(&mut self) -> io::Result<()> {
        self.close_page(true)?;
        self.page += 1;
        self.on_page = 0;
        self.day = None;
        self.last = None;
        self.out = write::create(&self.folder.join(page_file(self.page)))?;
        self.open_page()
    }

    fn event(&mut self, event: &Rendered) -> io::Result<()> {
        self.on_page += 1;
        self.events += 1;
        let mut html = String::new();
        let day = write::long_day(event.time);
        if self.day.as_ref() != Some(&day) {
            html.push_str(&format!("<h2 class=\"day\">{}</h2>\n", escape(&day, false)));
            self.day = Some(day);
            self.last = None;
        }
        let anchor = event.id.as_deref().map(anchor);
        let id_attribute = anchor
            .as_ref()
            .map(|a| format!(" id=\"{a}\""))
            .unwrap_or_default();
        let time = format!(
            "<time datetime=\"{}\">{}</time>",
            write::iso(event.time),
            write::clock(event.time)
        );
        let name = escape(&event.sender_name, false);
        match &event.shown {
            Shown::Change(change) => {
                self.last = None;
                html.push_str(&format!(
                    "<p class=\"change\"{id_attribute}>{time} {name} {}</p>\n",
                    escape(change, false)
                ));
            }
            shown => {
                let grouped = self.last.as_ref().is_some_and(|(sender, at)| {
                    *sender == event.sender
                        && event.time.duration_since(*at).is_ok_and(|d| d < GROUP)
                });
                self.last = Some((event.sender.clone(), event.time));
                html.push_str(&format!(
                    "<article class=\"m{}\"{id_attribute}>",
                    if grouped { " cont" } else { "" }
                ));
                if grouped {
                    html.push_str(&format!(
                        "<div class=\"gap\">{time}</div><div class=\"body\">"
                    ));
                } else {
                    html.push_str(&format!(
                        "<div class=\"avatar c{}\" aria-hidden=\"true\">{}</div><div class=\"body\">\
                         <p class=\"meta\"><span class=\"sender\" title=\"{}\">{name}</span> {time}</p>",
                        colour(event.sender.as_str()),
                        escape(&initials(&event.sender_name), false),
                        escape(event.sender.as_str(), true),
                    ));
                }
                if let Some(quote) = &event.reply {
                    html.push_str(&self.quote(quote));
                }
                if let Some(root) = &event.thread {
                    html.push_str(&format!(
                        "<p class=\"thread\">In a thread{}</p>",
                        match self.link(root) {
                            Some(link) => format!(" (<a href=\"{link}\">its start</a>)"),
                            None => String::new(),
                        }
                    ));
                }
                html.push_str(&content(shown, &name));
                if !event.reactions.is_empty() {
                    let reactions: Vec<String> = (event.reactions.iter())
                        .map(|(key, n)| format!("{} {n}", escape(key, false)))
                        .collect();
                    html.push_str(&format!(
                        "<p class=\"reactions\">{}</p>",
                        reactions.join(" · ")
                    ));
                }
                html.push_str("</div></article>\n");
            }
        }
        self.out.write_all(html.as_bytes())?;
        if let Some(id) = &event.id {
            self.remember(id.clone());
        }
        Ok(())
    }

    /// The message a reply answers, linked if its page is known.
    fn quote(&self, quote: &Quote) -> String {
        let link = self.link(&quote.id);
        let open = |text: String| match &link {
            Some(link) => format!("<a href=\"{link}\">{text}</a>"),
            None => text,
        };
        match &quote.original {
            Some((sender, preview)) => format!(
                "<blockquote class=\"reply\">{} {}</blockquote>",
                open(escape(sender, false)),
                escape(preview, false)
            ),
            None => format!(
                "<blockquote class=\"reply\">In reply to {}</blockquote>",
                open("a message from earlier".into())
            ),
        }
    }

    /// A link to event `id`, if its page is known.
    fn link(&self, id: &EventId) -> Option<String> {
        let page = *self.pages.get(id)?;
        Some(if page == self.page {
            format!("#{}", anchor(id))
        } else {
            format!("{}#{}", page_file(page), anchor(id))
        })
    }

    fn remember(&mut self, id: OwnedEventId) {
        if self.order.len() == REMEMBERED
            && let Some(oldest) = self.order.pop_front()
        {
            self.pages.remove(&oldest);
        }
        self.pages.insert(id.clone(), self.page);
        self.order.push_back(id);
    }
}

/// What a message shows, after its name and time.
fn content(shown: &Shown, name: &str) -> String {
    match shown {
        Shown::Message {
            style,
            document,
            edited,
            file,
        } => {
            let mut html = String::new();
            let body = document.to_page_html();
            let mark = if *edited {
                " <span class=\"edited\">(edited)</span>"
            } else {
                ""
            };
            if !body.is_empty() {
                let (class, prefix) = match style {
                    Style::Text => ("text", String::new()),
                    Style::Notice => ("text notice", String::new()),
                    Style::Emote => ("text emote", format!("<p class=\"emote\">* {name}</p>")),
                };
                html.push_str(&format!(
                    "<div class=\"{class}\">{prefix}{body}{mark}</div>"
                ));
            } else if *edited {
                html.push_str(&format!("<p>{mark}</p>"));
            }
            if let Some(file) = file {
                html.push_str(&attached(file));
            }
            html
        }
        Shown::Undecryptable(why) => format!(
            "<p class=\"problem\">Unable to decrypt this message{}</p>",
            why.as_ref()
                .map(|why| format!(": {}", escape(why, false)))
                .unwrap_or_default()
        ),
        Shown::Deleted => "<p class=\"deleted\">Message deleted</p>".into(),
        Shown::Other(what) => format!("<p class=\"other\">{name} {}</p>", escape(what, false)),
        Shown::Change(_) | Shown::Hidden => String::new(),
    }
}

/// A message's file: shown if it's an image, video or audio, linked
/// anyway; or why it was left out.
fn attached(file: &Attached) -> String {
    let name = escape(&file.name, false);
    let size = file
        .size
        .map(|size| format!(" ({})", human_size(size)))
        .unwrap_or_default();
    match &file.saved {
        Ok(path) => {
            let link = href(path);
            let shown = match file.kind {
                MediaKind::Image => format!(
                    "<a href=\"{link}\"><img src=\"{link}\" alt=\"{}\" loading=\"lazy\"></a>",
                    escape(&file.name, true)
                ),
                MediaKind::Video => {
                    format!("<video controls preload=\"metadata\" src=\"{link}\"></video>")
                }
                MediaKind::Audio => {
                    format!("<audio controls preload=\"none\" src=\"{link}\"></audio>")
                }
                MediaKind::File => String::new(),
            };
            format!(
                "<figure class=\"file\">{shown}<figcaption><a href=\"{link}\">{name}</a>{size}</figcaption></figure>"
            )
        }
        Err(why) => format!(
            "<p class=\"file left-out\">{name}{size}: {}</p>",
            escape(why, false)
        ),
    }
}

/// A page's head, linking `style`.
fn head(title: &str, style: &str) -> String {
    format!(
        "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n\
         <meta http-equiv=\"Content-Security-Policy\" content=\"{POLICY}\">\n\
         <meta name=\"referrer\" content=\"no-referrer\">\n\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
         <title>{}</title>\n<link rel=\"stylesheet\" href=\"{style}\">\n</head>\n<body>\n",
        escape(title, false)
    )
}

/// "messages.html", "messages-2.html", …
pub(super) fn page_file(page: usize) -> String {
    match page {
        1 => "messages.html".into(),
        page => format!("messages-{page}.html"),
    }
}

/// An event's anchor: "e" and its ID with only letters, digits, `-` and
/// `_`.
fn anchor(id: &EventId) -> String {
    let safe: String = id
        .as_str()
        .chars()
        .map(|c| match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' => c,
            _ => '_',
        })
        .collect();
    format!("e{safe}")
}

/// A relative path as a link: percent-encoded but for `/`.
pub(super) fn href(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    for byte in path.bytes() {
        match byte {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' => {
                out.push(byte as char)
            }
            byte => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// Text with its line breaks, escaped.
fn text_lines(text: &str) -> String {
    (text.lines())
        .map(|line| escape(line, false))
        .collect::<Vec<_>>()
        .join("<br>")
}

/// Up to two letters of a name, for its avatar: "AR" for "Alex Robinson".
fn initials(name: &str) -> String {
    let words = name.trim_start_matches('@').split_whitespace();
    let letters: String = words
        .filter_map(|word| word.chars().find(|c| c.is_alphanumeric()))
        .take(2)
        .flat_map(char::to_uppercase)
        .collect();
    if letters.is_empty() {
        "?".into()
    } else {
        letters
    }
}

/// One of 8 avatar colours, the same for the same user.
fn colour(user: &str) -> u32 {
    // FNV-1a: stable across runs and platforms.
    let hash = user.bytes().fold(0x811c_9dc5_u32, |hash, byte| {
        (hash ^ u32::from(byte)).wrapping_mul(0x0100_0193)
    });
    hash % 8
}

const STYLE: &str = r#":root {
  color-scheme: light dark;
  --text: #1d1d1f; --muted: #6e6e73; --line: #e5e5ea; --back: #ffffff;
  --quote: #f2f2f7; --link: #3554d1; --problem: #c22b5b;
}
@media (prefers-color-scheme: dark) {
  :root {
    --text: #f2f2f7; --muted: #a1a1a6; --line: #38383a; --back: #1c1c1e;
    --quote: #2c2c2e; --link: #8fa4ff; --problem: #ff7aa2;
  }
}
* { box-sizing: border-box; }
body {
  margin: 0 auto; max-width: 860px; padding: 24px 16px 48px;
  font: 15px/1.45 -apple-system, BlinkMacSystemFont, "Segoe UI", Inter, Roboto, sans-serif;
  color: var(--text); background: var(--back);
}
a { color: var(--link); }
h1 { font-size: 24px; margin: 4px 0 8px; }
header { border-bottom: 1px solid var(--line); padding-bottom: 12px; margin-bottom: 8px; }
.up { margin: 0; font-size: 13px; }
.topic { margin: 4px 0; }
.about, .state, .count, .meta time, .gap, .change, .edited, .note, .thread, figcaption {
  color: var(--muted);
}
.about { font-size: 13px; }
.pages { margin: 12px 0; font-size: 13px; color: var(--muted); }
.day { font-size: 13px; font-weight: 600; color: var(--muted); text-align: center; margin: 20px 0 8px; }
article.m { display: flex; gap: 10px; margin-top: 12px; }
article.cont { margin-top: 2px; }
.avatar, .gap { flex: 0 0 36px; }
.avatar {
  height: 36px; border-radius: 50%; color: #fff; font-weight: 600; font-size: 14px;
  display: flex; align-items: center; justify-content: center;
}
.gap { font-size: 11px; text-align: right; padding-top: 3px; visibility: hidden; }
article.cont:hover .gap { visibility: visible; }
.body { min-width: 0; flex: 1; }
.body p, .body ul, .body ol, .body pre, .body blockquote, .body h1, .body h2, .body h3 {
  margin: 2px 0;
}
.meta { margin: 0; }
.sender { font-weight: 600; }
.meta time { font-size: 12px; margin-left: 6px; }
.notice { color: var(--muted); }
.emote { font-style: italic; }
.reply {
  margin: 2px 0 4px; padding: 2px 8px; border-left: 3px solid var(--line);
  background: var(--quote); font-size: 13px; overflow: hidden; text-overflow: ellipsis;
}
.text blockquote { border-left: 3px solid var(--line); padding-left: 8px; color: var(--muted); }
pre, code { font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace; font-size: 13px; }
pre { background: var(--quote); padding: 8px; border-radius: 6px; overflow-x: auto; }
.math { font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace; font-size: 13px; }
.spoiler { background: var(--text); color: var(--text); border-radius: 3px; }
.spoiler:hover { background: transparent; }
figure.file { margin: 4px 0; }
figure.file img, figure.file video { max-width: 100%; max-height: 420px; border-radius: 8px; display: block; }
figcaption { font-size: 13px; }
.left-out, .problem, .stopped { color: var(--problem); }
.deleted, .other { color: var(--muted); font-style: italic; }
.reactions { font-size: 13px; }
.change { font-size: 13px; margin: 6px 0 6px 46px; }
.change time { margin-right: 6px; }
.note { margin-top: 24px; font-style: italic; }
.tree { list-style: none; padding: 0; }
.tree li { padding: 4px 0; }
.tree .space { font-weight: 600; margin-top: 8px; }
.count { font-size: 13px; margin-left: 6px; }
.d1 { margin-left: 20px; } .d2 { margin-left: 40px; } .d3 { margin-left: 60px; }
.d4 { margin-left: 80px; } .d5 { margin-left: 100px; } .d6 { margin-left: 120px; }
.d7 { margin-left: 140px; } .d8 { margin-left: 160px; } .d9 { margin-left: 180px; }
.c0 { background: #3554d1; } .c1 { background: #b2387e; } .c2 { background: #2f8f6b; }
.c3 { background: #c2662b; } .c4 { background: #6a4cc2; } .c5 { background: #1f86a8; }
.c6 { background: #9a7b1c; } .c7 { background: #c23a3a; }
"#;
