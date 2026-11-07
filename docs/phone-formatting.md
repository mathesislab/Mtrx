# Formatting and math on the phones

Goal: the phones show what the desktop shows: Markdown / Matrix HTML formatting (bold, italic, code, quotes, lists, headings, links, mentions) and math, in the system font at the system text size. Today they show plain text.

Rules that apply:
- Core sends finished state. The apps don't parse HTML, Markdown or LaTeX, and don't look for links.
- One model for both phones. Records, not functions, so Android JVM tests can build them.
- Work off the main thread, cached, and only for rows on screen.
- iOS behaviour is canonical where the platforms disagree.
- No new dependencies (none is needed below; Q2 and Q8 are about size).

Agreed for the phones in #57: own bubbles #C7CFF3 with black text (dark mode: #2E3352 with white), so math looks the same in both bubbles. System text size (Dynamic Type / font scale).

Paths: `core/` = `crates/mtrx-core/src/`, `fmt/` = `crates/mtrx-format/src/`, `ffi/` = `crates/mtrx-ffi/src/`, `app/` = `crates/mtrx-app/src/`, `ios/` = `apps/ios/Mtrx/`, `android/` = `apps/android/app/src/main/kotlin/org/mtrx/app/`.

---

## Today

**Format (`fmt/`, shared)**
- `document.rs`: `Document { blocks }`, a tree.
  - `Block`: `Paragraph`, `Heading{level}`, `Quote(Vec<Block>)`, `List{start, items}`, `CodeBlock{language, code}`, `Math(tex)` (display), `Rule`.
  - `Inline`: `Text`, `Styled{style, content}`, `Code`, `Link{url, content}`, `Math(tex)` (inline), `Mention{user_id, name}`, `LineBreak`.
  - `Style`: bold, italic, strikethrough, underline, superscript, subscript, spoiler.
  - `from_html` reads `org.matrix.custom.html` (`data-mx-maths` for math, `matrix.to` links as mentions, drops `<mx-reply>`). `from_plain` splits paragraphs.
  - Both linkify bare URLs (`linkify`, `:107-117`). It needs a scheme, so `www.example.org` isn't a link.
  - `to_plain_text` gives Markdown-like text with `$…$`.
- `render.rs` (feature `render`, desktop only): `Renderer::render(&Document, &RenderStyle) -> Image`.
  - The whole message becomes one Typst program. User text enters as string literals. LaTeX goes through `mitex` (vendored patch) and the `RENAMED_SYMBOLS` table.
  - `typst_render` makes straight RGBA, trimmed rows. `pieces` gives each text leaf's rectangles, for selection and links.
  - Fonts: all of `typst-assets` (9.6 MB: NewCM text and math, Libertinus, DejaVu Sans Mono), the app's text fonts (Inter), and the system emoji font (memory-mapped, desktop paths only).
  - `comemo`'s memo cache is never evicted (`typst::comemo::evict` is never called).

**Core**
- `core/timeline.rs:251-264` `Message{kind, body, document, …}`. `document` is built once per SDK item (`:1268-1271`) from the formatted body, else the plain body.

**Desktop**
- `app/room/view.rs:631-690`: a text, notice or emote message
  - without math is drawn natively by `app/message.rs`: spans with bold/italic/underline/strike, monospace code on a tinted background, links underlined in the primary colour, mentions in the primary colour (bold if us), quote bars, "•" / "n." markers in a 20 px column, code blocks in a box, a heading bold at 16 px (body 15). Superscript, subscript and spoiler show as plain text.
  - with math is one Typst image of the whole message (`app/math.rs`). It's rendered in `spawn_blocking`, keyed by (document hash, width in 40 px steps, scale, text colour), and cached in memory with no limit. Until it's ready, or if it fails, the native view shows the LaTeX as `$…$` code.
- `app/app/window.rs:248-259` `math_params`: room width, pixel scale, theme text, link and muted colours.
- Inter is bundled (`app/fonts.rs`, 2 MB) so typeset and native text match.

**FFI**
- `ffi/types.rs:445-462` `Message`: `body` (`document.to_plain_text()`), `raw_body`, `kind`, `edited`, `media`, `reactions`, `reply_to`. No structure.
- The whole timeline is re-sent on every change (`ffi/timeline.rs:61`).

**iOS**
- `ios/Views/TimelineRowView.swift:228-238` `text(_:)`: one `Text` of `body`, links found by `NSDataDetector` (`ios/Views/Formatting.swift:46-59`), `www.` included. Emote: italic "Name " before it. " (edited)" in footnote.
- Links open through SwiftUI's `openURL`. Mentions are plain names. LaTeX shows raw.
- A caption shows only if `body != media.filename` (`:203`).

