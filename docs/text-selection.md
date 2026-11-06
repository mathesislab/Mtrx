# Copying from messages and selecting text

Status: done. This describes how it works and what is left.

## Why it needed building

- iced only selects text in input fields (`text_input`, `text_editor`).
  The text it displays (`text`, `rich_text`) cannot be selected, and iced
  has no right-click menus.
- A message with math is drawn as one Typst image of the whole message.

## Message menu

Right-click on a message, or "⋯" in its hover toolbar, opens a menu
(`message_menu.rs`). Right-click opens it at the cursor (`context_menu.rs`,
which records where the click was and floats the menu there through the
popover's panel); "⋯" opens it below the toolbar.

| Item         | When                        | Does                                              |
|--------------|-----------------------------|---------------------------------------------------|
| Copy         | text messages               | the selection if the message is in it, else the whole message as a selection copies it (math as `$…$`) |
| Copy image   | math messages that rendered | the picture, via `arboard` (iced's clipboard holds text only) |
| Save image…  | math messages that rendered | PNG, dark text on white, 3× scale                  |
| Copy image   | pictures                    | the picture, decoded, as for math                  |
| Save image…  | pictures                    | the file as sent, under its own name              |
| Save…        | other files                 | the file as sent, under its own name              |

A picture shown large (the viewer) has Copy image and Save image… on
right-click too.

Right-clicking a link opens the link's menu instead, at the cursor: Open
and Copy link. Hovering a link shows its address in the window's bottom
left corner (`link_status.rs`).

Separate Copy text, Copy LaTeX and Copy link items were dropped: copying
gives the same text, formulas included as `$…$`.
The "Select text…" stopgap (the text in a read-only editor) was removed
once the timeline itself became selectable.

## Selection in the timeline

Besides messages' text, sender names and times, reply quotes, day
dividers, state lines, the start of the room and the lines in place of a
message (deleted, undecryptable, "sent a …") are selectable, in reading
order. "Loading…" is not. A click (press and release without moving) on a
sender's name or time still opens their card, and on a quote goes to the
original; a drag selects.

⌘A selects the text on screen (partly shown rows too), so it never
loads or copies the whole history. Drag across words, lines and messages. Double-click selects a word (or a
formula), triple-click a paragraph, and Shift-click extends, as do
Shift+← and Shift+→ (with ⌥ by word) and Shift+↑ and Shift+↓ (by line,
as laid out; by paragraph in typeset messages), across messages. ⌘C copies,
and Esc or a click beside the text clears. Dragging past the top or bottom
of the timeline scrolls it, and scrolling while dragging extends the
selection to the text under the cursor. A message editor without a
selection of its own leaves ⌘C to the timeline.

### Units

Each message is split into units: paragraphs, headings, code blocks and
displayed math (`message.rs`). One walk over the document builds both the
view and the units (`text_map`), so the two cannot disagree. A unit has:

- its text as shown;
- a prefix used when copying from its start along with other units ("> "
  in quotes, "- " or "1. " in lists);
- the gap before it ("\n\n" between blocks, "\n" between list items).

A selection copies the text as shown, like a browser. It is not the
Markdown-like text of "Copy text": inline code has no backticks and links
have no URL, but math is `$…$` and lists and quotes keep their marks.

### Selection state

The selection lives in `Room` (`selection.rs`), not in the widgets:

```text
Selection { anchor: Point, focus: Point, dragging }
Point     { event: part key, pos: Pos }
Pos       { unit, offset }   // byte offset in the unit's text
```

### Rows and parts

`rows.rs` walks the shown entries once into rows (a message, under its
sender's name or grouped with the one before; a run of state events; a
day; the start), and each row into parts with text, in reading order:

| Part              | Key               | Units                         |
|-------------------|-------------------|-------------------------------|
| sender and time   | `{event}/sender`  | name, time (gap ", ")         |
| reply quote       | `{event}/reply`   | name (prefix "> "), preview (gap ": ") |
| message text      | event id          | the message's units           |
| line              | event id, `{first}/state`, `day/{secs}`, `start` | the line |

The view draws from these rows and parts and the copied text is made
from them, so the two cannot disagree. The room numbers the parts in
reading order (`rows::order`) to order the two points, and gives each
part what it covers (`Bounds::in_message`). Parts of a row are copied a
line apart, rows a blank line apart. With several messages, each sender's
run of messages is preceded by "Name, HH:MM", unless their sender line is
in the selection (it then copies as the same text).

A reply's preview is cut to a few lines with "…", depending on the
width, and is copied as shown. The room does not know the cut, so ⌘C
(and the menu's Copy, and ⌘F with a selection) first reads what each
preview shows from the view, by an operation (`Room::read_shown`), then
copies (`selection::Message::CopyAsShown`), text and HTML. Shift+arrows step through a
preview as a whole text, not as cut.

The mouse release that ends a drag is caught by a subscription, wherever
it happens.

### Widgets

- `selectable/geometry.rs`: what the two below share. `Geometry`
  hit-tests and highlights laid-out text in the units of `Pos`. `Pointer`
  reports `Press` (with the word or the unit for double and triple
  clicks; a double click just after a word selects it) and, while
  dragging, `Drag` when the cursor is beside the lines. A press and
  release on a link without moving still opens it, and `on_click` does
  the same for all the text (a sender's name, a quote). It uses the
  cursor's position, not the event's, because a scrollable only moves
  the cursor into its content's coordinates.
- `selectable/text.rs`: rich text as iced's `rich_text` draws it, its
  geometry through the paragraph's cosmic-text layout (`iced_graphics`,
  same version as iced). `lines` cuts it to fit a few lines, and `id`
  reports the text it shows.
- `selectable/image.rs`: a typeset math message, its geometry from the
  pieces of text over the image.
- `drag_scroll.rs`: wraps the timeline. While dragging past its edge it
  asks for scrolling each frame, and passes a cursor at the edge to the
  text, since the content moves under a cursor that does not.

### Math messages

The renderer (`mtrx-format`) puts invisible `metadata` markers around
each text leaf in the Typst source. A leaf is each text, inline code and
inline math, and each code block and displayed math, in reading order.
In the laid-out page, glyphs between a leaf's markers belong to that
leaf. Text glyphs carry their byte offset within the leaf. Code glyphs
(`raw`) carry their offset within their line, so a show rule on
`raw.line` puts a marker with the line's number before each line, and
the tabs `raw` turns into spaces are mapped back (all spaces of a tab are
one piece, the tab). Math is laid out from generated Typst, so it becomes
one piece per line, selected whole. `Image::pieces` lists each piece's
leaf, byte range and rectangle. `message::text_map` says where each leaf is in the units, so
the image is selected with the same units as native text, and formulas
copy as `$…$`. The text map also says which link each leaf is in (a
mention's too), so a piece of a link behaves as in native text: a press
and release without moving opens it (a mention, the person's card,
which has the way to the direct chat), the cursor is a pointing hand over
it, its address shows in the corner, and right-clicking it opens the
link's menu.

## Left to do

- Find (⌘F) searches messages' text only, not names, quotes or lines: it
  searches all loaded entries, the parts are of the shown ones.
- Info cards keep their own selection: names, IDs, addresses, links,
  read-only name and topic, and the facts line use `selectable_text`,
  which copies its selection with ⌘C.

## Tests

- Unit: units and their copy text (`message.rs`, `selection.rs`), hit
  testing and highlighting over an image (`selectable/image.rs`), double
  clicks, the cursor and the find bar's current match
  (`selectable/geometry.rs`), edge
  scrolling speed (`drag_scroll.rs`).
- Simulated mouse: clicking, hovering and right-clicking a link in a
  typeset message, and its cursor (`selectable/image.rs`).
- Simulated mouse: dragging, double and triple clicks, and dragging in a
  scrolled timeline (`message.rs`).
- Renderer: pieces in reading order and in the right places, code by
  character (`mtrx-format/tests/format.rs`), tabs (`render.rs`); in the
  app, code in a typeset message by character (`selectable/image.rs`).
- Rows and parts, and the copied text with names put once and quotes as
  shown (`rows.rs`); a click and a drag on text with an action, and text
  cut to its lines (`selectable/text.rs`); a click and a drag on a quote
  (`actions.rs`).
- Snapshot: a selection from inside a math message across three senders
  and a reply's quote, with its copied text (`app/tests.rs`).
