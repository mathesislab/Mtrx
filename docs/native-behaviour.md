# Behaving like a Mac app: to do

Status: A done in code; two items wait on a check by hand. What the text system and a native app give
for free, written down once so that it is done once, not found one issue
at a time.

## Why iced, and why this order

A web view (`docs/web-variant.md`, branch `web-variant-dd34eb`) gave the
composer, selection, accessibility and spell checking for free, at the
price of three rendering engines (WebKit, WebKitGTK, WebView2), with Linux
the weakest and most of our users on Mac and Linux. iced renders the same
everywhere, so the gaps below are paid for once, in our code or in a copy
of one iced widget.

The list is grouped by where the fix lives, and ordered by what it buys
per day:

- **A. App code.** Cheap, no fork, each item on its own. Do these first.
- **B. The composer.** iced's `text_editor` exposes no cursor by offset
  and no history, so these need a vendored `iced_widget` (as
  `vendor/iced_graphics` already is). One fork, one project, carrying
  every item in B.
- **C. Menu bar.** On macOS the menu's key equivalents are matched before
  winit sees the key, so every Edit shortcut must be routed back into the
  focused field. After B, not before.
- **D. Text rendering.** Screenshot tests; fixes where they fail.
- **E. Limits.** Not fixable in our code; listed so they are not
  rediscovered.

## How

- An item at a time, A before B before C.
- Each item gets a test with simulated input (`iced_test`, as in
  `app/tests.rs`) where it can, so it stays done.
- Fix a gap where it starts: B in the editor layer, not a key binding
  here and there.
- `[x]` done, `[ ]` to do; "unchecked" means nobody has tried it yet.

## A. App code

### A1. Window

- [x] Closing the window: decided 2026-10-25 from user feedback, by
      platform (`app::closing`). On macOS the close button, ⌘W and Window ▸
      Close hide the window, as a Mac app's: the app goes on, synced (Dock
      badge, notifications, presence), and clicking its Dock icon, a
      notification or its entry in the Window menu brings it back (a
      window in full screen leaves it first). Quit (⌘Q, the Dock, logging
      out) goes offline and ends the session through `Message::CloseWindow`.
      Windows and Linux keep one window whose closing quits (Ctrl+W too),
      through `Message::CloseWindow` (decided 2026-10-24). Hidden in our
      winit (`with_hides_on_close`), so the window and iced's state stay.
      To check by hand on macOS.
- [x] Size, position and display remembered (`placement.rs`), and
      maximised; full screen is not, the window opens as it was before. A
      window left on a display since unplugged opens on the main one.
- [x] Full screen, and tiling with the system's window layouts (checked
      by hand 2026-10-25). One macOS quirk, see E.

### A2. Keyboard

- [x] ⌘K: room search.
- [x] ⌘+ ⌘− ⌘0: text size.
- [x] Shift with an arrow does not extend a selection (reported with
      0.2.0). A press in the timeline never reached the composer, so it
      kept the focus and took Shift+arrows: iced's `Stack` stopped at an
      event captured beside it. Patched in `vendor/iced_widget`.
- [x] ⇧⌘← and ⇧⌘→ extend to the start and end of the line, ⇧⌘↑ and ⇧⌘↓
      to the first and last message on screen.
- [x] Page Up/Down and ⌥↑/⌥↓ scroll the timeline; ⌘↑/⌘↓ go to the oldest
      loaded and the newest message (when the composer does not take them).
      The composer passes Page Up/Down on, so they scroll from it too.
- [x] ⌥⌘↑ and ⌥⌘↓ (or ⌃Tab and ⌃⇧Tab) go to the room above and below;
      ⌘1–⌘9 to the first nine rooms.
- [x] ⌘F: find in the room's loaded messages (`find.rs`). Counted from
      the newest; Enter and ⌘G go to older matches.
- [x] A room opens (or is chosen again) with the cursor in its composer.
- [x] Typing with nothing focused goes to the composer.
- [x] ↑ in an empty composer edits our last message.

### A3. Mouse

- [x] Pointing hand over links and buttons, I-beam over selectable text.
- [x] Drop files onto the window to attach them.
- [x] The link's address shows on hovering it (in a corner, as in a
      browser).