**Android**
- `android/timeline/EventRows.kt:384-438` `MessageBody`: an `AnnotatedString` of `body`.
  - `findLinks` (`timeline/Links.kt`, own regex, `www.` → `https://`) is underlined in `BubbleColors.link`.
  - `LinkedText` (`:446-489`) hit-tests taps itself and adds screen-reader actions.
  - Emote: "* Name body", italic. Notice: secondary colour.
- `timeline/TimelineRows.kt:84-87` `messageText` does the same caption check as iOS.
- `ui/Theme.kt:41-46` `BubbleColors(container, content, secondary, link)`.

**Can Typst run on the phones?**
- Yes. It's pure Rust (tiny-skia, rustybuzz, wasmi, syntect with fancy-regex). The only assembly is `psm` (via `stacker`), which supports aarch64 and x86_64 on iOS and Android. The core already depends on `mtrx-format` (without `render`).
- Size is the cost:
  - `Library::default()` registers everything, so LTO keeps wasmi (plugins), hayagriva (bibliographies), syntect, hayro and ICU data. typst 0.15 has no Cargo features to leave them out.
  - Estimate: +15–25 MB of code per binary, plus fonts (Q2). Today: iOS `Mtrx` binary 76 MB (local Release build, 31 Oct). Desktop 178 MB with Typst, iced and wgpu.
  - Android ships two ABIs, so it pays twice in the APK.
  - Not measured. Commit 1 measures it on CI (Q8).
- Threads: rendering is synchronous, so it runs in `spawn_blocking` on the FFI runtime. Typst uses rayon's global pool (one thread per core, started lazily).
- Memory: `comemo` must be evicted after renders (phones and desktop).
- Text font: SF Pro isn't readable or licensed as a font file, so Typst can't typeset phone text in the system font. Only the math needs Typst (Q1).

---

## Design

The phones draw text natively from a flat block/run model that core finishes. Each formula is a small Typst image placed in that text: inline math on the line's baseline, display math as its own block (Q1). The image is black on transparent, and the app tints it with the text colour, so one image serves both bubbles and both modes (Q3).

### Format: `fmt/text.rs` (new), `fmt/render.rs`

```rust
/// A document flattened for native text: blocks top to bottom, each with
/// its quote and list nesting; inline content as styled runs.
#[derive(Clone, Debug, PartialEq)]
pub struct TextBlock {
    pub kind: BlockKind,
    pub quotes: u8,              // quote bars to its left
    pub lists: u8,               // list indents
    pub marker: Option<String>,  // "•" or "3." on an item's first block
}
pub enum BlockKind {
    Paragraph(Vec<Run>),
    Heading { level: u8, runs: Vec<Run> },
    Code { code: String, language: Option<String> },   // trailing newlines trimmed
    Math(Formula),
    Rule,
}
pub enum Run {
    Text { text: String, style: TextStyle, link: Option<String> },   // line breaks are "\n"
    Code { code: String, style: TextStyle, link: Option<String> },
    Math { formula: Formula, link: Option<String> },
    Mention { user_id: String, name: String, style: TextStyle },
}
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct TextStyle { pub bold: bool, pub italic: bool, pub strikethrough: bool,
                       pub underline: bool, pub superscript: bool, pub subscript: bool,
                       pub spoiler: bool }
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct Formula { pub tex: String, pub display: bool }

impl Document {
    /// Adjacent runs with the same style and link are merged.
    pub fn text_blocks(&self) -> Vec<TextBlock>;
}
```

- Linkify also finds `www.` addresses (`url_must_have_scheme(false)`, prefixing `https://`), as iOS does today. The desktop gets this too.

```rust
// render.rs, beside Renderer::render
pub struct FormulaImage {
    pub width: u32, pub height: u32,
    pub alpha: Vec<u8>,    // one byte per pixel: black ink on transparent
    pub baseline: u32,     // pixels from the top to the baseline it sits on
}
impl Renderer {
    /// Math fonts only: no text fonts, no system emoji (phones).
    pub fn for_formulas() -> Self;
    /// One formula at `em_pixels` device pixels per em. The page is the
    /// formula's ink (`text(top-edge: "bounds", bottom-edge: "bounds")`),
    /// and `baseline` comes from its laid-out line. Fails above 4096 px a side.
    pub fn formula(&self, formula: &Formula, em_pixels: f32) -> Result<FormulaImage, RenderError>;
}
```

