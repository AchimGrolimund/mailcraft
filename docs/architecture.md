# MailCraft architecture

> **Last reviewed:** 2026-10-11 · **Last updated:** 2026-10-12 · **Change:** patch (`html` makes the line a message list shows) · **Target:** Microsoft Outlook (Microsoft 365)

How MailCraft is built today. Rules for working in the code are in [`AGENTS.md`](../AGENTS.md). The
shape — layers, "everything is a command", the swappable egui front end, the control channel, MCP,
`xtask` gates — is WordCraft's, applied to mail.

## Layers

13 crates, 3 apps and `xtask` (`cargo xtask stats` prints lines and tests per crate). Layering is
enforced by `cargo xtask layers`: a crate may only depend on crates in lower layers (one exception
inside L3: `net` drives `proto`), and nothing below L6 depends on egui, eframe, winit or rfd.

| Layer | Crate | Job |
|---|---|---|
| L0 | `time` | Dates and times without a clock dependency: proleptic Gregorian maths, weekdays, ISO weeks, RFC 5322 and ISO 8601 text, list and group labels; `now()` and the local zone offset are the only calls that ask the system (or the browser) |
| L0 | `control-key` | Per-window control-port key files shared by the app and the MCP bridge |
| L1 | `model` | The mailbox: accounts, folders, messages, contacts, calendars, events with recurrence, tasks, categories, rules, signatures; edit primitives (`move_message`, `delete_folder`…), conversations, the search query language, `repair()` for whatever a file held, and the sync data types |
| L1 | `html` | Hostile HTML mail → display blocks (`parse`), plain text (`to_text`) and the line a message list shows (`preview`, `text_preview`); plain text → blocks and HTML; entities; the safe-URL rule |
| L2 | `mime` | RFC 5322 / MIME reader and writer (`.eml`), encoded words, RFC 2231 parameters, charsets, mbox |
| L2 | `ical` | iCalendar reader and writer: events, to-dos, recurrence rules, alarms, attendees |
| L2 | `vcard` | vCard 2.1/3.0/4.0 reader, 3.0 writer |
| L3 | `proto` | IMAP4rev1 and SMTP clients over any `Read + Write` stream (no sockets, so they test against scripted transcripts and build for wasm) |
| L3 | `net` | TCP + TLS (rustls), sending through SMTP, two-way IMAP synchronisation, OAuth sign-in, MX lookup; empty on wasm |
| L4 | `engine` | `Session` (mailbox, selection, view, the message being written, undo), the **command registry** (`cmd/*.rs`), the message list (`list.rs`), the Outlook feature catalog (`catalog.rs`), the sample mailbox, file dispatch (`io.rs`) |
| L5 | `mcp` | MCP server: tools over the command registry; headless or bridged to the running app |
| L6 | `ui-egui` | The front end: title bar with search, ribbon, module bar, folder pane, message list, reading pane, composer, calendar, people, tasks, Backstage, dialogs, themes, control channel, headless `ui_shot` example |
| apps | `mailcraft` | Desktop app (eframe + wgpu; logging, window geometry, native dialogs, Send/Receive thread, control server) |
| apps | `mailcraft-cli` | `info`, `folders`, `list`, `show`, `import`, `export`, `convert`, `run`, `sync`, `commands`, `parity`, `mcp` |
| apps | `mailcraft-web` | Browser build (eframe web runner, WebGPU with WebGL2 fallback; browser storage, file picker, downloads) |

## Data flow

1. **Open:** the desktop app reads `<settings>/mailbox.mailcraft` (JSON) through `engine::io`;
   `Mailbox::repair` makes whatever the file held consistent. The browser reads the same JSON from
   its storage. Other files (`.eml`, `.mbox`, `.ics`, `.vcf`) come in through `file.import`.
2. **Act:** every user-visible action is a command id run through `Session::run` (ribbon,
   shortcuts, command search, CLI, control channel and MCP all dispatch by id). `Session::run`
   snapshots the mailbox for undo — items are `Arc`-shared, so a snapshot copies pointers, not mail —
   restores it if the command fails, and catches an escaped panic the same way (never-crash
   standard).
3. **Show:** the front end reads session state each frame. The message list is computed by
   `engine::list` (filter, search, sort, conversations, group headings) and cached on the mailbox
   revision and the view; only the rows on screen are laid out. A message's body is turned into
   display blocks by `html` and painted as formatted text: nothing in a message runs or is fetched.
