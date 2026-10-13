# Interface parity

> **Last reviewed:** 2026-10-13 · **Last updated:** 2026-10-13 · **Change:** minor (the Favorites of the folder pane are Outlook's own for a Microsoft account) · **Target:** Microsoft Outlook (Microsoft 365, classic desktop)

How the window compares with the target's. The look is the Craft apps' own (WordCraft's design
tokens, type rungs and ribbon widgets, with MailCraft's teal accent); the *arrangement* is the one
Outlook users know. No pixel of the target's artwork is used: every icon is drawn in code.

## The window

| Part | State | Notes |
|---|---|---|
| Title bar | yes | Quick Access (Send/Receive, Undo, Redo), search box with scope, account avatar. macOS integrated title bar. No customisation |
| Ribbon | yes | File, Home (changes with the module), Send / Receive, Folder, View, Help, and a Message tab while writing. Collapsible by double-click. No keytips (Alt), no customisation, no simplified ribbon |
| Module bar | yes | Mail, Calendar, People, Tasks, with the Inbox's unread count. No Notes, no reordering |
| Folder pane | yes | Favorites, account trees, collapse/expand, unread counts, context menu, resizable. For an account that signs in with Microsoft the Favorites are the folders Outlook shows there, in Outlook's order, read from the server (never written: they are changed in Outlook); one quiet line under the heading offers to connect them, or says why they can't be had, and File › Options switches back to the favourites kept here. No drag and drop, no search folders, no groups; a favourite that is a person, a category or a group is not shown |
| Message list | yes | Laid out from the target's at 100 %, to the pixel ([the measurements](#the-message-list-measured)): a header with All and Unread as tabs and the arrangement (By Date) with the arrow that turns the list over; rows of the sender (11 pt), the subject with the time at the end of its line, and 0–3 lines of preview (8 pt); a line of dots round the row the keys act on; the bar an unread message wears; a scroll bar with arrows in its own lane. Date/sender/importance groups, conversations as one row with a count, marks, category marks, flag and delete on the row under the pointer, the read switch down a row's left edge, dragging to a folder, context menu, multi-select with Ctrl/Shift, keyboard movement, virtualised. The preview is what the message says to its reader: the words of its HTML (the opening line a sender hid for message lists included), without markup, invisible padding or web addresses. No column view, no collapsing of groups, no opening a conversation in place; the tabs do not turn into a menu in a narrow list (the one in use stays, the others are in the arrangement's menu) |
| Reading pane | yes | Right, bottom or off; header with avatar, recipients, date; notes (flag, importance, replied, pictures withheld); attachment chips; conversation jump list; formatted body with clickable links. Simplified HTML (gap #6); no zoom, no inline reply |
| Composer | yes | In place of the reading pane: From (several accounts), To, Cc, Bcc, Subject, attachment chips, body; Send, Attach, Save draft, Discard. Plain text (gap #7); no pop-out window, no address autocomplete |
| Calendar | yes | Side pane with a month navigator and calendar list; day, work-week, week and month grids; drag to move. A second time zone in the time bar. No schedule view, overlay, drag to resize or create |
| People | yes | List with letter headings and favourites; contact card with recent mail. No business-card or table views, no groups |
| Tasks | yes | Filters, quick add, grouped list with check boxes, details pane. No To-Do bar, no table view |
| Backstage (File) | yes | Info (accounts, mailbox, rules, categories), Open & Export, Options (general, mail, calendar, signatures), About. No print, no automatic replies, no add-ins |
| Status bar | yes | Counts, messages from the last action, Send/Receive status, reading-pane switch. No zoom slider, no connection indicator |
| Dialogs | partial | Search Commands, New/Rename Folder, Go to Date, Sign In, unsaved-message prompt, About, and forms for appointments, contacts, tasks, accounts, categories and rules. The target has many more (Advanced Find, Rules Wizard, Options panes, permissions…) |

## The message list, measured

Distances in pixels at 100 %, from the target's list in its dark theme beside ours (2026-10-12).
A row is measured from the hairline above it.

| What | Target | Ours |
|---|---|---|
| Hairline to the top of the sender's capitals | 15 clear | 15 |
| Under the sender's capitals to the top of the subject's | 7 clear | 7 |
| Under the subject's capitals to the top of the preview's | 7 clear | 7 |
| Under the preview's capitals to the hairline below | 15 clear | 15 |
| Row to row, one line of preview | 72 | 72 (57 with none, 13 more for each further line: the system font's own line at that size) |
| Sender; subject, time and preview | 11 pt (capitals 11 tall); 8 pt (capitals 8 tall) | the same |
| Text from the list's left edge | 30 | 30 |
| Where text stops, and where the time ends, from the row's right end | 98; 24 | 98; 24 |
| Hairline | 1, `#666666`, from 2 inside the list's left edge to the rows' right end | the same (`border_strong`; `border` on a light list) |
| Right of the rows | 7 clear, a lane of 17 (`#141414`) with a handle of 9 and an arrow at each end, 5 to the list's edge | the same |
| Header | 48 high; tabs at 11 pt from 29 in, a line of 1 under the one in use on the header's last row; the arrangement at 9 pt, its arrow 11 from the rows' end | the same |
| Selected row | filled `#424242`, a line of dots (`#BDBDBD`, one on, one off) round it | `#444444` (`selection`), dots in `text_dim` |
| Unread | a bar of 4 (`#58B8FE`) down the row from 2 inside the list's edge | the same (`accent`), with the sender, subject and time in the heavier weight and the subject and time in the accent |

What differs: letters are drawn by our own rasteriser, not the system's, so strokes are a little
heavier than the target's at these sizes; the lines of a two- or three-line preview are 13 apart
by the font's own measure, which the reference (one line of preview) does not show; where marks
go when a row has several (files, importance, categories) is ours, the reference having none.

## Themes, sizes, input

- **Light, dark and follow-the-system** themes; every colour comes from `theme::Tokens`.
- **Interface size** 80–200 % on top of the system scale.
- **Keyboard:** the registry's shortcuts follow the target's where it has them (Ctrl+N, Ctrl+R,
  Ctrl+Shift+R, Ctrl+F, Ctrl+Q/U, Ctrl+1–4, Ctrl+E, Ctrl+Shift+A/Q/C/K, F9, Delete, arrows).
  Missing: keytips, Customize Keyboard, full keyboard access to every control, screen-reader
  labels beyond what egui/AccessKit give by default.
- **Languages:** English; every string goes through `tl!` and a pseudo-language proves it, but
  there are no catalogs and no right-to-left layout (gap #14).
- **Touch and pen:** whatever egui provides; nothing designed for them.

## Hardware and platforms

Rendering is egui on wgpu (DirectX 12 with an OpenGL fallback on Windows, Metal, Vulkan; WebGPU
with a WebGL2 fallback in the browser). Verified here: Windows 11 x64 and a Chromium browser.
High-DPI is handled by egui; multi-window, system tray, notifications and badges are absent
(gap #11).

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-10-11 | major | First version |
| 2026-10-12 | minor | The message list measured against the target's: rows, header, scroll bar, previews |
| 2026-10-12 | patch | The calendar's time bar can show a second time zone |
| 2026-10-13 | minor | The Favorites of the folder pane are Outlook's own, read from the server, for an account that signs in with Microsoft (M4.2) |
