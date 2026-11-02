# Mtrx
Matrix-protocol communication app

Website and download: <https://mathesislab.github.io/Mtrx/>

Native Rust Matrix client for macOS, Windows and Linux, built on
[matrix-rust-sdk](https://github.com/matrix-org/matrix-rust-sdk).

## Layout

- `crates/mtrx-core`: Matrix logic and state (sessions, room list, timelines,
  encryption). No UI; exposes plain types and live update streams.
- `crates/mtrx-format`: Markdown/LaTeX to Matrix HTML, HTML to a document
  model, math typesetting with Typst.
- `crates/mtrx-cli`: the `mtrx` command-line client on top of core.
- `crates/mtrx-app`: the desktop app (iced).
- `crates/xtask`: build tasks (`cargo xtask bundle`, `icons`, `version`).
- `testserver/`: local Synapse for development and tests.

## Desktop app

```sh
cargo run --release -p mtrx-app
```

On macOS, `cargo xtask bundle` builds `target/bundle/Mtrx.app` with its icon
(`crates/mtrx-app/assets/logo.svg`); drag it to Applications.

## CLI

```sh
cargo run -p mtrx-cli -- --help
cargo run -p mtrx-cli -- login --homeserver matrix.org --user alice
cargo run -p mtrx-cli -- rooms
cargo run -p mtrx-cli -- read "Room name" -n 20
cargo run -p mtrx-cli -- send "Room name" "Hello **world**"
cargo run -p mtrx-cli -- watch "Room name"
cargo run -p mtrx-cli -- schedule "Room name" "tomorrow 9:00" "Good morning"
cargo run -p mtrx-cli -- scheduled       # list; then unschedule ID / send-now ID
cargo run -p mtrx-cli -- note "Idea: compare basal melt"   # a note to self
cargo run -p mtrx-cli -- notes --since 2026-10-01 -o notes.md  # notes as Markdown
cargo run -p mtrx-cli -- verify          # emoji verification with another device
cargo run -p mtrx-cli -- recovery recover
cargo run -p mtrx-cli -- presence @bob:matrix.org   # state, last active, status
cargo run -p mtrx-cli -- presence --set unavailable --status "In a meeting"
```

Scheduled messages are kept (encrypted) on this device and sent while the app
or `mtrx watch` runs. One whose time passed while neither ran is held as
overdue until you send or cancel it.

Rooms and spaces created with Mtrx trust their members: anyone may invite,
rename, change the topic (with the room's calls and links) and picture, and pin
messages. Removing people, deleting others' messages and changing permissions stay with
admins. `--moderated` gives the usual Matrix rules instead, and
`let-everyone-edit ROOM_OR_SPACE` opens existing rooms (needs admin rights).

A room's calls and links are lines at the end of its topic, so other apps
show them too: `[call] URL`, `[call Weekly passcode 123] URL`, `[link] URL`,
`[link Paper draft] URL`. The first call is the one the header's Join button
opens; the others, and the links, are in the menu beside it. The caret below
the room's name opens the topic panel: the topic in full, its links, and the
pinned messages (pinned from a message's menu), each a click away.

Presence (online, idle, offline, last active, a status message) is read
from the server, about once a minute for the people shown, as sync does
not carry it; our own is sent again every 20 s while Mtrx runs, since
servers drop it otherwise (Synapse after 30 s). Some servers do not share
presence: `mtrx presence` then says "This server does not share
presence", or, for Synapse with presence off, shows everyone offline with
no last activity. The apps show it as `docs/mobile.md` section 11
describes; the desktop is idle after 5 minutes without input, focused
or not, and offline once closed.

`--profile NAME` selects an account. Secrets go to the OS keychain;
`--secret-store file` keeps them in a plain file instead (development only).

## Google Meet links

Mtrx can create Google Meet links with your own Google OAuth client; it ships
no Google credentials. One-time setup in the
[Google Cloud console](https://console.cloud.google.com/):

1. Create a project.
2. APIs & Services > Library: enable "Google Meet REST API".
3. APIs & Services > OAuth consent screen: user type External; add your
   Google account under Test users.
4. APIs & Services > Credentials > Create credentials > OAuth client ID,
   application type "Desktop app". Note the client ID and secret.

Then sign in once (or later in the app's Settings):

```sh
cargo run -p mtrx-cli -- google login --client-id ID   # prompts for the secret
cargo run -p mtrx-cli -- google meet                   # prints a new Meet link
cargo run -p mtrx-cli -- google status                 # or: google logout
```

As long as the app is unverified ("Testing" status), only the listed test
users can sign in. The Google sign-in is app-wide, not per profile.

## Local test server

Needs Docker (on macOS e.g. `brew install colima docker && colima start`).

```sh
testserver/run.sh                  # Synapse on http://localhost:8008
testserver/add_user.sh alice alicepw
cargo test -p mtrx-core --test integration -- --ignored
cargo test -p mtrx-app -- --ignored   # UI snapshots in target/ui-snapshots/
```

## Releasing

```sh
cargo xtask version 0.1.5          # Cargo.toml, Cargo.lock, iOS and Android apps
git commit -am "Version 0.1.5" && git push
git tag -a v0.1.5 -m "Mtrx 0.1.5" && git push origin v0.1.5
```

A test fails if the apps' versions differ from `Cargo.toml`. The tag runs
`.github/workflows/release.yml`, which builds the macOS, Linux and Windows
packages and attaches them to a draft release, to be published by hand; the
website links to the latest published release's.

## License

Copyright (C) 2026 Alexander Robinson

Mtrx is free software: you can redistribute it and/or modify it under the
terms of the GNU Affero General Public License as published by the Free
Software Foundation, either version 3 of the License, or (at your option) any
later version. See [LICENSE](LICENSE).

In short: if you distribute Mtrx or a program built on it, or run a modified
version for others over a network (a bot, bridge or hosted client), you must
make its source code available under the same licence.

The code up to and including commit 35a7290 (20 October 2026) was published
under the MIT licence.