- [x] Right-click a link: Open, Copy Link (for a mention, Info: the
      person's card, as clicking it shows).
- [x] Right-click a picture: Copy Image, Save Image…, in the timeline
      and in the viewer, sharing math's clipboard code (`message_menu.rs`).

### A4. Selection in the timeline

`docs/text-selection.md` has what is done.

- [x] Drag, double- and triple-click, Shift-click, ⌘A, ⌘C, across
      messages; math as `$…$`.
- [x] Sender names, reply previews, notices and state lines selectable.
- [x] Links in typeset (math) messages clickable.
- [x] Code in math messages selectable by character.

### A5. Clipboard and paste

Paste is a binding we can replace (`Binding::Custom`, then
`Action::Edit(Edit::Paste)`), and `arboard` reads pictures and HTML, so
these stay outside the widget.

- [x] Copy as text, math as `$…$`.
- [x] Copy as HTML as well, so pasting into Mail or a document keeps
      formatting, links and lists.
- [x] ⌘V of a picture (a screenshot, say) attaches it, as dropping a file
      does.
- [x] ⌘V of rich text (from a web page or Mail) as Markdown: links, lists,
      emphasis.
- [x] ⌥⇧⌘V pastes as plain text.
- [x] Right-click in the composer: Cut, Copy, Paste, Select All (and
      Emoji & Symbols), `edit_menu.rs`. The pick needed the winit
      #4748/#4749 backport and a field that keeps its input method while
      the Character Viewer is key (checked by hand 2026-10-25).

## B. The composer: inside `text_editor`

iced 0.14's `text_editor` has no undo, and of the macOS key bindings only
the arrows (with ⌥, ⌘ and ⇧), Home/End, ⌘A/C/X/V and ⌃A/E/B/F/H/D. Its
`Content` gives the text, the selection and a line/column cursor: nothing
to build an undo stack or a word deletion on from outside. So: vendor
`iced_widget`, and do all of B in that copy. Vendored with A2
(`vendor/iced_widget`, its README lists our patches).

Decided (2026-10-24): the patch first, ours to keep; whether to offer it
upstream is judged once it works, so no feature waits on iced.

### B1. Input methods

The group writes European languages, so accents come first. Chinese,
Japanese and Korean go through an input method (type, pick a candidate,
Enter confirms), which nobody here uses: correct, not urgent. The widget
forwards every key press to the binding without looking at the preedit,
so that Enter probably sends the half-composed text today.

- [x] Dead keys: ⌥E then E gives é (checked by hand 2026-10-25).
- [x] Press-and-hold accent menu (#30): the held letter is typed once
      (keys the input system keeps type nothing) and the pick replaces it,
      with the mouse or a digit. The fields report the text around the
      caret to macOS (winit 0.31's surrounding text, through
      `vendor/iced_core`, `iced_winit`, `iced_widget`, `winit`). Checked by
      hand in the composer 2026-10-25.
- [ ] Chinese, Japanese and Korean input: candidates next to the caret,
      and Enter confirms the composition rather than sending.
- [ ] Dictation (🎤 or Fn Fn). winit reported no caret, so Dictation
      would not start; patched in `vendor/winit` (winit #4748/#4749). To
      check by hand, from the app bundle (`cargo xtask bundle`): without
      a bundle identifier Dictation stalls.
- [x] ⌃⌘Space opens Emoji & Symbols and inserts the pick. The pick is
      inserted (winit #4748/#4749 backport), but ⌃⌘Space itself is the key
      equivalent of the Edit menu's Emoji & Symbols, which macOS adds to an
      Edit menu: the Edit menu is there now (C). Checked by hand
      2026-10-25.

### B2. Editing

- [x] Double- and triple-click select a word and a line (iced).
- [x] ⌘Z undo, ⇧⌘Z redo; typing grouped into words, as in a text view.
      The history is the editor's (`vendor/iced_graphics`): a new draft
      (sent, another room) starts a fresh one. A paste, an input method's
      commit and a completed mention are a step each.
- [x] ⌥⌫ and ⌥⌦ delete a word; ⌘⌫ deletes to the start of the line
      (as shown, when it wraps), ⌘⌦ to its end. A step each to undo.
- [x] ⌘↑ and ⌘↓ go to the start and end of the draft (with ⇧, select).
      The timeline takes them (A2) while no editor is focused.
- [x] ⌥↑ and ⌥↓ go to the start and end of the paragraph, then on to the
      one before or after (with ⇧, select). The timeline takes them (A2)
      while no editor is focused.
- [x] ⌃K deletes to the end of the line, ⌃Y yanks it back, ⌃T transposes,
      ⌃N and ⌃P move down and up, ⌃O opens a line. macOS only. ⌃K goes to
      the end of the paragraph, as macOS's; its text is kept once for the
      app, so ⌃Y yanks it into the other editor too. iced's ⌃A, ⌃E, ⌃B, ⌃F
      and ⌃D did nothing (their text, a control character, typed nothing);
      they work now, ⌃A and ⌃E to the paragraph's edges as macOS's.
- [x] Dragging selected text moves it, with ⌥ copies it.
- [ ] Dropping text from another app inserts it; dragging it out to one.
      winit takes only files dropped (`registerForDraggedTypes` with
      `NSFilenamesPboardType` in `window_delegate.rs`, and its
      `draggingEntered:`/`performDragOperation:` read file names only). It
      needs `NSPasteboardTypeString` registered there, a dropped text
      event with its position (and one while it is dragged over, for the
      caret) through `iced_winit` to the editor, as `Edit::MoveSelection`
      but inserting; dragging out needs an `NSDraggingSource`
      (`beginDraggingSessionWithItems`) from the view.

## C. Menu bar

The app had no menu bar of its own (winit's default app menu only), so no
Edit menu: none of its items, and no place to find shortcuts. A menu takes
the shortcuts of its items before the window, so ⌘C or ⌘Z never reach the
focused field as key presses.

Decided (2026-10-25): macOS only (Windows and Linux keep none), with
AppKit's menus (`menu_bar.rs`, objc2), not `muda`. The Edit and View items
send the text system's action messages (`copy:`, `undo:`,
`makeTextLarger:`, ...) to the window's view, which our winit passes on as
standard actions (`Event::StandardAction`, through our `iced_winit` and
`iced_core`): the focused editor or text input does what the key does (the
editor through its key binding, so ⌘V still pastes pictures), else the
app's shortcuts take them as the keys (the timeline's selection, find, the
text size). The fields tell what they can undo, redo and cut along with
their input method, and the items are enabled by it. A disabled item still
takes its shortcut, which then reaches nothing (checked 2026-10-25), so an
item is disabled only where its key does nothing. Quit goes through
closing, also from the Dock and when logging out (`applicationShouldTerminate:`
in our winit). See the READMEs in `vendor/`.

Done, with tests of the actions doing what the keys do, and checked by
hand on macOS 2026-10-25:

- [x] App menu: About, Settings… (⌘,), Hide, Hide Others, Show All, Quit.
      Quit (⌘Q) goes through `Message::CloseWindow`, as closing does:
      winit's own Quit exited without going offline or ending the session.
- [x] Edit: Undo, Redo, Cut, Copy, Paste, Paste as Plain Text (⌥⇧⌘V),
      Select All, Find (⌘F, ⌘G, ⇧⌘G); macOS adds Emoji & Symbols, Start
      Dictation and AutoFill.
- [x] View: Bigger, Smaller, Actual Size (⌘+ ⌘− ⌘0, already keys), Enter
      Full Screen (⌃⌘F).
- [x] Window: Minimize (⌘M), Zoom, Close (⌘W), Bring All to Front.
- [x] Help: the website.

## D. Text rendering

Screenshot tests (as in `app/tests.rs`) at each text size, in messages and
in the composer. All unchecked.

- [ ] Colour emoji, sequences (families, skin tones) and flags.
- [ ] Chinese, Japanese and Korean, with their fonts.
- [ ] Right to left (Arabic, Hebrew), mixed with left to right.
- [ ] Combining marks (é as e + ´), Devanagari, Thai.
- [ ] Long words and URLs wrap within the message.
- [ ] Math next to text in each of the above.

## E. Limits of iced

Not fixable in our code alone.

- **Accessibility.** iced has no accessibility tree, so VoiceOver,
  Voice Control and Switch Control see nothing. Upstream work in iced.
- **The system's text services.** Spell checking, autocorrect, text
  replacements, smart quotes, Writing Tools, Services and Look Up come with
  a native text view. Spell checking could be done with `NSSpellChecker`,
  but iced's editor cannot underline a word yet.
- **Dragging out of the window.** winit cannot start a drag, so a picture
  or file cannot be dragged into Finder or another app; it would take a
  native drag session per platform (decided 2026-10-24: not now).
- **Top and bottom tiles.** A window tiled to the top or bottom half and
  then resized goes back to its tile when chosen again in Mission Control.
  Left and right tiles keep the new size. Nothing in our code, iced or
  winit sets the frame: macOS's tiling (checked 2026-10-25).
- **Keyboard focus.** Tab and ⇧Tab between controls with a focus ring, and
  Space to press a button: iced's buttons are not focusable.