4. **Send/Receive:** `sync.request` (pure) says what to do; a host thread runs `net::sync`, which
   sends the Outbox through SMTP and downloads new mail and flag changes over IMAP with `proto`
   and `mime`; `sync.apply` (pure) takes the result in and runs the rules on new Inbox mail. The
   engine and the UI never touch a socket, which is why they build for the browser.
5. **Save:** the desktop app writes the mailbox file a few seconds after each change and on
   exit; the browser writes to its storage; File › Save As and the exports write other files (a
   download in the browser).

## Decisions worth knowing

- **Event times are wall-clock times** in the user's zone ("floating"), so a 9:00 series stays at
  9:00 across a daylight-saving change. Readers convert UTC times on the way in; named zones are
  taken as written for now (see `docs/gaps.md`).
- **Conversations are grouped by topic** (the subject without `RE:`/`FW:` prefixes) within an
  account, as the incumbent does, not by `References` chains.
- **A mailbox file, and its content beside it.** On the desktop `mailbox.mailcraft` is JSON holding
  everything but content; bodies and attachments over a kilobyte are write-once files named by
  their SHA-256 in `mailbox.mailcraft.blobs/` (`crates/engine/src/store.rs`). A save after a read
  mark writes a few megabytes on a background thread, atomically; a self-contained single file is
  still what the browser, exports and `file.saveAs` use. Everything is in memory while the app
  runs, which is the next limit (see `docs/gaps.md`).
- **A Send/Receive arrives in pieces.** `mailcraft_net::sync_with` emits what was sent, what the
  server was told, then mail folder by folder (the Inbox first, newest first, fifty at a time),
  each piece safe to apply by itself (`apply_sync_part`), so mail shows while the rest is on its
  way and a run that is cut short loses nothing: the UID bookkeeping only advances past what was
  handed over.
- **Microsoft Graph beside the mail protocols.** For an account that signs in with Microsoft, the
  host first asks Graph to send the Outbox and, every quarter of an hour, for the calendar and
  contacts (`crates/net/src/graph.rs`); the engine merges them as server-kept, read-only items
  (`cmd/remote.rs`, `Origin`). Mail itself still goes over IMAP.
- **Two-way sync without a journal of everything.** Flags are compared with what the server last
  said (`Message::synced`): a flag changed here since then goes up, one changed there comes down,
  and here wins when both did. Moves and permanent deletes can't be seen by comparing, so the
  mailbox keeps them as `Mailbox::pending` until a run has told the server (`RemoteOp` in
  `SyncRequest`, answered in `SyncResult`). A server copy of a message that is already here (sent
  mail the server files itself, a move the server gave no new UID for) is matched to it rather
  than added. Nothing is expunged without UIDPLUS, and undo history ends at a move, delete or
  send that reached the server. Not pushed yet: folder renames and deletes, drafts, categories.
- **Secrets stay out of our files.** A password or an OAuth refresh token lives in memory, and in
  the operating system's credential store when the owner wants it remembered (`Services::secrets`;
  Windows and macOS so far). The mailbox file, `ui.json` and logs never hold one.
- **Signing in on the provider's page.** Microsoft accounts use OAuth 2.0 (authorization code with
  PKCE, a loopback redirect, the system's browser) and XOAUTH2. The engine only knows that an
  account signs in that way (`Account::oauth`); the host with sockets does it and hands tokens in.
- **Plain-text composing.** Rich text needs an editor component the project does not have yet.

## Testing and gates

- Unit tests in every crate; property tests feeding arbitrary bytes to the readers (`mime`,
  `ical`, `vcard`, `proto`) and round-tripping what the writers produce; in-process fake IMAP and
  SMTP servers on loopback for `net`; `engine/src/tests.rs::hostile_params_never_panic` runs every
  command with junk params; an MCP acceptance test drives a realistic triage through the protocol.
- `cargo xtask ci`: fmt, clippy `-D warnings`, tests, asset attribution, layering, wasm build.
- `cargo xtask parity` regenerates [`parity-checklist.md`](parity-checklist.md); a test keeps the
  catalog percentage above a floor that only rises.
- GitHub Actions (shared `storytold/craft-common-actions` workflows): packaging lint, FreeBSD and
  Windows ARM64 when build files change, release builds on pushes to `release`.

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-10-11 | major | First architecture doc |
| 2026-10-12 | patch | `html` also makes the line a message list shows (`preview`, `text_preview`) |
