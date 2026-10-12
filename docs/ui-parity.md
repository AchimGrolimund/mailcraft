# Interface parity

> **Last reviewed:** 2026-10-11 · **Last updated:** 2026-10-11 · **Change:** major (first version) · **Target:** Microsoft Outlook (Microsoft 365, classic desktop)

How the window compares with the target's. The look is the Craft apps' own (WordCraft's design
tokens, type rungs and ribbon widgets, with MailCraft's teal accent); the *arrangement* is the one
Outlook users know. No pixel of the target's artwork is used: every icon is drawn in code.

## The window

| Part | State | Notes |
|---|---|---|
| Title bar | yes | Quick Access (Send/Receive, Undo, Redo), search box with scope, account avatar. macOS integrated title bar. No customisation |
| Ribbon | yes | File, Home (changes with the module), Send / Receive, Folder, View, Help, and a Message tab while writing. Collapsible by double-click. No keytips (Alt), no customisation, no simplified ribbon |
| Module bar | yes | Mail, Calendar, People, Tasks, with the Inbox's unread count. No Notes, no reordering |
| Folder pane | yes | Favorites, account trees, collapse/expand, unread counts, context menu, resizable. No drag and drop, no search folders, no groups |
| Message list | yes | Two-line rows with preview lines (0–3), date/sender/importance groups, sort and filter menus, conversations as one row with a count, markers, category dots, context menu, multi-select with Ctrl/Shift, keyboard movement, virtualised. No column view, no drag and drop, no inline flag/delete on hover |
| Reading pane | yes | Right, bottom or off; header with avatar, recipients, date; notes (flag, importance, replied, pictures withheld); attachment chips; conversation jump list; formatted body with clickable links. Simplified HTML (gap #6); no zoom, no inline reply |
| Composer | yes | In place of the reading pane: From (several accounts), To, Cc, Bcc, Subject, attachment chips, body; Send, Attach, Save draft, Discard. Plain text (gap #7); no pop-out window, no address autocomplete |
| Calendar | yes | Side pane with a month navigator and calendar list; day, work-week, week and month grids; drag to move. No schedule view, overlay, time-zone bar, drag to resize or create |
| People | yes | List with letter headings and favourites; contact card with recent mail. No business-card or table views, no groups |
| Tasks | yes | Filters, quick add, grouped list with check boxes, details pane. No To-Do bar, no table view |
| Backstage (File) | yes | Info (accounts, mailbox, rules, categories), Open & Export, Options (general, mail, calendar, signatures), About. No print, no automatic replies, no add-ins |
| Status bar | yes | Counts, messages from the last action, Send/Receive status, reading-pane switch. No zoom slider, no connection indicator |
| Dialogs | partial | Search Commands, New/Rename Folder, Go to Date, Sign In, unsaved-message prompt, About, and forms for appointments, contacts, tasks, accounts, categories and rules. The target has many more (Advanced Find, Rules Wizard, Options panes, permissions…) |

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
