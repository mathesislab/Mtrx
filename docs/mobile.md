# iOS and Android: the shared Rust layer

Status: the Rust layer (`crates/mtrx-ffi`) and the first iOS and Android
apps (sections 3 and 4) are written. The library builds for both in CI
(`mobile-ffi.yml`); each app builds, passes its unit tests and starts to the
sign-in screen in CI, iOS on the simulator (`ios.yml`) and Android on an
emulator (`android.yml`). Unsigned. The iOS app was tried on a phone: it
signs in and lists rooms, but a new device cannot read encrypted rooms
without recovery or verification, which stage 1 (section 6) adds; the
library side of it is done. Stage 2 (section 7) organises the room list by
space, and stage 3 (section 8) adds reactions, replies, edits, deleting,
read receipts and typing; their library sides are done too. Stage 4
(section 9), which polishes what the phone test of stages 1 to 3 found,
push notifications on Android through UnifiedPush and ntfy (section 10),
and presence (section 11, also on the desktop) are now in the apps. This
is the plan, and the contract the apps are built on.

## Decisions

- Native UIs on one Rust core: SwiftUI on iOS, Jetpack Compose on Android,
  both on mtrx-core through UniFFI bindings (as Element X does).
- First version, the same on both: sign in with a password or in the
  browser (OAuth), room list, room timeline with text and images, send a
  text message, sign out. No push, calls, scheduled messages or Google.
- One account per app (profile `default`).
- Mobile builds run on GitHub Actions only (macOS runner for iOS, Ubuntu for
  Android). Locally we build and test for the Mac. Unsigned; for our team.
- Minimums: iOS 16; Android API 26.

## 1. Core changes

- [x] Secrets from the app: `SecretStore::Custom(Arc<dyn SecretBackend>)`.
      The OS keyring (`SecretStore::Keyring`) is desktop only, because
      keyring 4's `Entry` API does not build for iOS (its Apple store needs
      the `protected` feature there, `v1` enables only `keychain`) and has no
      store on iOS or Android (`Entry::new` fails with `NoDefaultStore`).
      keyring-core with `apple-native-keyring-store::protected` (iOS) or
      `android-native-keyring-store` would work, but one callback serves both
      apps. A custom store gets keys relative to the data directory
      (`profiles/default`), since iOS moves an app's container on updates.
- [x] `Profile::new` takes the data root; `with_cache_root` moves the SDK's
      event and media caches to a directory the OS may clear. The desktop
      default (`directories`) is not built for iOS and Android.
- [x] Browser login in two steps: `Session::start_oauth_login(profile,
      homeserver, redirect_uri)` returns the login page URL;
      `OAuthLogin::finish(redirect_url)` completes it. The desktop's
      `login_oauth` (local web server) is built on them. The mobile redirect
      is `APP_OAUTH_REDIRECT_URI` = `io.github.alex-robinson:/oauth`: a
      private-use scheme, which servers (MAS) require to be the client URI's
      host (`alex-robinson.github.io`) reversed.
- [x] `MediaRef` serializes (handed to the apps as an opaque string).
- [x] Other dependencies build for mobile as they are: reqwest with rustls
      (platform verifier, see Android below), bundled SQLite, `image`.
      `open` and notify-rust are in the CLI and app only.

## 2. The FFI (`crates/mtrx-ffi`)

UniFFI 0.32 with proc-macros; namespace `mtrx` (Swift module `MtrxFFI`
wrapping the C module `mtrxFFI`; Kotlin package `uniffi.mtrx`). Library
types `staticlib` (iOS), `cdylib` (Android), `lib`; binary `uniffi-bindgen`.
IDs are strings, times `Date`/`Instant`; no Ruma types.

- `Mtrx(dataDir, cacheDir, secrets)`: one per app. `restore() -> Session?`,
  `login(homeserver, user, password)`, `supportsOauth(homeserver)`,
  `startOauthLogin(homeserver) -> OAuthLogin`. Push (section 10):
  `notification(roomId, eventId) -> Notification?`, from the
  process that receives a push, with or without the app's session.
- `OAuthLogin`: `url()`, `finish(redirectUrl) -> Session`.
- `Session`: `userId`, `deviceId`, `homeserver`; `startSync`, `stopSync`,
  `reconnect`; `connection()`, `subscribeConnection(listener)`;
  `subscribeRoomList(listener)`; `timeline(roomId) -> Timeline`;
  `draftKeeper(roomId) -> DraftKeeper`;
  `avatar(mxcUri, size) -> Data`, `mediaPreview(media) -> Data`,
  `mediaFile(media) -> path`; `logout`, `forget`. Encryption (section 6):
  `encryptionState()`, `subscribeEncryptionState(listener)` (section 9),
  `recoveryStatus()`, `recover(recoveryKey,
  listener)`, `enableRecovery() -> key`, `isDeviceVerified()`, `devices()`,
  `requestSelfVerification() -> Verification`,
  `subscribeVerificationRequests(listener)`. People (section 9):
  `userProfile(userId) -> User`, `invite(roomId, userId)` (to a
  room or a space). Push (section 10): `setPusher(gatewayUrl, pushkey,
  appId, appName, deviceName, lang)`, `deletePusher(pushkey)`,
  `pushers() -> [PusherInfo]`. Presence (section 11):
  `presence(userId) -> Presence`,
  `presenceActivity(activity)`, `setPresenceSharing(on)`,
  `subscribePresence(userIds, listener) -> TaskHandle`.
- `Verification`: `otherUser()`, `accept()`, `state()`, `subscribe(listener)`,
  `confirm()`, `mismatch()`, `cancel()`.