- `render` and `formula` call `typst::comemo::evict(10)` after compiling, as typst-cli does. This fixes the desktop too.
- Features (Q2): `render` stops turning on `typst-assets/fonts`. A new `typst-fonts` feature turns it on, and the desktop asks for it. A new `math-font` feature embeds the vendored `fonts/NewCMMath-Regular.otf` (1.2 MB, OFL) for the phones.

### Core: `core/math.rs` (new, feature `math`)

```rust
/// Typesets formulas for the phones. One renderer, made on first use.
pub async fn typeset(formula: Formula, em_pixels: f32) -> Result<Typeset, Error>;
pub struct Typeset { pub png: Vec<u8>, pub width: u32, pub height: u32, pub baseline: u32 }
```
- `spawn_blocking` on a `LazyLock<Renderer::for_formulas()>`. PNG (grey + alpha) is encoded with the core's existing `image` crate.
- `em_pixels` is rounded to whole pixels, so cache keys stay few.
- No cache in core: the apps keep decoded images (Q4).

### FFI

```rust
// ffi/types.rs: Message gains
/// What its text shows. Empty for a file without a caption. An emote starts
/// with the sender's name, and all its runs are italic.
#[uniffi(default)] pub text: Vec<TextBlock>,

#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct TextBlock { pub kind: BlockKind, pub quotes: u8, pub lists: u8, pub marker: Option<String> }
#[derive(Clone, Debug, PartialEq, uniffi::Enum)]
pub enum BlockKind { Paragraph { runs: Vec<TextRun> }, Heading { level: u8, runs: Vec<TextRun> },
                     Code { code: String, language: Option<String> }, Math { formula: Formula }, Rule }
#[derive(Clone, Debug, PartialEq, uniffi::Enum)]
pub enum TextRun {
    Text { text: String, style: TextStyle, link: Option<String> },
    Code { code: String, style: TextStyle, link: Option<String> },
    Math { formula: Formula, link: Option<String> },
    /// Tap: show the person. `you`: drawn bold.
    Mention { user_id: String, name: String, style: TextStyle, you: bool },
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, uniffi::Record)] pub struct TextStyle { /* as fmt */ }
#[derive(Clone, Debug, PartialEq, Eq, Hash, uniffi::Record)] pub struct Formula { pub tex: String, pub display: bool }
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct FormulaImage {
    /// PNG, black on transparent: draw it tinted with the text's colour.
    pub png: Vec<u8>,
    pub width: u32, pub height: u32,   // pixels
    pub baseline: u32,                 // pixels from the top
}

// ffi/lib.rs (or ffi/math.rs)
/// Typeset `formula` at `em_pixels` device pixels per em (the body text size
/// × screen scale). `MtrxError::Other` for LaTeX it can't read: show the
/// formula's `tex` as code instead.
#[uniffi::export]
pub async fn typeset_formula(formula: Formula, em_pixels: f32) -> Result<FormulaImage, MtrxError>;
```
- It's a free function, not a `Session` method: it needs no account, and UI-test mode can use it.
- The nesting is flat on purpose: no recursive records across UniFFI, and the apps lay blocks out in one `ForEach` / `Column`.
- `you` and the emote prefix need the own user and the sender:
  - `ffi/timeline.rs` `timeline_rows(rows, own)`, with `own` from `RoomTimeline` (core has `room.own_user_id()`, `core/timeline.rs:607`);
  - `TimelineRow::new(row, own)` replaces `From<&core::Row>`;
  - `Message::new(message, sender, own)`.
- `body` stays for Copy, Select text, the accessibility fallback and edits.
- The caption rule moves into core: `text` is empty when a file's body is its file name.
- mtrx-ffi turns on `mtrx-core/math`, which turns on `mtrx-format/{render,math-font}`. These are workspace crates, so nothing new goes in `Cargo.lock`. The CLI and the desktop's core don't get Typst through core.

### iOS

- New `ios/Views/MessageText.swift` `MessageTextView(blocks, own, kind, edited)`, replacing `text(_:)` in `TimelineRowView.swift:228-238`.
  - A `VStack(alignment: .leading, spacing: 6)` of blocks, indented 16 pt per list level.
  - `marker` in a 20 pt column. Each quote level adds a 3 pt bar in the bubble's secondary colour.
  - A paragraph is one `Text` built from runs: `AttributedString` with `.font` (bold / italic), `.strikethroughStyle`, `.underlineStyle`, and `.baselineOffset` with a smaller font for superscript and subscript.
  - Code: `.monospaced()` with `.backgroundColor` at 15 % of the text colour.
  - Links: `.link`. A mention's link is a `mtrx-person:<user id>` URL, which the row's `OpenURLAction` turns into "show person". Other links open as now. Mentions are in the link colour, bold if `you`.
  - A heading is the same `Text` in `.title3.bold()`, for every level (desktop: one size).
  - A code block is `.callout.monospaced()` in a rounded tinted box. A rule is a `Divider`.
