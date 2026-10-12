# MailCraft roadmap

**Stage: early alpha** · next: alpha that works with a real account

> **Last reviewed:** 2026-10-11 · **Last updated:** 2026-10-12 · **Change:** minor (progress log) · **Target:** Microsoft Outlook (Microsoft 365, classic desktop)

MailCraft aims for complete parity with Microsoft Outlook, then goes further on speed, openness,
privacy and agent control. This page is the summary; the assessment is
[`docs/target-app-parity.md`](docs/target-app-parity.md), the work list is
[`docs/gaps.md`](docs/gaps.md), and milestones and the current focus are in
[`docs/roadmap.md`](docs/roadmap.md).

## Headline numbers (2026-10-12)

| Number | Value | Kind |
|---|---|---|
| Ribbon/menu catalog coverage | **164 / 310 (52.9%)** | measured (`cargo xtask parity` → [`docs/parity-checklist.md`](docs/parity-checklist.md)) |
| Ready for real work with a real account | **~25%** | estimated ([how](docs/target-app-parity.md#methodology)); two-way sync and Microsoft sign-in exist now, but nothing has run against a real server yet |
| Ready as a local viewer and organiser of exported mail | **~55%** | estimated |
| Commands / crates | 175 commands · 13 crates, 3 apps, `xtask` | measured (source) |
| Tests | see `cargo xtask stats` (every crate; property tests on every reader) | measured |
| Builds verified | Windows x64 (native) and the browser build (wasm) | measured |
| Releases | none published; Windows x64 and Linux x86_64 packages built and installed locally ([releasing.md](docs/releasing.md)) | measured (local builds) |

The gap between the two readiness numbers is the story of this release: the mailbox, the
commands, the interface and the file formats are in good shape, and the connection to real mail
servers is the thinnest part: two-way and with Microsoft sign-in since 2026-10-12, but still never
run against a real server.

## Where it is strong

- The engine: every action a command, undoable, with a failed command leaving nothing behind and
  every command fuzzed with junk parameters.
- The readers: messages, calendars, contacts and server responses are treated as hostile and
  property-tested with arbitrary bytes.
- Privacy by default: nothing in a message runs or is fetched; passwords and sign-ins are kept only
  by the operating system's credential store, never in MailCraft's own files.
- One code base for desktop and browser.
- Agent control: CLI, control channel, MCP.

## What stands between here and beta

In order (details and estimates in [`docs/gaps.md`](docs/gaps.md)): run against real servers;
publisher verification for the Microsoft registration, and sign-in for Google;
calendar and contacts with a server (Microsoft Graph, CalDAV/CardDAV); inline pictures and basic
layout in HTML mail; rich-text replies; meeting invitations by mail; reminders that fire; a store
that scales.

## Progress log

| Date | What landed |
|---|---|
| 2026-10-12 (late) | Time zones (M3.5, gap 10; [calendar-parity.md](docs/calendar-parity.md)): a zone database in every build, the browser's too (the IANA rules with their history, the names Windows has for zones, the system's zone; `mailcraft-time::Zone` over jiff, about 350 KB); every stored time is a moment, because the mailbox now says which zone its clock times are by, and a mailbox from before opens as it was and sends nothing; a computer that travels keeps every appointment at its moment, every all-day item on its day and every series at its own clock, with no change for the server; items and series with zones of their own, picked beside Start and End under **Time Zones**, kept by Microsoft 365 (`start`/`end` in the item's zone, `originalStartTimeZone`, `recurrenceTimeZone`) and in `.ics` (`TZID` with `VTIMEZONE`, both ways); invitations from other zones at the right time here; a zone for the calendar and a second one in the time bar; the mailbox's working hours in its own zone. The Graph stand-in takes the zone names the reference lists and no others, and the flows run a mailbox from before zones to Berlin, Kiritimati and home without one write to the server |
| 2026-10-12 (night) | An icon of its own, in v0.1.1: a carrier pigeon with a sealed letter, in the manner of a wood engraving and the framing of the Craft apps (ink, paper and crimson on the rounded tile). It is drawn by a script in the repository (`assets/app-icon/source/draw_art.py`), and `cargo xtask icon` makes every size from it ([assets/app-icon/README.md](assets/app-icon/README.md)) |
| 2026-10-12 (night) | Public, and released: the repository is [billsecond/mailcraft](https://github.com/billsecond/mailcraft) under the project's own identity (`io.github.billsecond.mailcraft`), and [v0.1.0](https://github.com/billsecond/mailcraft/releases/tag/v0.1.0) holds 23 files built on GitHub's machines from the tag: Windows x64, x86 and ARM64 (`.msi`, portable zip), macOS universal (`.dmg`, the command-line tool), Linux x86_64 and aarch64 (`.AppImage`, `.deb`, `.rpm`, `.flatpak`, `.tar.gz`), FreeBSD, the web build and `SHA256SUMS.txt`. Unsigned. The checks run on Windows, Linux and macOS; what they found on the first day: a place in a text past its end where a script has no font, a link split over two lines, tests that held Windows' keys on a Mac, and a macOS image tool that attaches Finder information to every file |
| 2026-10-12 (evening) | The message list measured against Outlook's, to the pixel at 100 % ([ui-parity.md](docs/ui-parity.md#the-message-list-measured)): 15 clear above the sender and under the preview in rows 72 apart, the sender at 11 pt over subject, time and preview at 8 pt, the time at the end of the subject's line, All and Unread as tabs with the arrangement and its arrow beside them, a line of dots round the row the keys act on, a scroll bar with arrows in a lane of its own. Previews say what a message says: the words of its HTML, the preheader included, without markup, padding (`&zwnj;` and its kind) or web addresses, worked out when a row is drawn, so mail that was already here reads as cleanly |
| 2026-10-12 (evening) | A calendar change reaches the server or the owner is told (M3.1), after a change of time made here didn't show in the owner's Outlook: the server's "yes" is checked against what comes down next (and an answer older than the change puts nothing back); a waiting change is never dropped without a word; events of other calendars, shared ones included, are written through their calendar; a series told in UTC counts its days in UTC; what is only on this computer says so beside the account's calendar and moves there in one action; File › Info › Check Calendar Connection makes, reads, changes and deletes one test appointment on the real account and reports every step; every calendar job is logged (what was sent by name, the zone, a tag of the event's id, what came down, what was put back). Tested against a stand-in that takes only what the documented Graph service takes |
| 2026-10-12 (evening) | Pictures in real mail (M3.3): newsletters built in columns (`font-size:0` round every section) and messages that are only pictures are shown, with the offer to download; the fetcher takes packed answers, WebP, addresses with spaces in them and a host's addresses in turn; a picture that couldn't be downloaded keeps its room and says why, with Try again; pictures can always be downloaded from a sender's domain; what was downloaded is kept on this computer for 30 days and cleared from File › Options |
| 2026-10-12 (afternoon) | The calendar, second and third waves ([calendar-parity.md](docs/calendar-parity.md)): the grid worked like Outlook's (resize by the edges, Ctrl+drag to copy, the keyboard, Month and all-day dragging, a navigator of several months, time scales, week numbers); repeating items as series both ways with "just this one or the entire series"; meetings for real through Microsoft 365 (invitations, updates, cancellations and answers, each only on the owner's yes); every calendar of the account by group, side by side or overlaid; categories; working hours from the mailbox; meeting requests answered in the reading pane with a look at the day; appointment and meeting windows with their own command bar, recurrence dialog, unsaved-changes question and a Scheduling Assistant. Mail: text in a message can be selected and copied. 110,000 lines, 1,300 tests, 172 commands, `cargo xtask ci` green; in use on one real Microsoft 365 mailbox, everything else against stand-ins |
| 2026-10-12 (day) | On a real Microsoft 365 mailbox, and looking like Outlook: the folder tree as the server has it (nested, in Outlook's order, names kept whole); Outlook's palette, the system font on Windows, the rail, the folder pane and a one-row Simplified ribbon (Classic kept as a choice), measured against a reference shot; pictures on the web downloaded only when asked, through a fetcher that tells the server nothing but the address; messages and appointments in windows of their own; a reminders window that is its own window; changes pushed to the server without waiting for a long download. Calendar, first wave ([calendar-parity.md](docs/calendar-parity.md)): click or drag over empty time and type, the current calendar, appointments written to Microsoft 365 at once, a question before changing someone else's meeting, a calendar that keeps itself current. Installing: `mailto:` links and file types, one window per mailbox, the Windows installer and the Linux packages built and installed for the first time (the installer had WordCraft's product identity; X11 needed a library the packages did not ask for), a release workflow of MailCraft's own, and the icon tool ready for Craft-style artwork. 88,000 lines, 1,160 tests, `cargo xtask ci` green |
| 2026-10-12 (night) | Getting ready for a real mailbox: message content out of the rewritten mailbox file and saving in the background (a save after a read mark went from about 1.7 s on the window's thread to about 20 ms off it); Send/Receive in pieces with progress and Cancel, Exchange's non-mail folders skipped, messages over 25 MB kept as a note, every command bounded in time; Microsoft Graph for sending and for the calendar and contacts (read-only); HTML mail with inline pictures, tables and a whitelist of formatting; reminders that fire; recipients suggested as they are typed; messages dragged onto folders; flag and delete on the row under the pointer; a question before anything that can't be taken back; the last Send/Receive's problems kept to read and copy |
| 2026-10-12 | A real account, end to end on paper: account setup as a wizard that recognises the provider (domain table, and mail exchangers looked up over DNS-over-HTTPS for a company's own domain); Microsoft sign-in (OAuth 2.0 authorization code with PKCE in the system browser, XOAUTH2 for IMAP and SMTP); sign-ins remembered in Windows Credential Manager and the macOS keychain; two-way sync (read marks, flags, moves, deletes, folders created by a move, a copy of sent mail; MOVE and UIDPLUS used when offered, never a plain EXPUNGE); Send/Receive by itself on start, every few minutes and soon after a change; the interface follows the system's light or dark appearance; a message's left edge switches read and unread. Calendar, People and Tasks views and every dialog form landed the same day. Tested against in-process fake servers only |
| 2026-10-11 | First version: workspace, layering and gates in the WordCraft mould; `time`, `model`, `html`, `mime`, `ical`, `vcard`, `proto`, `net`, `engine` (143 commands, catalog, sample), `mcp`, `ui-egui` (mail, composer, calendar, people, tasks, Backstage, dialogs, light and dark), desktop app, CLI, browser build; packaging and workflows adapted, not yet run |