- `Timeline`: `roomId`, `subscribeRows(listener)`, `paginateBack(events)`,
  `seek(eventId) -> itemId` (loads older events until the message is
  there; `MtrxError.EventNotFound` if not), `itemOf(eventId) -> itemId`
  (waits, without loading, until the message is there), `sendText(markdown)`,
  `markRead`. Messages (section 8), each taking the
  row's `TimelineRow.id`: `reply(itemId, markdown)`, `edit(itemId,
  markdown)`, `react(itemId, key)` (adds ours, or removes it),
  `delete(itemId)` (redacts), `retry(itemId)`, `discard(itemId)`; and
  `setTyping(typing)`, `subscribeTyping(listener)`.
- `DraftKeeper`: a room's draft, kept on the device. `restore() -> Draft?`
  (once), `changed(draft)` after every change of the composer (saved
  after a 1.5 s pause), `flush()` when leaving the room or going to the
  background.
- Listeners (implemented by the apps): `RoomListListener.onUpdate(RoomListState)`,
  `TimelineRowsListener.onUpdate([TimelineRow])`,
  `TypingListener.onUpdate([String])` (names; on every change only),
  `ConnectionListener.onUpdate(ConnectionState)`,
  `EncryptionStateListener.onUpdate(EncryptionState)`,
  `VerificationListener.onUpdate(VerificationState)`; each gets the whole
  state, now and after every change. `PresenceListener.onUpdate([String:
  Presence])` gets all presences known after every change, and
  `onUnavailable()` once if the server does not share presence. `RecoveryListener.onUpdate(RecoveryStep)`
  gets each step of a recovery; `VerificationRequestListener.onRequest(Verification)`
  each incoming request. `SecretBackend`: `load`, `save`,
  `delete` (throws `SecretError.Failed`).
- `TaskHandle` (from each subscribe): `cancel()`, `isFinished()`; dropping
  it cancels.
- Records: `RoomListState { rooms, loaded, spaces, roomsOutsideSpaces }`
  (section 7), `SpaceRooms { spaceId, roomIds }`, `RoomSummary`, `TimelineRow
  { id, content }` (`start`, `day`, `readMarker`, `summary(text)`, `message` with
  its grouping, `decrypting` and `actions: [MessageAction]` in the menu's order),
  `TimelineEvent` (with `sender`, `editable`, `readBy: [User]`), `EventContent`,
  `Message { kind, body, edited, media, reactions, replyTo, rawBody }`,
  `Reaction { key, count, byUs, senders: [User] }`, `User { userId, name,
  avatarUrl, displayName }` (`displayName`: the name, else the ID's
  localpart), `ReplyQuote { eventId, state, sender: User?, kind, body }`,
  `QuoteState` (`loading`, `loaded`, `failed`), `Media`,
  `SendState` (`failed(error)`: retry or discard). `TimelineRow.id` is unique and stable (also when a
  local echo becomes the server's event): use it as list identity.
  `EventContent.unableToDecrypt`: an encrypted event without its key (yet);
  `TimelineEvent.utdCause: UtdCause?` says why, as far as the SDK knows
  (`unknown`, `sentBeforeWeJoined`, `verificationViolation`,
  `unsignedDevice`, `unknownDevice`, `historicalMessageAndBackupIsDisabled`,
  `historicalMessageAndDeviceIsUnverified`,
  `withheldForUnverifiedOrInsecureDevice`, `withheldBySender`; nil for
  other events).
  Encryption: `EncryptionState { deviceVerified, recovery, keysArriving }`,
  `RecoveryStatus` (`unknown`, `enabled`, `disabled`, `incomplete`),
  `RecoveryStep` (`checkingKey`, `importingIdentity`,
  `downloadingKeys(done, total)`), `VerificationState` (`waiting`,
  `emojis([VerificationEmoji])`, `confirmed`, `sharingKeys`, `done`,
  `cancelled(reason)`), `VerificationEmoji { symbol, description }`,
  `DeviceInfo { id, name, verified, current }`. Push: `PusherInfo {
  pushkey, appId, appName, deviceName, lang, gatewayUrl, eventIdOnly }`,
  `Notification { roomId, eventId, roomName, roomAvatarUrl, direct,
  sender: User, body, highlight, noisy, timestamp }`. Presence: `Presence { state,
  lastActive, currentlyActive, statusMessage, dot, line }`, `PresenceState`
  (`online`, `unavailable`, `offline`), `PresenceDot` (`online`, `idle`),
  `PresenceActivity` (`foreground`, `background`, `input`, `signOut`).
- Errors: `MtrxError` (`NotLoggedIn`, `AlreadyLoggedIn`, `LoginCancelled`,
  `RoomNotFound`, `InvalidInput`, `SecretStorage`, `PresenceUnavailable`,
  `Other(message)`).
  `InvalidInput(reason)`: a malformed ID or URL, or a recovery key that is
  not one or not the account's ("The recovery key is not valid").
  `PresenceUnavailable`: the server does not share presence (section 11).
- `oauthRedirectUri()`, `initLogging(filter)` (stderr: Xcode's console),
  `presenceLine(presence, now)`, `presenceDotLabel(dot)`.

Full snapshots instead of diffs: simpler for the apps, and cheap at our
sizes. Diffs (the SDK's `VectorDiff`s) can come later for big timelines.

### Threading

- The library owns a multi-threaded tokio runtime. Each async call runs as a
  task there and is awaited from Swift's or Kotlin's executor; cancelling
  the Swift `Task` or Kotlin coroutine cancels it.
- Listeners and `SecretBackend` are called on the runtime's threads. The
  apps hop to the main thread before touching UI state (`@MainActor` /
  `DispatchQueue.main`; `Dispatchers.Main` / a `StateFlow`), and return
  quickly.
- Objects may be used from any thread. Swift releases them deterministically;
  in Kotlin call `close()` (or `cancel()` on a `TaskHandle`) instead of
  waiting for the garbage collector.

### Media

`avatar` and `mediaPreview` return encoded image bytes (cached in the SDK's
store); `mediaFile` saves the whole decrypted file under
`<cacheDir>/profiles/default/media/` and returns its path. Signing out
deletes it.

## 3. iOS app

- Build: `scripts/build-ios-ffi.sh` fills the Swift package
  `apps/ios/MtrxFFI` (`mtrxFFI.xcframework` for device and simulator, arm64;
  `Sources/MtrxFFI/mtrx.swift`). Add it as a local package; `import MtrxFFI`.
  CI's artifact `MtrxFFI-ios` is the same package, zipped.
- Start: `Mtrx(dataDir: Application Support, cacheDir: Caches, secrets:)`
  with a Keychain `SecretBackend` (generic passwords, service
  `org.mtrx.Mtrx`, account = key, `kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly`).
  Then `restore()`; if nil, show sign-in.
- Sync: `startSync()` when the scene becomes active, `stopSync()` when it
  goes to the background (iOS suspends the process).
- Browser login: `ASWebAuthenticationSession(url: login.url(),
  callbackURLScheme: "io.github.alex-robinson")`; pass the callback URL to
  `finish`. No URL type in Info.plist is needed for this session.

### What exists

SwiftUI, iOS 16, iPhone and iPad, bundle `org.mtrx.Mtrx`; no team (signed
ad hoc for the simulator). Sign in
(the homeserver, then a password or the browser, as `supportsOauth` says),
room list, room timeline with text and images, sending text, sign out.
Stage 1 (section 6): the banner, Settings' "Encryption" section, the
recovery key screen with its steps, emoji verification in both directions
(one sheet over everything, Settings returns after it; a request that
comes while another is unanswered replaces it, one during a verification
is cancelled), "The recovery key is not valid." under the field, and
"Could not decrypt this message." rows. Stage 2 (section 7): the Chats
and Rooms tabs. Stage 3 (section 8): reactions, replies, edits,
deleting, retrying and discarding a refused send, read receipts and
typing. Stage 4, the app's part: Settings as a third tab, the Rooms tab
named Spaces, a space's rooms in Rooms and People sections with a sort
menu, swiping a message to reply, the person's picture in a direct
chat's header, and a button to the newest message. Stage 4, the library's
part: the encryption state pushed, "Decrypting…" while keys arrive, and
the person's info sheet. Presence (section 11): the dots, the line under
the name, "Share my presence" and ours from the lifecycle.

```
apps/ios/
  Mtrx.xcodeproj      app target Mtrx, tests MtrxTests, local package MtrxFFI;
                      folders are synchronized groups (no per-file entries)
  Mtrx/
    MtrxApp.swift     entry; scene phase -> sync and our presence
    Core/             Keychain backend, Feed (listeners -> AsyncStream),
                      browser sign-in, error messages, InputWatcher
                      (touches and typing, for idle)
    Models/           AppModel (phase: starting, signed out, signed in),
                      SignInModel, SessionModel (room lists, selected space,
                      sort, connection, sync, encryption state,
                      verification requests), PresenceModel (others'
                      presences, ours, the switch), RecoveryModel,
                      VerificationModel, TimelineModel (also composing,
                      actions, typing, going to a quote, at the bottom),
                      PersonInfoModel, ImageCache, TimelineRows (rows,
                      menu entries),
                      RoomLists (also sections, sort), SwipeToReply, Emoji
    Views/            sign-in, home (Chats, Spaces and Settings tabs,
                      encryption banner), room (timeline, typing line,
                      composer, header, the person's info sheet),
                      message sheets (who reacted or read, all emoji,
                      select text), settings, recovery key, verification
    Assets.xcassets   icon and logo (`cargo xtask icons`), accent colour
  MtrxTests/          models with fakes of the FFI classes
```

- Threading: listeners yield into an `AsyncStream` that keeps only the
  newest state; a main-actor task applies it. Models are `@MainActor`
  `ObservableObject`s; views only read them.
- Home: a `TabView` with Chats, Spaces and Settings, each a
  `NavigationStack` (also on iPad); rooms open with the tab bar hidden
  (`.toolbar(.hidden, for: .tabBar)`), recovery pushes from Settings, and
  a verification is a sheet over any tab. `RoomLists` derives the chats
  and the spaces (with their rooms and unread dots, then "Other rooms")
  from each `RoomListState`. Spaces is a paged `TabView` (spaces, the
  selected space's rooms; a "Spaces" button goes back too); the page and
  the selected space live in `SessionModel`, the space in UserDefaults
  (`selectedSpace`: a space ID or `other-rooms`), removed at sign-out. A
  space's rooms (and Other rooms) are in two sections, Rooms and People
  (its direct chats; an empty section is left out), both in the order of
  the page's sort menu: Default (the list's), A to Z, Z to A, Most recent
  (`lastActivity`, none last; ties keep the list's order). The choice is
  for all spaces, not Chats, and kept in UserDefaults (`roomSort`), also
  after sign-out. The encryption banner is the first row of each list and
  leads to Settings; each list has a search by name, names starting with
  it first (while searching, that is the order). Invites are listed but
  answered on the desktop (no join call in the FFI yet).
- Encryption state: `subscribeEncryptionState` with the other
  subscriptions in `SessionModel.start()`, just before the first
  `startSync()` (the library pushes the state now, `recovery` `unknown`
  before sync, and on every change, so the order does not matter), and
  cancelled with them at sign-out. `SessionModel.encryption` is the last
  pushed state; `encryptionState()` is not called (the first push comes at
  once). Recovery and verification only end: the banner goes when the
  pushed state says so. Undecryptable rows say "Decrypting…" while
  `keysArriving`, else "Could not decrypt this message."
  (`MessageRow.undecryptableText`); `SessionModel.keysArriving`, a
  publisher, is handed to each `TimelineModel`, which rebuilds its rows
  when it changes, so rows on screen follow. `utdCause` is not shown.
- Timeline: rows keyed by `TimelineRow.id`, as the library gives them:
  runs of state events in one line, messages grouped by sender.
  A flipped scroll view (the list and each row upside down) on every
  version, so the newest row is the lazy stack's first, where it starts.
  Anchored at the bottom instead (`defaultScrollAnchor`, as on iOS 17+
  before), a reopened room sometimes landed short of the bottom, or past
  it with nothing drawn: the lazy stack estimates the rows it has not
  drawn from those it has, every change of the rows (more loaded,
  decrypted, a reset) changes the estimate of all of them, and the anchor
  kept an offset that was no longer the bottom. Flipped, the scroll edge
  effects (iOS 26) are hidden, as they would be drawn at the wrong edges,
  and the list is clipped to the bars; the context menu has its own
  preview (the flipped one is squashed into a line); VoiceOver reads the
  rows oldest first. Scrolled more than 40 pt up (iOS 18+: the scroll
  geometry; before: the end no longer drawn), a round down-arrow button
  at the lower right goes back to the bottom. Older messages load while
  the top is on screen; read when shown.
  Others' messages outside direct chats have the sender's name above the
  first of a group and their avatar (32 pt) beside its bubble, with the
  avatars' column kept for the rest of the group; direct chats show
  neither, as before.
- Messages (section 8): holding a bubble opens a context menu (the
  bubble is its preview) with what `MessageRow.actions` allows: on iOS
  17 the six common emoji as a palette (ours selected), then "More
  Reactions" (a palette fits six, so no "+" in it); on iOS 16 a "React"
  submenu with both. Then Reply, Copy, Select Text (a sheet with a
  `UITextView`), Edit (`editable`), Delete (own, sent; confirmed), Retry
  and Discard (failed). React and Reply once `eventId != nil`. The full
  picker is a searchable grid of the emoji the system's Unicode data
  marks as emoji by default, one character each, searched by Unicode
  name (no skin tones, flags or sequences). Bubbles no longer have
  `textSelection` (its long press would take the menu's). From iOS 18,
  swiping a message to the left (one with Reply) replies to it: the
  bubble follows, a reply icon shows at the right, and letting go past
  50 pt replies. A UIKit pan (`UIGestureRecognizerRepresentable`) that
  begins only for a mostly leftward drag, so scrolling and the back
  gesture (rightward) are left alone; a SwiftUI drag in the flipped list
  stopped it scrolling.
- Header: in a direct chat with someone else (`TimelineModel.person`: a
  `directUser` not ours, so not the Notes to self) the title is the
  room's picture and name, a button opening the person's info sheet
  (`TimelineModel.personInfo`, a `PersonInfoModel`; medium and large
  detents): the chat's name and picture at once, then the profile's
  (`userProfile`) where set (a failure keeps the chat's, silently); the
  user ID, selectable, with a copy button ("Copied" for 2 s); "Add to
  space…" (left out without spaces) pushes the top-level spaces
  (`RoomLists.topLevelSpaces`, following the room list while open) with
  their pictures. Tapping one invites (one at a time, a spinner on it); on
  success the list closes and "Invited to <space>." shows under the
  button for 3 s; on failure the reason shows in an alert, and the list
  stays.
- Reactions: chips under the bubble, wrapping (`FlowLayout`); ours in
  the accent colour; tap toggles, hold lists who reacted. Quotes: a bar,
  the sender and three lines inside the bubble (above an image, in a
  bubble of its own); a file's kind and name as "Image: cat.png"; tapping
  a loaded one scrolls to the original and marks it for 2 s, after
  `seek(eventId)` loaded older events as needed (scrolled once the rows
  have the row it returns; a second tap while it looks is ignored). Read
  receipts: 16 pt avatars at the row's right, three
  then "+N"; tap lists them. "(edited)" follows the text.
- Composer: a bar "Replying to <name>" with the first line, or "Editing"
  (the field holds `rawBody`; the draft from before comes back after);
  ✕ cancels; a send that fails restores both. `setTyping` on every
  change of the draft (calls chained so they arrive in order), `false`
  when the room disappears. The typing line sits above the composer
  (always one line high); three people are "Anna, Ben and Cleo are
  typing…", more "Anna, Ben and N others…". `subscribeTyping` runs
  while the room is on screen.
- Actions (`react`, `delete`, `retry`, `discard`) run in tasks the model
  creates; sending, replying and editing in `send()`. The library runs
  back-paginations one at a time, so loading older messages and looking
  for a quote do not overlap.
- Drafts: the room's `DraftKeeper` restores the draft when the room
  opens (put in the composer only if it is empty; its reply once
  `itemOf` found the message and the rows have it), hears every change
  (during an edit, the draft from before it), and flushes when the room
  disappears, after sending, and when the app leaves the foreground.
- Presence (section 11): `PresenceModel`, owned by `SessionModel` and an
  environment object of its own. `SessionModel.setForeground` (from the
  scene phase) runs the lifecycle: in the foreground `presenceActivity(
  .foreground)` (the first time after `setPresenceSharing`) before
  `startSync()`; to the background, a `beginBackgroundTask`, then
  `.background`, then `stopSync()`. `logout` tells `.signOut` first;
  `forget` tells nothing. These calls are chained, so they reach the
  library in order. Input: `InputWatcher`, behind the tabs, puts a
  recognizer on the window that reports each touch or press and fails at
  once (it cancels and delays nothing; sheets are in the same window), and
  counts text changing (`UITextField`/`UITextView` notifications) as
  input, as the keyboard is a window of its own; each is `.input` (the
  library keeps the idle timer). Others: the subscription is for the Chats
  tab's people, which include the open direct chat's (it is in the room
  list); dots only in Chats, not in a space's People section. The dot's
  ring is outside its size (12 + 2 + 2 pt), centred on the avatar's
  circle at 45°. The line re-renders each minute (`TimelineView(
  .everyMinute)`, with `presenceLine(presence, now)`). The person sheet
  calls `presence` from its `.task`, into the map (the subscription's
  next update replaces it). Once the server does not share presence,
  nothing more is told either (every call would fail).
- Images: avatars and `mediaPreview`s decoded off the main thread, kept in
  an `NSCache` (the library caches the downloads).
- Tests: fakes subclass the generated classes (`init(noHandle:)`), so the
  models need no protocol layer. Stage 3: rows carrying reactions, quote,
  edited and readers; menu entries for own, others', unsent, failed,
  deleted and undecryptable messages; "+N"; quote texts; emoji search;
  replying, editing, cancelling and a failed reply; typing calls, names
  and lines; the actions; going to a quote (shown, after paginating, at
  the room's start, after 20 calls). Stage 4: the Rooms and People
  sections, the four orders (with no `lastActivity`), the remembered
  order; which messages swipe to reply, and the swipe's start, offset
  and threshold; when the button to the bottom shows. Stage 4, the
  library's part: the banner following pushed states (also after a
  recovery), nothing asked, the subscription ended at sign-out; the
  undecryptable row's text by `keysArriving`, also in a timeline already
  open; the person sheet (the profile replacing the chat's name and
  picture, a partial profile, a failure; copying; an invite succeeding,
  failing, one at a time; none in the Notes to self or a room); the
  top-level spaces. Presence: the dot and the line for each state, status
  message and age; the subscription following the loaded room list's
  direct chats (not the Notes to self), kept on other changes, replaced
  when they change, only in the foreground, ended at sign-out; the
  person sheet's fresh value, kept on a failure; hiding after
  `onUnavailable` or the error, with nothing read or set after; online
  and offline around sync, offline before the first sync when not
  sharing, the switch (on by default, in the background), idle after no
  input and online at the next, failures ignored.

Locally: run `scripts/build-ios-ffi.sh` once (or unzip CI's `MtrxFFI-ios`
into `apps/ios`), open `apps/ios/Mtrx.xcodeproj`, run on a simulator; for a
device, choose a team under Signing. From the command line:

```
cd apps/ios
xcodebuild build -project Mtrx.xcodeproj -scheme Mtrx \
  -destination 'generic/platform=iOS Simulator'
xcodebuild test -project Mtrx.xcodeproj -scheme Mtrx \
  -destination 'platform=iOS Simulator,name=<an iPhone>'
```

Simulator builds are signed ad hoc, Xcode's default there; no team is
needed. Do not turn signing off (`CODE_SIGNING_ALLOWED=NO`): an unsigned app
has no application identifier, and the Keychain refuses it ("A required
entitlement is not present"), so no account can be opened.

CI (`.github/workflows/ios.yml`): builds the package with the script, the
app (Release) for the simulator, runs the unit tests on the first iPhone
simulator, starts the app there (it must still run after 15 s, without a
crash report; the screenshot is the artifact `ios-smoke`), and uploads
`Mtrx-ios-simulator` (the zipped `.app`; install with `xcrun simctl install
booted Mtrx.app`). Passes; the screenshot shows the sign-in screen.

To do: signing in, the room list, the timeline, recovery and verification
tried against a server (only the models' unit tests and the sign-in screen
are checked so far; the CI start does not check which screen shows; the
stage 3 room was seen in the simulator with made-up items only);
opening images full size and
files (`mediaFile`); answering invites; reconnecting when the network
returns (`reconnect`); a build for devices (signing).

## 4. Android app

Jetpack Compose (Material 3) in `apps/android`, a Gradle project with a
version catalog (`gradle/libs.versions.toml`: AGP 8.10.1, Kotlin 2.1.21 with
its Compose compiler plugin, Compose BOM 2025.05.01; Gradle 8.14.3, JDK 17).
Application id `org.mtrx.app`, minSdk 26, target and compile SDK 36. Built
and tested by CI only; no Android SDK is used locally.

- `:mtrx-ffi`, an Android library: `scripts/build-android-ffi.sh` fills it
  with `src/main/jniLibs/{arm64-v8a,x86_64}/libmtrx_ffi.so` and
  `src/main/kotlin/uniffi/mtrx/mtrx.kt` (gitignored), next to the committed
  `org/mtrx/ffi/AndroidPlatform.kt`. Dependencies: JNA
  (`net.java.dev.jna:jna:5.17.0@aar`), `kotlinx-coroutines-android`,
  `androidx.annotation`, and the platform verifier's Kotlin part
  (`org.rustls:rustls-platform-verifier`, an aar from the crate's Maven
  repository on GitHub, at the version of `rustls-platform-verifier-android` the build
  reads from Cargo.lock). Consumer R8 rules keep `com.sun.jna.**`,
  `uniffi.mtrx.**` and `org.rustls.platformverifier.**`.
- `:app` (`org.mtrx.app`), only the ABIs the library is built for:
  - `MtrxApplication`: `AndroidPlatform.initialize` first, then
    `Mtrx(filesDir, cacheDir, secrets)`, push registration, the message
    notifications (channel `messages` created), the `Account` and the push
    handler; sync follows `ProcessLifecycleOwner` (start in the foreground,
    stop in the background). It runs in every start of the process, the
    push service's too (no `android:process`).
  - Push notifications (section 10), through UnifiedPush
    (`org.unifiedpush.android:connector` 3.3.5: `PushService`,
    `UnifiedPush.register`, `unregister`, `getDistributors`,
    `getSavedDistributor`, `saveDistributor`, `resolveDefaultDistributor`).
    `model/Push.kt`: `PushRegistration`, told by the `Account` when a
    session opens (sets the pusher again on the saved endpoint, then picks
    the distributor: the saved one, else the only or the system's default
    one, else Settings offers the installed ones; and registers) and
    before it signs out or is forgotten (`deletePusher`, the connector's
    `unregister`, then `logout`; a failed sign-out registers again); a
    new endpoint replaces the pusher (`setPusher` with the endpoint as
    pushkey, the gateway from UnifiedPush's discovery at the endpoint's
    origin: `GET <origin>/_matrix/push/v1/notify` answering
    `{"unifiedpush":{"gateway":"matrix"}}`, else UnifiedPush's gateway;
    unreachable counts as one; `appId` `org.mtrx.app`, `Build.MODEL`, the
    locale's tag), deleting the earlier one's; unregistered by the
    distributor deletes it. Its state (`Off`, `NoDistributor`, `Choose`,
    `Registering`, `Registered`, `Failed`) and the switch are flows for
    Settings; DataStore `push` (`enabled`, on by default and kept at
    sign-out; `endpoint`; `permission_asked`), `model/PushPreferences.kt`.
    `push/MtrxPushService.kt` (the connector's service, not exported):
    endpoints, failures and unregistrations to `PushRegistration`; each
    push to `PushHandler` in a coroutine of the service, which keeps
    itself started (allowed while the distributor raises the app; logged
    otherwise) and holds a wake lock of 25 s while pushes run, then stops.
    `push/PushHandler.kt`, a plain class: parses the gateway's JSON
    (`room_id` and `event_id`; a push with only counts clears all message
    notifications when `unread` is 0; garbage is ignored), then
    `Mtrx.notification` within 20 s: nil posts nothing; `NotLoggedIn` unregisters; another error, the timeout or the
    service stopping first posts "New message"; nothing for the room open
    on screen while the app is in the foreground (`Account.foregroundRoom`,
    set by the room screen). `push/MessageNotifications.kt`: a room's
    notification (tag the room ID, id 0) in channel `messages` (high),
    `MessagingStyle` with its last six messages of this process, the
    sender as `Person` (the avatar through `Images` while the app's session
    is open, waited for up to 3 s), the room's name as title unless
    direct, silent unless `noisy`; group `messages` with a summary from
    two rooms on; tap: `MainActivity` with `EXTRA_ROOM_ID`, which the
    `Account` keeps as `roomRequest` until the navigation opens
    `room/{roomId}`. A room's notification goes when its screen shows,
    when it is marked read, and when its `unreadNotifications` drops to 0
    in a `RoomListState`; all go at sign-out. Settings: "Notifications"
    above "Encryption", the switch "Push notifications" (off and fixed
    while no distributor is installed), then "Via <app>", "Registering…",
    "Several apps can deliver notifications" with the apps to choose,
    "Install a UnifiedPush app, such as ntfy, to get notifications while
    Mtrx is closed." with "Get ntfy" (`market://`, else F-Droid's page),
    "Registration failed: <reason>", and "Notifications are off in
    Android's settings" with a button to them; distributors and the
    permission looked at again whenever Settings resumes.
    `POST_NOTIFICATIONS` is asked when the switch is turned on and once
    after a sign-in. Debug builds allow plain HTTP to `10.0.2.2` and
    `localhost` (`src/debug`, a network security config), for the
    instrumented tests.
  - `model/Secrets.kt`: `FileSecretBackend`, one file per secret in
    `noBackupFilesDir/secrets` named by the SHA-256 of its key, encrypted
    with AES-256-GCM (random IV, the key as associated data) by a key that
    stays in the Android Keystore (alias `mtrx-secrets`). Backups and device
    transfer are off, as the key cannot follow the data.
  - `model/Account.kt`: the session holder (no Compose): `restore()` at
    start, sign-in, sign-out and `forget()`; state, room list and connection
    as `StateFlow`s set by the listeners (on the library's threads); session
    changes and sync under one mutex; subscriptions cancelled and closed at
    sign-out. Browser sign-in: `startOauthLogin`, the page in a Custom Tab,
    and `OAuthRedirectActivity` (intent filter for scheme
    `io.github.alex-robinson`; the path `/oauth` is checked in code, as
    Android ignores path filters without a host) calls `finish` and returns
    to `MainActivity`. Back from the browser without the redirect cancels.
    Encryption (section 6) too: `encryption`, as `subscribeEncryptionState`
    reports it (section 9; subscribed at the first `startSync`, the handle
    with the session's other subscriptions, cancelled and closed at
    sign-out; a state arriving after sign-out is dropped; never asked with
    `encryptionState()`), `recover` with its
    steps as a flow (in the account's scope, so leaving the screen does not
    stop it), our `devices`, and the verification: the request subscription
    is one of the session's; a request from another user is cancelled, one
    from our own device is offered (one at a time; declined while a
    verification runs).
  - `model/VerificationModel.kt`: a `Verification` and its subscription,
    held by the `Account` (outgoing or accepted), its state, `confirm`,
    `mismatch`, `cancel` in the account's scope; `close()` cancels it unless
    it ended, then closes both. `model/Encryption.kt`: the banner rule and
    the recovery's state (`InvalidInput` from `recover` is
    `Recovery.InvalidKey`: "The recovery key is not valid." under the
    field, the key kept).
  - `model/Spaces.kt` (section 7): `SpaceEntry` (a space, or "Other
    rooms") and `SpaceSelection`, held by the `Account`: the selected entry
    while the room list has it, saved through `SelectedSpaceStore`
    (`model/ViewPreferences.kt`: Preferences DataStore `view`, key
    `selected_space`; `androidx.datastore:datastore-preferences`); a saved
    one waits for a list that has it and is forgotten when a loaded list
    does not; cleared at sign-out (and `forget`). `RoomSortChoice`, also
    the `Account`'s: the order of the spaces' rooms (`RoomSort`: Default,
    A to Z, Z to A, Most recent), saved through `RoomSortStore` (key
    `room_sort`), kept at sign-out (the device's, not the account's).
    `rooms/RoomLists.kt`: the tabs' lists from each `RoomListState`
    (`chats`, `spaceRows` with the unread dot, `roomsOf`, and `sectionsOf`:
    a space's rooms and its direct chats apart, each `sorted`: by name with
    the locale's `Collator`, case aside; most recent by `lastActivity`, never
    active last; ties in list order).
  - `model/Images.kt`: avatars and image previews, decoded (downsampled)
    and kept in an in-memory LRU cache.
  - Presence (section 11), `model/Presence.kt`: `PresenceModel`, one per
    session, made by the `Account` and in `AccountState.SignedIn`,
    closed at sign-out. The `Account` runs the lifecycle under its lock:
    `foreground()` before `startSync()` (`presenceActivity(FOREGROUND)`,
    the session's first time after `setPresenceSharing`, waited for), and
    `background()` before `stopSync()` (`BACKGROUND`, waited for); a
    sign-out tells `SIGN_OUT` first, so `offline` precedes `logout` (a
    failed one comes back with the next `foreground()`); `forget` closes
    it first and tells nothing. These calls are chained, so they reach the
    library in order. Input: `MainActivity`'s `dispatchTouchEvent` and
    `dispatchKeyEvent`, and each change of the composer's draft (the soft
    keyboard is another window), call `Account.input()`, each `INPUT` (the
    library keeps the idle timer). Others: the subscription is for the
    people of the loaded room list's direct chats (`directPeople`; the
    open direct chat's person is one of them), debounced by 1 s (room
    lists come in bursts), replaced only when they change, ended in the
    background and at sign-out; dots only in Chats, not in a space's
    People section. `refresh(userId)` when the person sheet opens, into
    the map. Once the server does not share presence, nothing more is
    told either. "Share my presence": `SharePresence` (DataStore `view`,
    key `share_presence`), the `Account`'s, read before the first sync.
    The dot (`ui/PresenceViews.kt`): `Presence.dot`, its ring outside its
    size (12 + 2 + 2 dp), centred on the avatar's circle at 45°; 10 dp on
    the header's 32 dp avatar. The line re-renders each minute, with
    `presenceLine(presence, now)`.
  - Screens: sign-in (homeserver, then password or "Sign in with browser" as
    `supportsOauth` says), the home (`ui/HomeScreen.kt`, section 7: a
    bottom bar with Chats, Spaces and Settings, only on this screen; Chats
    the direct chats; Spaces a `HorizontalPager` of the spaces, with "Other
    rooms", and the selected entry's rooms, titled with its name, back or
    the arrow to the spaces; tapping the Spaces tab again also returns to
    them; a new selection, tapped or restored, pages right; the entry's
    page in two sections, Rooms and People (its direct chats; each left
    out while empty), in the order chosen in the top bar's sort menu, for
    every space; Settings the account, privacy ("Share my presence"),
    encryption and "Sign out" (after "Sign out?"); tab, page and scroll positions are the screen's saved
    state, so they are kept while a room, recovery or verification is
    open; rows with avatars, unread and mention counts, last activity,
    "Invited"; banners for offline and signed out by the server over all
    tabs), and a room's timeline (the room's name, and for a direct chat
    with someone else (not the Notes to self) the room's avatar beside it,
    both tapped for the person's info sheet, `timeline/PersonInfoSheet.kt`
    with `PersonInfoModel`, held by the timeline's view model: the chat's
    name and avatar, then the profile's where set (`userProfile`; a
    failure keeps the chat's); the user ID, selectable, with a copy button
    ("Copied" in the sheet's snackbar before Android 13); "Add to space…"
    while there are top-level spaces (`topLevelSpaces`), listing them with
    their avatars, back to the person with ← or back; tapping one invites
    (one at a time, a spinner on its row, in the actions' scope so leaving
    the room does not cancel it): "Invited to <space>." and back to the
    person, or the error's reason with the list kept, both in the sheet's
    snackbar; newest at the bottom, keyed
    by `TimelineRow.id`; day dividers, read marker, grouped bubbles, send
    states, images, state events summarised in a line as on the desktop;
    older events near the top; marked read while the newest message
    shows; for an event without its key "Decrypting…" while
    `keysArriving`, otherwise "Could not decrypt this message."
    (`undecryptableText`; `utdCause` not shown); away from the bottom, a round down-arrow button over the
    messages at the bottom right goes to the newest). Messages (section 8): the sender's name in
    the body text size above a group, the avatar beside its first bubble;
    holding a bubble opens a bottom sheet with the six common reactions
    (ours marked) and "+" for the full picker (`androidx.emoji2:
    emoji2-emojipicker`'s `EmojiPickerView` in a dialog, themed with
    AppCompat's DayNight, which its resources need), then Reply, Copy,
    Select text, Edit, Delete (after "Delete this message for everyone?"),
    Retry and Discard, as `timeline/TimelineRows.kt`'s `messageActions`
    allows; swiping a bubble left (a horizontal `draggable`, so vertical
    drags scroll and edge swipes stay the system's back) shows the reply
    icon and, past 64 dp when let go, replies as Reply does, for the
    messages Reply is offered for (`EventRow.canReply`); it springs back;
    reaction chips under the bubble (tap toggles, hold lists who);
    the reply's quote in the bubble (tap goes to the original); "(edited)"
    muted after the text; read receipts as up to three 16 dp avatars and
    "+N" on a line under the message at the right margin (tap lists
    them); a typing line above the composer (`subscribeTyping`, its
    handle cancelled and closed when the room is left); the composer's
    bar for a reply or an edit with ✕, an edit starting from `rawBody`
    and giving back the earlier text when it ends; `setTyping` on every
    change, one call at a time in order, and `false` when the room is
    left. The timeline's view model takes each action's row from the
    latest rows; sends and actions run in a scope of the model's own, not
    `viewModelScope`, which leaving the room cancels (a cancelled call
    aborts the library's task); the timeline is closed once the last
    typing notice is sent. Going to a quote's original calls `seek` (a
    new tap replaces the one under way) and scrolls once the rows have
    the row it returns. The room's `DraftKeeper` restores the draft (its
    reply found with `itemOf`), hears each change of the draft and flushes when the room is left, on send and when the app
    stops; leaving a room (the membership) keeps its draft. Encryption: the banner over
    Chats and Spaces (opens the Settings tab); the Settings tab with the
    account and "Encryption" (status lines, "Enter recovery key", "Verify
    with another device", or the note that another device is needed); the
    recovery key screen with its steps and progress; the verification
    screen (one for both directions; back cancels it while it runs); the
    prompt for a request, over any screen. Navigation Compose: `home`,
    `room/{roomId}`, `settings/recovery`, `settings/verification`. Icons
    the core set lacks (spaces, reply, sort) are drawn from their Material
    paths (`ui/Icons.kt`).
  - Unit tests (JVM) on fakes of the library's objects (subclasses made with
    `NoHandle`): secrets, the account's sync and sign-out, timeline rows,
    the timeline's view model; messages (section 8): the rows' reactions,
    quotes, edits and readers ("+N"), the menu's entries for others',
    our, unsent and failed messages, replying and editing in the
    composer, typing both ways and on leaving the room (a send under way
    still completes), the typing line's texts, and going to a quote's
    original (loaded already, after paginating, arriving after the call,
    at the room's start, after 20 pages); swiping to reply only to sent
    messages, and no reply to one not sent; the encryption state as
    pushed (subscribed once sync starts, the banner going after a recovery
    or a verification without asking, dropped after sign-out) and the
    banner rule, the undecryptable row's text for both values of
    `keysArriving`; the person's info sheet (a header's person only for
    someone else's direct chat, the profile replacing the chat's name and
    avatar where set, a failure keeping them, closing it stops the
    loading; an invite succeeding, also after leaving the room, failing
    with the list kept, one at a time) and the top-level spaces; the
    recovery's steps and errors (an invalid key), the verification model
    (states, confirm, mismatch, cancel, close), and verification requests
    (another user's cancelled, ours accepted or declined); the Chats and
    Rooms lists from a `RoomListState` like the library's integration test
    (two top-level spaces, a nested one, direct chats, invites), a space's
    Rooms and People sections and the four orders (names in either case,
    ties, rooms never active), the selected space (saved, restored at the
    next launch, forgotten when gone or at sign-out), and the chosen order
    (saved, restored, a choice before the saved one is read wins, kept at
    sign-out). Push (section 10): registration (the only distributor
    saved, the system's default taken, several offered and one chosen,
    none until one is installed; the endpoint becoming the pusher with
    the gateway at its origin; the pusher set again at each start, an
    endpoint before the session set when it opens, a changed endpoint
    replacing the pusher; turning off deleting it and unregistering,
    remembered at the next start, and on again registering; an endpoint
    after turning off ignored; failures; unregistered by the
    distributor; a push without session), at sign-in, at a restore, at
    sign-out (pusher deleted, then unregistered, then logout), after a
    failed sign-out and at `forget`; parsing pushes (with and without
    `event_id`, counts, garbage) and the handler (posted, nil, not to
    notify, the room on screen, an error or 20 s as "New message",
    signed out, a count of 0); the notification's title and sound; the
    room on screen only in the foreground; clearing on opening, on
    `markRead` (not when it fails) and when a room's count drops to 0;
    a tapped notification's room; the Settings switch and lines.
    Presence (section 11): the subscription following the loaded room
    list's direct chats (not the Notes to self), kept on other changes,
    replaced when they change (once for a burst), only in the
    foreground, ended at sign-out; the person sheet's fresh value, kept
    on a failure; hiding after `onUnavailable` or `PresenceUnavailable`
    (also from telling ours), with nothing read or told after, and shown
    again in the next session; the foreground told before sync starts
    (after "Share my presence", once), the background before it stops,
    the sign-out before `logout` (and the foreground again when it
    fails), nothing at `forget`, input passed on in the foreground, the
    switch (saved, kept at sign-out), failures ignored; the Settings
    footnote.
  - Instrumented test (`androidTest`, `PushEndToEndTest`), against Synapse
    on the host (`http://10.0.2.2:8008`): registers Bob and Alice with
    the shared-secret admin API, signs Bob in through the app's `Account`
    (syncing) and Alice in a second `Mtrx`; Alice makes an encrypted room
    over HTTP inviting Bob, who joins over HTTP; the pusher is set through
    `PushRegistration.onNewEndpoint` (`http://10.0.2.2:9/upTEST?up=1`) and
    listed with `eventIdOnly`; Alice's mention, as the gateway's JSON,
    goes to the app's `PushHandler`; the posted notification (tagged with
    the room) has her text as typed and her as the person; opening the
    room clears it; then a push for Bob's own message posts nothing (his
    sending reads the room, which clears a notification while he syncs,
    so this comes after the clearing).
- Build locally, with the Android SDK and NDK, Rust's two Android targets
  and cargo-ndk: `scripts/build-android-ffi.sh`, then in `apps/android`
  `gradle :app:assembleDebug` (Gradle 8.11.1 or later, or a wrapper made with
  `gradle wrapper --gradle-version 8.14.3`; `local.properties` or
  `ANDROID_HOME` for the SDK). Tests: `gradle :app:testDebugUnitTest`.
- CI (`.github/workflows/android.yml`, Ubuntu): the library and bindings
  with the build script, then the debug APK, artifact `Mtrx-android-debug`,
  then the unit tests and the test APK, then Synapse (`testserver/run.sh`),
  then on an x86_64 emulator (API 34) `scripts/android-smoke.sh`: installs
  and starts the APK, checks that the sign-in screen shows, the process
  lives and the log has no crash; screenshot, UI tree and log as artifact
  `android-smoke`; and `gradle :app:connectedDebugAndroidTest`, the
  instrumented test above (reports as `android-reports` on failure).

Remaining:

- Each CI run signs with a new debug key, so an update needs the previous
  build uninstalled; a shared debug keystore (in the repository or a CI
  secret) fixes that.
- Run only to the sign-in screen (emulator, CI); signing in, the room list,
  the timeline, recovery and verification are not yet tried on an emulator
  or device (only their models' unit tests); nor are the messages'
  menus, sheets and the emoji picker, swiping to reply, the button to the
  newest message, the Settings tab, "Decrypting…", and the person's info
  sheet with its invites, nor presence (dots, lines, the switch, idling).
  Push notifications are tested on the emulator
  with the push handler called directly; registering with a real
  distributor (ntfy), the push service, the Settings section, a tapped
  notification and the summary are not yet tried. Not done: sending
  images, opening files, release signing.

## 5. Builds and CI

- [x] Icons: `cargo xtask icons` makes the iOS icon (full bleed, opaque)
      and sign-in logo, the Android adaptive icon (the square's colour
      behind the bubble, which fits the safe zone) and sign-in logo, and
      the website's logo from `crates/mtrx-app/assets/logo.svg`; they are
      committed. An xtask test fails when the committed vector files differ
      from what it makes.
- [x] `scripts/generate-ffi-bindings.sh`: host build, Swift and Kotlin into
      `target/bindings/` (checks the API goes through UniFFI). Swift
      bindings compiled and run against the macOS library locally.
- [x] `scripts/build-ios-ffi.sh`, `scripts/build-android-ffi.sh` (profile
      `mobile`: release with LTO; `--debug` for a quicker build). Generated
      files are gitignored. The bindgen binary links the library, so each
      script also builds mtrx-ffi once for the host.
- [x] `.github/workflows/mobile-ffi.yml`: iOS (xcframework, the package
      built for the simulator, a simulator executable linked against it) and
      Android (cargo-ndk with the runner's NDK; checks the JNI entry point).
      Both pass.
- [x] Tests: conversions and the runtime (unit); sign-in, room list,
      timeline, image, send and sign-out; recovery (a new device reads an
      encrypted message after the recovery key, in its open timeline); and
      emoji verification between two devices, after which the new one reads
      history, also in its open timeline; the rooms by space (nested spaces,
      a direct chat with a space member, invites in and outside spaces);
      messages between two people (a reply and its quote; a reaction by
      each, counted and named on both sides, and removed again; an edit,
      marked edited; a deletion; typing, by name; a read receipt moving to
      the message read, with the reader's name); a send the server refuses,
      retried and then discarded, after which the next message goes;
      stage 4: the encryption state through the listener (the recovering
      device sees `keysArriving` true, then verified with recovery enabled
      and no keys arriving; the verified device sees `deviceVerified`
      turn true), someone's profile, and an invitation to a space, which
      shows in the invitee's room list; push (section 10): a pusher set,
      listed, set again (replaced) and deleted (twice), and a second
      `Mtrx` on Bob's data reading Alice's messages in an encrypted room
      while Bob's app syncs (decrypted, a mention and a plain message
      that notify, Bob's own message nil, a missing event an error, and
      Bob's app still decrypting afterwards); presence (section 11):
      Alice sets online with a status message and Bob reads it, she sets
      unavailable and Bob's subscription gets the change at its next read
      (unavailable, not offline: the library kept it past Synapse's 30 s),
      offline read back, and a user Bob shares no room with refused by
      Synapse as that user's error, not as presence unavailable: all
      through the bindings against Synapse (`cargo test -p mtrx-ffi --test
      integration -- --ignored`, in the Linux workflow).

## 6. Stage 1: encryption

A new device has no keys for earlier messages, so encrypted rooms show
nothing readable until the user enters the recovery key or verifies the
device with another one (the desktop app, say). Both apps add the same four
things; the library part is done and tested (section 5).

**State and banner.** (Stage 4 replaces the asking below with a
subscription, section 9.) `Session.encryptionState()` returns
`EncryptionState { deviceVerified, recovery }`. There is no subscription
(the core has no stream for it): ask (a) when the room list first reports
`loaded == true` after `startSync()`, (b) after `recover` returns, (c) after
a verification reaches `done`, (d) when the app returns to the foreground.
Keep the result in the session model (iOS `SessionModel`, Android
`Account`). Show a banner at the top of the room list when
`!deviceVerified || recovery == incomplete`; not while `recovery ==
unknown` (no sync yet: ask again at the next moment above). Text: "Verify
this device or enter your recovery key to read encrypted messages."; a tap
opens the encryption settings.

**Settings, section "Encryption".** Status lines: "This device: verified /
not verified", "Recovery: set up / not set up / not unlocked on this
device" (`enabled` / `disabled` / `incomplete`). Two actions while the
banner would show:
- "Enter recovery key" (unless `recovery == disabled`): a screen with a
  multi-line text field (monospaced, no autocorrection or capitalisation;
  spaces are fine) and a Recover button calling
  `recover(recoveryKey:listener:)`. The listener's steps, in order:
  `checkingKey` ("Checking the key"), `importingIdentity` ("Verifying this
  device"), then `downloadingKeys(done, total)` once per room (a progress
  bar, "Fetching keys: done of total rooms"; there may be none). It can
  take a minute with many rooms. A mistyped key, or one of another
  account or an older backup, fails at `checkingKey` with
  `MtrxError.InvalidInput("The recovery key is not valid")`: show the
  reason under the field and keep the key for correcting. Other errors are
  `MtrxError.Other(reason)`: "Could not recover." with the reason below. On
  success go back to Settings and ask `encryptionState()` again.
- "Verify with another device" (while `!deviceVerified`):
  `requestSelfVerification()` returns a
  `Verification`; open the verification screen with it. (`devices()` lists
  our devices, `current` marking this one; if no other device is listed, say
  that one is needed and offer only the recovery key.)
`enableRecovery()` (set up recovery, returns the new key to show once) is
there, but not part of stage 1's UI: accounts set it up on the desktop.

**Verification screen**, one for both directions, driven by
`verification.subscribe(listener)` (the whole `VerificationState` each time;
it ends after `done` or `cancelled`):
- `waiting`: "Accept the request on your other device." (outgoing) or
  "Waiting for the other device…" (incoming, after accepting); a Cancel
  button → `cancel()`.
- `emojis(emojis)`: the seven emoji in order, in a grid (the `symbol` large,
  its `description` below), "Do they match the other device?", buttons
  "They match" → `confirm()` and "They do not match" → `mismatch()`.
- `confirmed`: "Waiting for the other device to confirm…".
- `sharingKeys`: "Finishing… keep the app open."
- `done`: "This device is verified." and a Done button; ask
  `encryptionState()` again.
- `cancelled(reason)`: "Verification was cancelled." with the reason; Close.
`state()` gives the current state without waiting. Keep the `Verification`
(and its `TaskHandle`) in a model, not a view: dropping it stops following
the protocol and the other device times out. In Kotlin `close()` both when
the screen is done.

**Incoming requests.** Right after `startSync()` (with the other
subscriptions), `subscribeVerificationRequests(listener)`; keep the handle
for the session and cancel it at sign-out. Requests arrive only while sync
runs. On `onRequest(verification)`: if `verification.otherUser() !=
session.userId()` (another person: not in stage 1) call `cancel()`.
Otherwise show "Another device wants to verify this one." with Accept →
`accept()`, then the verification screen (it starts at `waiting`; the other
device starts the comparison), and Decline → `cancel()`.

**Timelines.** Show `EventContent.unableToDecrypt` as a muted, italic row
"Could not decrypt this message." (grouped like messages; not hidden).
After a recovery or verification such rows turn into the messages by
themselves, in timelines already open: the listener gets the new items; the
apps need not reopen anything.

**Threading.** As for all calls (section 2): async calls are awaited from
Swift's or Kotlin's executor and listeners run on the library's threads,
so hop to the main thread before touching UI state. `recover`'s listener
is called during the call. Cancelling the awaiting Swift `Task` or Kotlin
coroutine cancels the call, so run `recover` and `confirm` from the model
(iOS: a `Task` the model owns; Android: `viewModelScope` or the `Account`'s
scope), not from a view's lifetime.

**Errors and tests.** No new `MtrxError` cases (iOS switches over them
exhaustively). App tests use fakes as before: subclass `Verification` with
`init(noHandle:)` / `NoHandle`, and `Session` for `encryptionState`,
`recover`, `requestSelfVerification` and `subscribeVerificationRequests`.

## 7. Stage 2: organising chats

The room list is one long list today. The desktop organises it by space: a
column of spaces, then the rooms of the selected space. The apps get the
same lists from `RoomListState`, computed by the library as the desktop
computes them (`mtrx_core::RoomsBySpace`); the library part is done and
tested (section 5).

**Layout.** A bottom bar with two tabs, Chats and Rooms; hidden while a
room is open. Tapping a room pushes the room screen; the system back
gesture returns to the list it came from (Chats, or the space's rooms),
which keeps its tab, page and scroll position.

**Chats.** Direct chats only, newest message first:
`rooms.filter { kind == direct }`, in the order of `rooms`. Nothing new is
needed: `rooms` is already ordered by recency (the SDK's: latest message or
recency stamp, rooms with unsent messages first). Keep that order; do not
re-sort by `lastActivity`, which is nil for some rooms (an invite, say).
Show `lastActivity` as the time. Invites to direct chats are there
(`invited`), and the Notes to self (`directUser.userId == session.userId()`).

**Rooms.** A horizontal pager of two pages, like the desktop's two columns.
- Left page, the spaces: `spaces` in order (top-level spaces, most
  recently active first, as the desktop's space column), each shown by its
  `RoomSummary` (look up `spaceId` in `rooms` for the name and avatar), with
  a dot while any of its `roomIds` has `unreadMessages > 0`. Then one entry
  "Other rooms" for `roomsOutsideSpaces`, left out while that is empty.
- Tapping an entry selects it and pages to the right; the right page shows
  the selected entry's rooms: `roomIds` (or `roomsOutsideSpaces`) mapped to
  their `RoomSummary`s, in that order, under the space's name. Swiping right,
  or back on Android, returns to the spaces.
- Remember the selected entry (a space ID, or "other rooms") across
  launches: in the app's preferences (UserDefaults / DataStore), deleted at
  sign-out. At start, while it is in `spaces`, select it and open on the
  right page; the first states may be a cache, so wait until `loaded` before
  deciding it is gone. If it is gone (the space was left, or it became
  nested in another), or when it disappears while shown, show the spaces
  page with nothing selected. Without a saved entry, open on the spaces
  page. (The desktop saves its space and room in `view.json` the same way.)

**What each list holds** (the desktop's rules, applied by the library):
- `spaces`: joined spaces in no joined space. Nested spaces are not
  entries of their own: their rooms are listed in the top-level space.
- A space's `roomIds`: every room below it, through nested spaces (by the
  spaces' `m.space.child` events); invites to rooms and spaces below it;
  direct chats with someone who is a joined member of it; rooms the user put
  into one of its categories on the desktop. Not the nested spaces
  themselves. A room can be in several spaces. The Notes to self is a direct
  chat with a member (the user), so every space lists it, as the desktop
  shows it in every space.
- Direct chats join a space's list once the space's members are loaded: an
  update shortly after the space first appears (a network call per space
  then, and again when its number of joined members changes).
- `roomsOutsideSpaces`: rooms no space lists, and invites to spaces outside
  them. No direct chats (they are all in Chats) and no joined spaces.
- Invites (`invited`) are in the lists they would belong to once joined;
  mark them as invites. They are answered on the desktop (no join call yet).
- Everything is in the order of `rooms`, most recently active first.

Differences from the desktop, by design: its first entry is Home, with
direct chats, invites and rooms in no space (and, as the user's synced
`at_home` settings say, the favourites of spaces or every room), where the
apps have "Other rooms"; and it splits a space's rooms into
sections (invites, favourites, the user's categories, rooms, people, low
priority), where the apps show one list in the order of `rooms`.

**Updates and tests.** Each `RoomListState` is whole: derive the lists from
each one (cheap at our sizes); rows are keyed by room ID. The new fields
have defaults, so `RoomListState(rooms:loaded:)` in the apps' fakes still
builds; tests of the lists pass `spaces` and `roomsOutsideSpaces`. No new
`MtrxError` cases.

## 8. Stage 3: messages

The timeline shows text and images today. Stage 3 brings what the desktop
does with messages: reactions, replies, edits, deleting, read receipts,
typing, and retrying a failed send. The library part is done and tested
(section 5); everything comes in the timeline's items, whole, as before.

**Rows and actions.** Every action on a message takes its row's
`TimelineRow.id` (`itemId`), which identifies also a message not sent yet
(it has no `eventId` until then). Pass the ID of a row from the latest
items; a row gone meanwhile (or a reply to a message not yet sent) fails
with `MtrxError.InvalidInput`. Server failures, and `retry` of a message
that did not fail or `discard` of one already sent, are `MtrxError.Other`:
show the reason in a short alert or snackbar.

**Messages.**
- Sender: the name (`sender.displayName`) in the normal
  text size above the first bubble of a group (it is too small today), and
  the avatar (`sender.avatarUrl`) beside the bubble, at its side (others'
  messages on the left), not in a corner of the screen. Not for our own.
- "(edited)", muted, after the text when `Message.edited`.
- Deleted (`EventContent.redacted`): a muted, italic "Message deleted".

**Holding a message** opens a menu (iOS: a context menu with a preview;
Android: a bottom sheet), offering only what applies:
- React: a row of common emoji (👍 ❤️ 😂 😮 😢 🎉, ours highlighted) and
  "+" for a full picker → `react(itemId, key)`.
- Reply → the composer replies (below).
- Copy (`Message.body`) and Select text (a sheet with the text selectable).
- Edit, when `TimelineEvent.editable` → the composer edits (below).
- Delete, when `own && eventId != nil`, after a confirmation ("Delete this
  message for everyone?") → `delete(itemId)`.
- For `sendState == failed`: Retry → `retry(itemId)`, Discard →
  `discard(itemId)`. Later messages wait until it is retried or
  discarded. `waiting` (offline) needs nothing: it is sent when the server
  is back.
React and Reply need a sent message: offer them once `eventId != nil`.

**Reactions** (`Message.reactions`, in the order first given): chips under
the bubble, each the `key` and its `count`; ours (`byUs`) highlighted with
the accent colour. Tap → `react(itemId, key)` (adds ours or removes it).
Long-press → a sheet listing `senders` (avatar and name; `name` may be nil
until known: show the ID's localpart).

**Replies.** `Message.replyTo` is the quote, drawn above the text inside
the bubble: a bar in the accent colour, the sender (`sender.displayName`)
in semibold, and up to three lines of `body`
(for `kind` image, video, audio or file: "Image", "Video", "Audio" or
"File" and the name). While `state == loading`: "Loading…" (the library
fetches the original, also when it is far back; the quote updates by
itself). `failed`: "Could not load the message", not tappable. Tapping a
loaded quote goes to the original, as the desktop does: find the row whose
`event.eventId == quote.eventId`; if it is there, scroll to it and
highlight it for two seconds; if not, `seek(eventId)` loads older events
(up to 20 pages of 50, or to the room's start) and returns the row's ID,
to scroll to once the rows have it (they arrive separately). It fails
with `EventNotFound`, "The original message was not found in the room's
history". Show a spinner while it looks; cancelling the call stops it.

**Composer.**
- Reply: a bar above the field, "Replying to <name>" and the first line of
  the message, with ✕ to cancel; Send → `reply(itemId, text)` instead of
  `sendText`, then the bar closes.
- Edit: the field filled with `Message.rawBody` (the Markdown we wrote),
  a bar "Editing" with ✕ to cancel; Send → `edit(itemId, text)`. Only one of replying and editing at a time.
- Typing: `setTyping(true)` when the text changes and is not empty,
  `setTyping(false)` when it is emptied, sent, or the room is left (the
  library throttles: call it on every change).

**Typing line.** `subscribeTyping(listener)` when the room opens (cancel
the handle when it closes) gives the names of the others typing, on every
change (nobody at first). One muted line just above the composer, its
height kept while empty so the timeline does not jump: "Anna is typing…",
"Anna and Ben are typing…", "Anna, Ben and 2 others are typing…".

**Read receipts.** `TimelineEvent.readBy`: the others whose receipt is on
this event, i.e. who read up to here (never us; receipts on state events
are moved to the message before, so folded state lines lose none). Small
round avatars (16 pt/dp) at the right margin of that message's row, at most
three, then "+N"; tap → a sheet listing them (avatar and name). Names and
avatars may arrive in a later update. `markRead()` as today.

**Threading and errors.** As before: the actions are async, run them from
the model (iOS: a `Task` the model owns; Android: `viewModelScope`), not
from a view's lifetime, so leaving the screen does not cancel a send.
`TypingListener` is called on the library's threads: hop to the main
thread. No new `MtrxError` cases.

**Tests.** The new record fields have defaults (`reactions: []`, `replyTo:
nil`, `rawBody: ""`, `editable: false`, `readBy: []`), so the apps' fakes
build unchanged. Fakes of `Timeline` subclass it (`init(noHandle:)` /
`NoHandle`) and override the actions to record their calls, and
`subscribeTyping` to keep the listener and call it. Worth testing in the
models: which menu entries show for own, others', failed and unsent
messages; reply and edit state in the composer;
the quote jump (found at once, after `seek`, not found); typing line
texts; reader overflow ("+N").

## 9. Stage 4: polish

The phone test of stages 1 to 3 went well. This stage fixes what it found
and adds what the user asked for. The library part (the encryption state
as a subscription, keys arriving, someone's profile, inviting) is done and
tested (section 5); the rest is in the apps, which do the same on both.

**Encryption state, pushed.** After the recovery key the banner stayed
for a while: the apps asked `encryptionState()` right after `recover`
returned, and parts of the state settle later. Now
`Session.subscribeEncryptionState(listener) -> TaskHandle` calls
`EncryptionStateListener.onUpdate(EncryptionState)` with the state now and
after every change: this device verified (by the recovery key, or a
verification), the recovery's status (`unknown` until the first sync),
keys arriving and done. Subscribe right after `startSync()` with the other
subscriptions (sync must have started once for the state to be known);
keep the handle for the session and cancel it at sign-out (Kotlin: and
close it). Keep the last state in the session model and drop the asking
of section 6 (after the room list loads, after `recover`, after a
verification, at the foreground, and Android's asking again while
`unknown`). `encryptionState()` stays for a first read where one is needed
before the listener's first call. The banner rule is unchanged: `!deviceVerified
|| recovery == incomplete`, not while `unknown`.

**Decrypting.** After a recovery, rooms showed "Could not decrypt" until
their keys came from the backup. `EncryptionState.keysArriving` is true
while message keys are being fetched from the key backup: while the
backup is turned on (after the recovery key, or a verification that brings
its key), while `recover` fetches the keys of the 20 most recently active
encrypted rooms, and while a room's keys are fetched as its timeline
opens (once per room and session, once the backup is on). Rows of
`EventContent.unableToDecrypt` say "Decrypting…" while `keysArriving` and
"Could not decrypt this message." otherwise; both muted and italic, as
now. The text follows the state in rows already shown, and rows turn into
their messages by themselves as the keys arrive. The SDK also fetches
single keys for undecryptable messages in the background without saying
so, so a row can still become its message after `keysArriving` turned
false. `TimelineEvent.utdCause` is not used in this stage.

**Bottom bar.** Three items: Chats, Spaces, Settings. "Rooms" is renamed
"Spaces" (same icon as now; it opens on the spaces page). Settings moves
into the third item: its screen as now (the account, "Encryption", sign
out), as the tab's root, with the recovery key and verification screens
pushed from it. Remove the old way in (iOS: the toolbar button and its
sheet; Android: the room list menu's Settings). The encryption banner's
tap selects the Settings tab. The bar hides while a room, the recovery key
screen or the verification screen is open; each tab keeps its page and
scroll position.

**Swipe to reply.** Dragging a message bubble to the left starts a reply
to it, as the menu's Reply does (the composer's reply bar opens and the
field takes the focus). Only on rows that offer Reply (sent messages:
`eventId != nil`; not state lines, deleted or undecryptable rows), our
own and others'. The bubble follows the finger to the left, at most 72
pt/dp; a reply arrow (iOS `arrowshape.turn.up.left`, Android
`Icons.AutoMirrored.Filled.Reply`) appears at the row's right edge, fading
in with the distance. At 56 pt/dp a light haptic tick; releasing beyond it
replies, releasing before it does nothing; the bubble springs back either
way. Only a drag that starts mostly horizontal (and to the left) counts:
vertical scrolling, holding for the menu, tapping a quote, a reaction or
an image, and the system's back gesture from the left edge work as
before.

**Direct chat header.** In a direct chat (`kind == direct` with a
`directUser` other than ours: not the Notes to self) the room screen's
title shows the room's avatar (`avatarUrl`, which is the person's when
the chat has none; initials as in the list), 28 pt/dp and round, left of
the name. Tapping the avatar or the name opens the person's info sheet.

**The person's info sheet.** At once it shows what the room list knows
(the chat's name and avatar); `userProfile(directUser.userId)` then fills in
the profile's name and avatar where set (a failure keeps the chat's, with
no message). From the top: the avatar (80 pt/dp, round), the name, the
user ID (secondary text, selectable) with a copy button beside it (copies
the ID; iOS: shows "Copied" for 2 s; Android: a "Copied" snackbar unless
the system shows its own clipboard notice, from Android 13), then "Add to
space…". That opens a list of the top-level spaces (`RoomListState.spaces`
in order, each with its `RoomSummary` name and avatar); tapping one calls
`invite(spaceId, directUser.userId)`, with a spinner on the row meanwhile: on
success "Invited to <space name>." and the list closes back to the sheet;
on failure the error's reason in an alert (iOS) or snackbar (Android),
the list stays. "Add to space…" is left out while `spaces` is empty. The
server refuses an invite to someone already in the space; its reason is
shown as any other.

**Scroll to the latest.** While the newest row of the timeline is not on
screen, a round button (40 pt/dp, a down arrow: iOS `chevron.down`,
Android `Icons.Filled.KeyboardArrowDown`; the bar's material or surface
colour, with a shadow) floats over the timeline's bottom right corner,
16 pt/dp from the edge and above the composer and typing line. It fades
in and out. Tapping it scrolls, animated, to the newest row.

**A space's page in two sections.** The page of the selected space's
rooms (or "Other rooms") shows two sections: "Rooms" (its entries with
`kind != direct`: rooms, and invites to rooms and spaces) and "People"
(`kind == direct`: its direct chats, the Notes to self among them). A
section without entries is left out, with its header ("Other rooms" has
no direct chats). The page's menu (iOS: a toolbar `Menu` with a "Sort"
picker; Android: the top bar's overflow menu with radio items) sorts
both sections:
- Default: the order of `roomIds`, as now (the SDK's recency).
- A to Z: by `name`, ignoring case and accents, in the user's locale
  (iOS `localizedStandardCompare`; Android `java.text.Collator` with
  strength `PRIMARY`).
- Z to A: the reverse of A to Z.
- Most recent: by `lastActivity`, newest first; entries without it (an
  invite, say) after the others.
The sorts are stable: ties keep the default order (also in Z to A). One
choice for all spaces, remembered across launches and kept at sign-out (a
view preference, not the account's): UserDefaults `spaceRoomsSort` /
DataStore `view` key `space_rooms_sort`, with `default`, `aToZ`, `zToA`,
`recent`. A search filters within the sections and keeps the names
starting with the text first, then the chosen order. Nothing new is
needed from the library: `RoomSummary` has `name`, `lastActivity` and
`kind`.

**Errors, threading and tests.** No new `MtrxError` cases. `invite` fails
with `InvalidInput` (a malformed ID), `RoomNotFound` (not a room we are
in) or `Other` (the server refused: show the reason); `userProfile` with
`InvalidInput` or `Other`. Both are async: run them from the model, as
before. `EncryptionStateListener` is called on the library's threads: hop
to the main thread. The new record fields have defaults (`keysArriving:
false`, `utdCause: nil`), so the fakes build unchanged; fakes of
`Session` override `subscribeEncryptionState` (keep the listener, call
it), `userProfile` and `invite`. Worth testing in the models: the banner
following pushed states (without asking); the undecryptable row's text
for both values of `keysArriving`, changing in a shown row; the sections
and each sort (case and accents, nil `lastActivity`, ties, Z to A's ties),
the choice saved and restored; the info sheet (the profile replacing the
chat's name and avatar, a failure keeping them), an invite succeeding
and failing; which rows can be swiped to reply.

## 10. Push notifications (Android)

The apps notify only while they sync. Push brings notifications while the
app is in the background or not running, through UnifiedPush: the user
installs a distributor (ntfy), which holds one connection for all apps;
the homeserver posts to a Matrix push gateway (ntfy.sh or a self-hosted
ntfy, at `<ntfy>/_matrix/push/v1/notify`), which hands the post to the
distributor, which wakes the app. No Google services. The library part is
done and tested (section 5), and so is the Android app's (section 4; end
to end on the emulator in CI, not yet with a real distributor); iOS
(APNs) waits for an Apple developer account and uses the same calls
(below).

**Registering.** Library `org.unifiedpush.android:connector` (the current
3.x; check its docs for the exact names, which changed from 2.x). At
start, once signed in and with notifications on: pick the distributor
(the saved one, else the only or default one; several → let the user
choose, see Settings) and `register`. The connector calls back on the
app's thread:
- New endpoint (`onNewEndpoint`; at the first registration and whenever
  the distributor changes it): `session.setPusher(gatewayUrl, pushkey,
  appId, appName, deviceName, lang)` with `pushkey` = the endpoint URL
  (`https://ntfy.sh/upAbC…?up=1`), `gatewayUrl` = the endpoint's origin
  + `/_matrix/push/v1/notify` (UnifiedPush's discovery: a `GET` on it
  answers `{"unifiedpush":{"gateway":"matrix"}}` when that server is a
  gateway; otherwise use `https://matrix.gateway.unifiedpush.org/_matrix/push/v1/notify`),
  `appId` = `org.mtrx.app`, `appName` = "Mtrx", `deviceName` =
  `Build.MODEL`, `lang` = `Locale.getDefault().toLanguageTag()`. The
  library asks for the `event_id_only` format: the server sends no
  message text through the gateway. Save the endpoint (DataStore `push`,
  key `endpoint`); call `setPusher` again at each start while signed in
  (it replaces the pusher, so it is safe to repeat) and after a sign-in.
- Unregistered (`onUnregistered`) or turned off: `deletePusher(endpoint)`
  (succeeds if there is none), forget the endpoint.
- Registration failed: the Settings state below; retry at the next start.
- Signing out: `deletePusher`, then the connector's `unregister`, then
  `logout` (the server also drops a signed-out device's pushers).
`pushers()` lists the account's pushers (all devices), for Settings and
tests.

**The push.** The distributor delivers the gateway's body, the Matrix
push notification (bytes of UTF-8 JSON), for example:
```json
{"notification": {"event_id": "$x7…", "room_id": "!abc:example.org",
  "counts": {"unread": 2}, "prio": "high",
  "devices": [{"app_id": "org.mtrx.app", "pushkey": "https://ntfy.sh/upAbC…?up=1",
               "data": {"format": "event_id_only"}}]}}
```
Read `notification.room_id` and `notification.event_id`. A push without
`event_id` only updates the count (the server sends one when the user
reads on another device): when `counts.unread` is 0, cancel all of the
app's message notifications. Ignore what does not parse.

**The receiver.** For each push: `mtrx.notification(roomId, eventId)`.
It restores the saved session from the data directory and the secrets
without syncing, fetches the event (a short sliding sync, else
`/context`), decrypts it (if the key is missing, it runs the encryption
sync for up to about 6 s), applies the account's push rules, and returns:
- nil: nothing to show (our own message, a muted room, push rules that do
  not notify, an ignored sender, a deleted event). Post nothing.
- `Notification`: post it. `body` is the text, or "Image",
  "Video", "Audio", "File", "Location", "Sticker", "Call",
  "Encrypted message" (the key did not come in time), "Invited you",
  "New message", "New event". Text is the message's plain body as sent: a mention
  typed in Mtrx reads `@bob:example.org` there, where the timeline shows
  the name.
- `MtrxError.NotLoggedIn`: signed out; unregister. `Other(reason)`
  (network, the event not found): post "New message", titled "Mtrx",
  as the room's notification (tag `roomId`), so a push is not lost; a
  later push for the room that works replaces it.
It takes from a fraction of a second to about 10 s. Run it in a
coroutine the push service owns (connector 3.x delivers in a `Service`;
a 2.x `BroadcastReceiver` needs `goAsync()` and stays under 10 s), not on
the main thread, with a timeout of 20 s.

**Posting.**
- Channel "Messages" (`messages`, importance high), created at start;
  category `CATEGORY_MESSAGE`. `noisy` false (the push rules ask for
  no sound) → `setSilent(true)`. `highlight` is there for showing a
  mention apart; not needed now.
- One notification per room: tag = `roomId`, id = 0, `MessagingStyle`
  with the room's messages so far (keep the last few per room in memory
  or a small file; a new process starts from the one at hand), the
  sender as `Person` (`sender.displayName`; an avatar only when the app's
  `Session` is open: `session.avatar(sender.avatarUrl, 96)`), the
  conversation title `roomName` unless `direct`, `timestamp` as `when`.
  Grouped (`setGroup("messages")`) with a summary notification from two
  rooms on.
- Tap: a `PendingIntent` to `MainActivity` with the room ID, which
  navigates to `room/{roomId}` (signed in) and cancels the notification.
- Not for the room open in the foreground: the `Account` keeps the open
  room's ID (set by the room screen, cleared when it leaves); skip
  posting when it matches and the app is in the foreground
  (`ProcessLifecycleOwner` started).
- Cleared when the room is read: when the room screen calls `markRead`,
  and when a room's `unreadNotifications` drops to 0 in a
  `RoomListState` (read on another device, while the app syncs), and on
  a count push with `unread` 0 (above).
- Android 13+: `POST_NOTIFICATIONS`, asked when the user turns
  notifications on (and once after the first sign-in).

**Settings.** A "Notifications" section above "Encryption":
- A switch "Push notifications" (DataStore `push`, key `enabled`, on by
  default; kept at sign-out). On → register as above; off → unregister
  and `deletePusher`.
- Under it, the state: "Via ntfy" (the distributor's app label) when
  registered; "Several apps can deliver notifications" with a choice
  (`getDistributors`, `saveDistributor`, then register) when more than
  one is installed and none chosen; "Install a UnifiedPush app, such as
  ntfy, to get notifications while Mtrx is closed." with a button to
  ntfy's store page when none is installed (`getDistributors` empty;
  look again at each start and when Settings shows); "Notifications are
  off in Android's settings" with a button to them when the permission
  or the channel is off; "Registration failed: <reason>" after a
  failure.

**Processes and threads.** Android starts the connector's service in the
app's process, starting the process if it is not running;
`MtrxApplication.onCreate` runs first, so `AndroidPlatform.initialize`
and the one `Mtrx` are there (do not declare `android:process`; the
library would also work in a process of its own, but the app's start-up
runs in every process). `Mtrx.notification` uses a client of its own on
the same store as the app's `Session`, which may be open in this process
(syncing or not) or in no process; the SDK's cross-process locks keep
their writes apart (each client takes them under its own name). Do not
start sync for a push. Calls run one at a time in a process (the library
queues them), on the library's threads; hop to the main thread only for
UI state, and post notifications from any thread. The `SecretBackend`
is called from the push too: the Keystore key must be usable while the
device is locked (it is: the app's key needs no user authentication), so
pushes work on a locked phone after the first unlock.

**Testing in CI.** No real transport is available there, so the test
plays distributor and gateway. An instrumented test (`androidTest`, run
by `android.yml` on the emulator after the smoke test) with Synapse
started on the runner as in `linux.yml` (`testserver/run.sh`; the
emulator reaches the host's port as `http://10.0.2.2:8008`, server name
`localhost`):
1. Register two users with Synapse's shared-secret admin API (HMAC-SHA1
   of `nonce\0name\0password\0notadmin` with `mtrx-test-shared-secret`, as
   `mtrx_core::test_support::register`).
2. Sign Bob in through the app's `Account` (the app's data directory, so
   the push handler finds the session); sign Alice in with a second
   `Mtrx` on a temporary directory.
3. Make the room with the client-server API (the bindings cannot make
   rooms): Alice logs in over HTTP, creates a room inviting Bob, Bob
   joins over HTTP with a token of his own (another login).
4. Register a pusher the way the app does, with a fake endpoint
   (`http://10.0.2.2:9/upTEST?up=1`), and check `pushers()` lists it with
   `eventIdOnly`.
5. Alice sends `@bob:localhost …` (`Timeline.sendText`, which adds
   `m.mentions`); take its event ID from her timeline.
6. Deliver the push: send the connector's message intent to the app,
   with the JSON above (`room_id`, `event_id`, the fake `pushkey`) as the
   message bytes. The connector checks the token it saved when
   registering, so either register first with a stub distributor (a
   test app or component answering the connector's registration
   broadcast with the fake endpoint) or, if that proves brittle, call
   the handler that the connector's callback delegates to with the same
   bytes (keep that handler a plain class for this reason).
7. Assert within 30 s that `NotificationManager.activeNotifications`
   has a notification tagged with the room ID whose text is the message
   and whose person is Alice; then that a push for Bob's own message
   posts nothing; then open the room (or `markRead`) and assert the
   notification is gone.
`FakeMtrx` (it implements `MtrxInterface`) needs `override suspend fun
notification(roomId: String, eventId: String): Notification?`
before the unit tests compile again; Android CI on `mobile` fails until
then. Unit tests (JVM) on fakes: parsing pushes (with and without
`event_id`, garbage), what to post for each `Notification` (nil,
silent, the open room skipped, errors as "New message"),
the endpoint → `setPusher` arguments (gateway from the origin), the
Settings states, and clearing on read.

**iOS later.** The same calls: `setPusher` with a Sygnal gateway and the
APNs token as `pushkey` (an optional default payload for
`mutable-content` will be added as a defaulted argument), and
`Mtrx.notification` from a notification service extension, sharing the
data directory and Keychain through an app group: the extension is the
separate process the locks are made for.

## 11. Presence

Whether people are online, idle or away, and sharing ours. The library
part is done and tested (section 5); the rest is in the apps, which do the
same on both. The desktop follows the same rules, with "Share my presence"
under General, idle after 5 minutes without input (its window focused or
not), and offline when its window closes.

**How the library gets it.** Sync does not carry presence: simplified
sliding sync (MSC4186) has no presence extension, and the SDK delivers
none. So the library reads the presence endpoint: `presence(userId)` once,
`subscribePresence(userIds, listener)` for a set, reading each user about
once a minute (all at once at the start, then spread over the minute)
while the handle lives. Ours is set from what the app does
(`presenceActivity`, below), and the library sends it again every 20 s
until it is set again, set offline, or the `Session` is released (a send
that failed too): servers drop a presence that no
client refreshes (Synapse after 30 s, and its sliding sync does not
count). So a suspended or killed app goes offline by itself about 30 s
later.

**The dot.** On the avatar's bottom right, with a 2 pt/dp ring in the
background colour: `Presence.dot`, from the library: green for `online`,
amber for `idle` (`unavailable`), none for `offline` or unknown. 12 pt/dp on the Chats rows' avatars (44), 9 on the
direct chat header's (28), 20 on the person sheet's (80). In Chats only on
direct chats (`kind == direct`, the `directUser`'s presence; not the
Notes to self). Accessibility label `presenceDotLabel(dot)`: "Online" or
"Idle".

**The line under the name**, in the direct chat header (under the title)
and on the person sheet (under the name, above the user ID): secondary
text, one line, from the library (`Presence.line` as sent, and
`presenceLine(presence, now)` each minute, as ages change):
- the status message, if set;
- else `online`: "Active now"; `unavailable`: "Idle";
- else `offline` with `lastActive`: "Last active 2 hr ago" (English, as
  iOS's `RelativeDateTimeFormatter` `.abbreviated` writes it in British
  English: "just now" under a minute, "5 min ago", "1 day ago", "3 days
  ago", "2 wk ago", "4 mo ago", "1 yr ago");
- else nothing (the line is left out).
It follows the presences as they arrive.

**Reading others.** One `subscribePresence` per session, for the
user IDs of the direct chats' `directUser` in the room list (without our own),
started after the first loaded `RoomListState`, and replaced (subscribe
anew, cancel the old handle) only when that set of IDs changes, not on
every room list update. Cancel it when going to the background and
subscribe again in the foreground; cancel it at sign-out. Keep the last
map in the session model: the dots and the header's line come from it.
The person sheet also calls `presence(userId)` when it opens (fresher, and
for someone not in the map), keeping the map's value on failure. A user
missing from the map shows nothing. `PresenceListener` is called on the
library's threads: hop to the main thread.

**When the server does not share presence**, everything is hidden for the
rest of the session (until sign-out or the next launch): at the first
`onUnavailable()`, or the first call failing with `PresenceUnavailable`,
drop the map,
cancel the subscription and show no dots or lines. Other failures concern
one user (Synapse refuses the presence of someone we share no room with)
or the network: that user shows nothing, nothing else changes. Synapse
with presence turned off answers "offline" with nothing else for
everyone, which the rules above show as nothing.

**Our own presence**, while signed in and "Share my presence" is on, is
the library's: the app tells it what it does, with
`presenceActivity(activity)`, and the library sets ours on each change
only:
- `foreground` at sign-in, at start, and when the app comes to the
  foreground (iOS `scenePhase` `.active`; Android `ProcessLifecycleOwner`
  started), before `startSync()`, waited for: `online`.
- `input` for touches and key presses anywhere in the app (iOS a
  window-level gesture recognizer that does not cancel touches; Android
  `MainActivity.dispatchTouchEvent` / `dispatchKeyEvent`): after 5
  minutes in the foreground without input the library sets
  `unavailable`, and the next input `online` again.
- `background` when going to the background (iOS `scenePhase`
  `.background`, in a `beginBackgroundTask` so that it completes before
  suspension; Android `ProcessLifecycleOwner` stopped), before
  `stopSync()`, and `signOut` before `logout()`, both waited for:
  `offline` (until the next `foreground`, should signing out fail).

Each counts from the call: the apps make them in order. Failures are
logged and otherwise ignored, except `PresenceUnavailable` (hide
presence); the library sends a failed one again with its refreshes. No
status message is set (one set elsewhere, say `mtrx presence --set online
--status …`, is cleared).

**Settings.** A switch "Share my presence" in a "Privacy" section just
above "Encryption", on by default, a device
preference kept at sign-out (iOS UserDefaults `sharePresence`; Android
DataStore `view`, key `share_presence`). Footnote: "Others see when you
are online or idle." It is passed with `setPresenceSharing(on)` before the
session's first `foreground`, and at each change: off, the library sets
`offline` at once and stays so (at start, before `startSync()`, so that
sync does not report us online either); on, `online` at once if in the
foreground. Others' presence still shows. When
the server does not share presence, the footnote reads "This server does
not share presence." and the switch is disabled.

**Errors, fakes and tests.** `presence` and `subscribePresence` fail
with `InvalidInput` for a malformed ID; `presence`,
`presenceActivity` and `setPresenceSharing` with `PresenceUnavailable`
(above) or `Other`. Fakes of `Session`
implement `presence`, `presenceActivity`, `setPresenceSharing` and
`subscribePresence` (keep the listener, call it). Worth testing in the
models: the lifecycle's calls (foreground before `startSync`, after the
sharing setting the first time; background before `stopSync`; sign-out
before `logout`; input passed on; the switch), the subscription replaced
only when the direct chats' users change, and hiding after
`onUnavailable` or the error. The idle timer, the states set and the
line are the library's, tested there.

## Later

- The room list can show a room before its `m.room.encryption` event has
  synced (Synapse creates a room's state one event at a time, and sync can
  deliver the room in between), so a message sent in that window goes out
  unencrypted. The FFI tests wait for `RoomSummary.encrypted`. A core issue
  to look at (for the desktop too): sending should wait until the room's
  encryption state is known, rather than the apps working around it.

- Push notifications on iOS (APNs through a Sygnal gateway, a
  notification service extension calling `Mtrx.notification`; needs an
  Apple developer account).
- Sending images; mentions with `@` completion; forwarding.
- Several accounts; Face ID / biometric lock; verifying other people;
  setting up and resetting recovery from the apps.
- Timeline diffs instead of snapshots; log to OSLog and logcat.
- Signing and store distribution (TestFlight, Play internal testing).