- Math:
  - Inline: `Text(Image(uiImage, scale: displayScale, label: Text(tex)).renderingMode(.template)).baselineOffset(baseline − height)`, in points. The surrounding `foregroundStyle` / link tint colours it.
  - Display: a centred `Image` in the same tint (Q5 for wide ones).
  - Before an image is there, or if it failed, the run shows `$tex$` in monospace, as on the desktop.
- Loading: `ImageCache` gains `.formula(Formula, emPixels)` and keeps `(UIImage, baseline)` in its `NSCache` (cost = bytes).
  - The message view's `.task(id: emPixels)` loads its formulas, so only rows on screen ask, and the task is cancelled when a row goes.
  - `emPixels` = `UIFontMetrics(forTextStyle: .body).scaledValue(for: 17) × displayScale`, read from `@Environment(\.dynamicTypeSize)` and `\.displayScale`. A text size change gives new keys.
- " (edited)" goes after the last paragraph's runs, or on its own line if the message ends in a code block, math or a rule (the desktop rule).
- Deleted: `Formatting.linked` and its `NSDataDetector`, the emote branch, and the `body != filename` check.

### Android

- New `android/timeline/MessageText.kt` `MessageText(blocks, colors, onOpenLink, onShowPerson, onHold)`, replacing the `LinkedText(text(...))` calls in `EventRows.kt:416-421` and `LinkedText` itself (`:446-489`).
  - A `Column` of blocks, with the same indents, markers and quote bars as iOS.
  - A paragraph is a `buildAnnotatedString` of runs: `SpanStyle` for bold, italic, strike, underline; `baselineShift` + `0.75.em` for superscript and subscript; `FontFamily.Monospace` + background for code. Links and mentions are `LinkAnnotation.Clickable` (Compose UI 1.8 from BOM 2025.05.01): taps and screen-reader actions come built in, replacing `LinkedText`'s own hit test (`linkAt`); the hold stays on the bubble.
- Inline math: `appendInlineContent(id, alternateText = tex)` with `InlineTextContent(Placeholder(w.sp, h.sp, TextCenter))`.
  - Compose can't place a placeholder on an arbitrary baseline. The image is drawn in a placeholder padded above or below, so that its baseline lands on the text's baseline. The padding comes from `TextStyle`'s font metrics (`Paint.fontMetrics`).
  - It's tinted with `ColorFilter.tint(content or link colour)`.
- Display math: an `Image`, tinted. Fallback `$tex$` in monospace, as iOS.
- Loading: `Images` gains `formula(formula, emPx)` / `cachedFormula`, with bitmap + baseline in its `LruCache`. `produceState` in the row, so only composed (visible) rows load.
  - `emPx` = `with(LocalDensity) { MaterialTheme.typography.bodyLarge.fontSize.toPx() }`, so the font scale is included.
- Deleted: `timeline/Links.kt` and `LinksTest` (the cases move to `mtrx-format` tests), the `text(body)` builder, the emote branch, and `messageText`'s caption check.

### Desktop

- Unchanged, except `comemo::evict` and the `typst-fonts` feature in its manifest. Its native view keeps walking the `Document` tree, since selection units and leaves depend on it. Math messages stay one image (Q6).

---

## Tests

- `mtrx-format`:
  - `text_blocks`: nested quote in a list item (`quotes`, `lists`, marker only on the first block), ordered lists from `start`, styled link merging, a mention inside bold, inline and display math, `<br>` as "\n".
  - Linkify: `www.` and trailing punctuation (the cases from Android's `LinksTest`).
  - `formula`: the image grows with `em_pixels`; `baseline` is within the height and below the middle for `x`; a fraction is taller than `x`; bad LaTeX gives `Err`; over 4096 px gives `Err`; the alpha is non-empty.
- Core: `typeset` returns a PNG that decodes to the stated size; two concurrent calls both succeed.
- FFI (`types.rs` tests): `text` for a formatted message; `you` set only for our ID; an emote has the name run first and all runs italic; a file without a caption has empty `text`, with a caption not.
- iOS (`MtrxTests`): the `AttributedString` from runs (links, the mention URL, the fallback `$tex$`). The UI-testing fake gets one formatted message and one with inline and display math, and a UI test checks they show (accessibility labels).
- Android JVM: the `AnnotatedString` built from `TextBlock` records (links, mentions, the inline-content ids). A screenshot test in `screenshots/Screens.kt` with a formatted message (math as the fallback, since JVM tests can't call FFI).

## PR split

One PR, three commits, each building on CI:
1. Core + FFI: `fmt/text.rs`, linkify `www.`, `Renderer::{for_formulas, formula}`, comemo eviction, the font features (desktop manifest included), `core/math.rs`, FFI records, `typeset_formula`, `Message.text`, tests. The PR description gives the iOS and Android binary sizes before and after, from the CI artefacts (Q8).
2. iOS: `MessageText.swift`, the `ImageCache` formula key, the deletions.
3. Android: `MessageText.kt`, `Images.formula`, the deletions.

Size: M. About 350 lines of format/core/FFI plus tests, 250 per phone. No desktop commit: it keeps its own model (Q6).

---

## Open questions

1. **What Typst renders on the phones.**
   - (a) Each formula, as a small image placed in native text: system font, Dynamic Type reflow, VoiceOver and native links. Images don't depend on the bubble width, and the same formula is shared across messages. An inline formula can't break across lines.
   - (b) The whole message as one image, as the desktop does for math messages. The text around the math would be in Inter (bundled, +2 MB) not SF/Roboto, rotation re-renders, and links need hit rectangles from `pieces`.
   - (c) Native LaTeX libraries (SwiftMath, AndroidMath): smaller, but two new dependencies and formulas that differ from the desktop's.
   - Recommend (a).
2. **Fonts on the phones.**
   - (a) Vendor `NewCMMath-Regular.otf` (1.2 MB, OFL) into `mtrx-format` behind `math-font`. `typst-assets`' fonts become the desktop-only `typst-fonts` feature. It saves 8.4 MB per binary.
   - (b) Keep all `typst-assets` fonts (+9.6 MB, no work).
   - (c) The apps bundle the font and pass its path at start.
   - Recommend (a). It's a font file in the repo, not a dependency.
3. **Colour of formula images.**
   - (a) Black ink on transparent, tinted by the app with the text or link colour. One image for both bubbles, light and dark, and no re-render when the mode changes.
   - (b) Rendered in the colour, with the colour in the key (as the desktop).
   - Recommend (a).
4. **Caching.**
   - (a) In the apps' existing decoded-image caches (iOS `ImageCache`, 64 MB `NSCache`; Android `Images` `LruCache`), keyed by (tex, display, em pixels). Nothing on disk.
   - (b) Also an LRU of PNGs in core.
   - (c) A disk cache across launches.
   - Recommend (a). A formula renders in milliseconds once warm, and the apps already cache previews this way.
5. **Display math or code wider than the bubble.**
   - (a) It scrolls sideways inside the bubble (code blocks don't wrap).
   - (b) Math is scaled down to fit (no smaller than 60 %, then scrolls). Code wraps, as on the desktop.
   - (c) Both wrap or scale, never scroll.
   - Recommend (b). The bubble stays one gesture (swipe-to-reply is a horizontal drag), and code wraps as on the desktop.
6. **Should the desktop switch to the shared model?**
   - (a) No. It keeps its `Document` walk and whole-message math image, which character selection across math depends on. Only `comemo` eviction and the font feature change.
   - (b) Yes, later: native text with formula images on the desktop too, one rendering model everywhere. It would rewrite `selectable/image.rs` and the `pieces` mapping.
   - Recommend (a).
7. **Superscript, subscript and spoilers** (the desktop's native view shows them as plain text; its Typst image doesn't).
   - (a) On the phones: superscript and subscript raised or lowered and smaller. A spoiler is a filled box until tapped. The desktop's native view gets the same in a later desktop PR (showing a spoiler in the clear is a bug).
   - (b) Plain text, as the desktop's native view.
   - Recommend (a).
8. **Binary size.** Typst adds an estimated 15–25 MB of code to each binary, plus fonts (Q2). Android ships two ABIs.
   - (a) Accept, measured on CI in commit 1 and stated in the PR. Stop and ask if it's over 30 MB per binary.
   - (b) Measure first in a separate throw-away CI run, before any phone work.
   - (c) Don't ship Typst on the phones. Use (1c).
   - Recommend (a).
9. **Tapping a mention.**
   - (a) It opens the person sheet (iOS `PersonInfoView`, Android `PersonInfoSheet`). Mentions of us are bold, in the link colour, as on the desktop.
   - (b) It opens the `matrix.to` link in the browser.
   - Recommend (a).

## Decisions (2026-11-07)

The user took all recommendations: 1a, 2a, 3a, 4a, 5b, 6a, 7a, 8a, 9a. The manifest changes (features in `mtrx-format` and core, `mtrx-format` as a normal dependency of `mtrx-ffi`, the vendored math font) are approved with them.
