# Calendar: matching Outlook

Status: working list, 2026-10-12 (fifth revision: a change that doesn't reach the server can't go unnoticed). The owner's bar for the
calendar is "behaves exactly like Outlook"; this is the list that bar is checked against.
Behaviour is written down from using Outlook for Windows (classic) as a black box; nothing of it
is copied but how it behaves.

Legend: **Done** works and is tested · **Now** being built · **Next** planned, in the order of the
waves at the end · **Later** known, not scheduled. "Tested" means against stand-ins for the
server; what has met the real Microsoft 365 service is only what the owner has used.

## Views and getting around

| Outlook does | MailCraft |
|---|---|
| Day, Work Week, Week, Month views, switched from the ribbon and with Ctrl+Alt+1…4 | Done (the ribbon and the keys are the only switch, as in Outlook; the Simplified ribbon labels them) |
| Schedule view (people or calendars as rows) | Later |
| Today, Back, Forward; the title names the range; Next 7 Days | Done |
| Date navigator: click a day, a week, drag over days; shows more months when the pane is tall; bold days have events | Done (a drag is one run of up to 14 days; a click in the week margin shows that week) |
| Go to Date (Ctrl+G) | Done |
| Alt+1…0 shows that many days; Alt+= month, Alt+- week | Done |
| Half-hour time scale by default; 60, 15, 10, 6 and 5 minutes on request | Done (right-click the time bar, View › Time Scale, or File › Options); no minute labels at the fine scales yet |
| Working hours lighter than the rest, from the owner's settings (start, end, days) | Done: the mailbox's own (days, start, end; asked for once an hour, and it takes the permission to read mailbox settings) for the shading, Work Week's days and Home/End, the local options until they come and whenever File › Options › "Use my own working hours" is ticked. The times are taken as this computer's, whatever time zone the mailbox names. The first day of the week stays a local option: the mailbox's settings as Microsoft Graph gives them don't have it |
| A line at the present time | Done |
| Week numbers (option) | Done (ISO weeks) |
| Several calendars side by side or overlaid, each in its colour, under My Calendars / Other Calendars / shared | Done for the account's own: the list by group (the account's own first, its other groups, Other Calendars, On this computer), each group folding; every ticked calendar in a block of its own with its tab, in Day, Work Week, Week and Month; the arrow on a tab (or View › Overlay) lays calendars over one another, the one in front solid and the others dimmer, a click on a tab bringing it to the front; View › Side-by-Side stands them apart. Right-click a calendar: Show Only This Calendar, Overlay, Colour. A calendar that can only be read says so and takes no new items. Eight blocks at most (the calendars beyond share the last). Later: shared and other people's calendars, Schedule view |
| The calendar clicked in the list is the one new items go to | Done (the account's own calendar unless another is picked); and what is picked, typed or dropped in a calendar's block is that calendar's, which is then the one being worked in |
| Weather, To-Do bar, daily task list | Later |

## Working with items in the grid

| Outlook does | MailCraft |
|---|---|
| Click selects an item; click an empty slot selects the slot; drag selects a range of slots | Done (a range is within one day in the time grid; across days in Month and the all-day row) |
| Typing with a slot selected starts the item right there; Enter saves, Escape drops it | Done (a click elsewhere saves what was typed, as Outlook does) |
| Double-click an empty slot opens a new appointment for that time; double-click an item opens it | Done, each in a window of its own |
| Drag an item to another time or day; the grid scrolls at its edges | Done (snaps to the time scale, ghost), and the server is told at once |
| Drag the top or bottom edge to change start or end | Done (also the ends of all-day and multi-day bars) |
| Ctrl+drag copies; right-button drag asks Move or Copy | Done (a copy is a plain appointment: attendees and recurrence are not copied) |
| Drag in Month view, and all-day items between days; into and out of the all-day row | Done (a repeating item is not turned all-day or timed by dragging) |
| Arrow keys move the slot selection, Shift+arrows extend it, Tab goes from item to item, Enter opens, Delete deletes | Done; also Home/End, Page Up/Down, Alt+Up/Down to move the item, Ctrl+C/X/V (a cut item stays in place until it is pasted) |
| Hovering shows subject, time, location, organizer | Done |
| Right-click: Open, Private, Show As, Categorize, Meeting responses, Delete (and Move/Copy to another calendar) | Done: Open, Delete, Show As, Private, Categorize, Accept / Tentative / Decline, Save as iCalendar, Move to Calendar (from this computer to the account's), and a drop in another calendar's block moves the item there (Ctrl: a copy); Next: moving between server calendars (refused today, and said) |
| A recurring item asks "just this one" or "the entire series" when opened, moved, resized or deleted | Done, for series on this computer and on the server (an occurrence dragged to another day of a series that keeps its weekdays moves alone, without the question) |
| An item that came from someone else's invitation warns before it is changed here | Done (drag, move, resize, edit, delete, and the form's Save) |
| Deleting a meeting the owner organizes offers to send a cancellation; deleting an invitation offers to decline | Done: a cancellation with a note (Microsoft 365 has no deleting without telling the attendees); an invitation can be deleted quietly, declined with word to the organizer, or declined without |
| Overlapping items sit side by side; all-day and multi-day items in the band at the top; private items show a lock; tentative, free, out-of-office and working-elsewhere each have their own look | Done (tentative hatched, out of office purple, an item takes its first category's colour, and the edge down its left is always its calendar's) |

## The appointment and meeting windows

| Outlook does | MailCraft |
|---|---|
| Opens in its own window with its own ribbon: Save & Close, Delete, Show As, Reminder, Recurrence, Time Zones, Categorize, Private, importance | Done: new and existing items open in windows of their own, several at once, each with its own command bar (Save & Close or Send / Send Update, Delete or Cancel Meeting, Show As, Reminder, Recurrence, Categorize, Private, Invite Attendees, Check Names), Ctrl+S, Ctrl+Enter, and a question before closing with unsaved changes. One row, no tabs; no time zones; no importance (items have none yet) |
| Subject, location, start and end with date and time pickers, All day | Done |
| Recurrence dialog: daily, weekly on chosen days, monthly by date or by "second Tuesday", yearly; no end, after n, by date | Done as a dialog over the window; the form says the rule in one line. A window opened for one occurrence shows that day and saves to it alone |
| Reminder choices from "none" and "0 minutes" to "2 weeks" | Done |
| Time zones for start and end | Later (the mailbox keeps floating local times) |
| Rich-text body, attachments, inline pictures | Later (plain text) |
| Meeting: required and optional attendees with names checked against contacts; Send | Done: on a Microsoft 365 calendar Send creates the meeting there and the server mails the invitations; "Don't Send" keeps it on this computer. Updates and cancellations are sent the same way, each after a question |
| Scheduling Assistant: everyone's free/busy on a grid; Room Finder | Done for one day: a row per person, busy / tentative / out of office / working elsewhere / no information, and the meeting's time as a band to move and stretch. Next: several days, AutoPick; Later: rooms |
| Teams meeting button | Next (items have no place for an online meeting yet) |
| Tracking: who accepted, declined, tentatively accepted | Done, kept current with every download |
| Response options: request responses, allow new time proposals | Next |

## Invitations and replies

| Outlook does | MailCraft |
|---|---|
| A meeting request in the Inbox shows Accept, Tentative, Decline and Propose New Time, with a look at the day it falls on | Done but for Propose New Time: the bar under the header with Accept, Tentative, Decline (each with its three ways of answering) and Calendar…; when, where, who; what else is on at that time, and a look at that part of the day. Updates, cancellations, replies, meetings that are over and the owner's own each say what they are. Meeting mail has its mark in the message list |
| Accepting puts it on the calendar and answers the organizer (or not, as chosen) | Done from the message and from the calendar item, with Outlook's three choices: send with a note, send now, don't send. On Microsoft 365 the message is answered through the item the server put on the calendar (asked for if it isn't here yet). Moving an answered request out of the Inbox is an option, off by default |
| Updates and cancellations from the organizer change or remove the item | Done for what comes down from Microsoft 365; a cancellation in mail offers Remove from Calendar (it is not removed by itself) |
| Respond from the calendar item itself | Done |
| Propose New Time; forward a meeting; reply to all attendees with mail | Next |

## Reminders

| Outlook does | MailCraft |
|---|---|
| A reminders window of its own, listing what is due, with Dismiss, Dismiss All and Snooze for a chosen time | Done |
| Double-click opens the item | Done: in the item's own window; of an item that repeats, the occurrence the reminder is for, without asking |
| Sound, and a notification while the program is in the background | Later (the taskbar asks for attention) |

## Staying in step with Microsoft 365

| Outlook does | MailCraft |
|---|---|
| Shows what is on the server, within moments of a change made elsewhere | Done: every half minute while the calendar is in front, at once when the window comes back, the calendar is switched to, other days come into view or something is changed here; every five minutes otherwise |
| Everything made or changed here is on the server at once | Done, meetings and series included: a refusal is put back and said, an unreachable server is tried again. Nothing that mails other people goes without the owner's yes. Undo doesn't reach a change the server already has |
| A change made here is in Outlook, or the owner knows it is not | Done: the server's "yes" is not taken for the change having arrived. Every change it says it took is looked for in what comes down next (asked for again within seconds); what the server doesn't show after three looks, or shows otherwise than it was sent, is put back to what the server has, and the owner is told which item and what of it (status bar and Send/Receive problems). A waiting change that can't be sent any more is never dropped without a word either. An answer that is older than the change it follows (a view asked for right after a write) changes nothing here |
| What is only on this computer doesn't pass for the Outlook calendar | Done: a calendar of the mailbox's own that holds items beside the account's says "Only on this computer: not in your Outlook calendar" on its block and in the list, a line under the header counts the items and offers "Move to My Account's Calendar" (appointments move, meetings ask about their invitations, a copy of what the account's calendar has already just goes, an emptied calendar goes away), and a change to such an item says in the status bar that it stays here |
| (none: Outlook is the server's own program) | File › Info › Check Calendar Connection: on the real account, after a question that says exactly what it does, one test appointment a year ahead is made, read back, changed twice (once as an instant, once said in this computer's time zone), read back each time and deleted; the report has the server's status and words for each step, and what was sent beside what the server has |
| An item of another calendar than the main one, or of one that somebody shares | Done: written through its calendar (`/me/calendars/{id}/events/{id}`), which is the only way the service knows an item of a shared calendar by |
| Recurring series as series (not only as their occurrences), with exceptions | Done both ways for the rules the mailbox can express; "first weekday of the month" and the like still come down as loose occurrences |
| Categories with the mailbox's own names and colours | Done on items both ways; the mailbox's own list and colours come once the owner allows reading mailbox settings (asked for at the next Connect) |
| Every calendar and calendar group of the account; shared calendars; other people's calendars | Done for the account's own calendars and groups (see the list row above): only the account's main calendar is ticked when the list first comes, the rest wait to be ticked, and what the owner ticks stays; Later: shared and other people's |
| Free/busy of other people | Done, in the Scheduling Assistant (asked for only while it is in view) |
| Contacts in People, both ways | Done one way (down); Later: both ways |

## Waves

1. **Done 2026-10-12**: slots and typing, the current calendar, changes written to Microsoft 365,
   the invitation warning, a calendar that keeps itself current, own windows.
2. **Done 2026-10-12**: recurring series both ways with the occurrence-or-series question; resize by
   the edges, copy by Ctrl+drag, keyboard in the grid, Month and all-day dragging, the navigator,
   time scales, week numbers; every calendar of the account with its colour; categories; meetings
   for real (invitations, updates and cancellations through Microsoft 365, each after a question)
   and answering from the calendar; several appointment windows.
3. **Done 2026-10-12**: the reading pane's Accept / Tentative / Decline on a meeting request; the
   calendar list by group, side by side and overlaid; the Scheduling Assistant; the appointment
   window's own command bar, its unsaved-changes question and the recurrence dialog; working hours
   from the mailbox.
4. **Next**: Teams meetings and response options (items need a place for them first); Propose New
   Time; forwarding a meeting; the Scheduling Assistant over several days; moving items between
   server calendars; time zones in invitations from other zones.
5. **Later**: time zones per item, rich bodies and attachments in items, rooms, shared calendars,
   Schedule view, notifications in the background.

## Known differences from Outlook in what is done

- Event ids are the service's ordinary ones, which change when an item is moved to another
  calendar in Outlook: a change that was waiting for such an item is answered "not found", the
  item goes here and the owner is told; the next download has it in its new calendar. (Asking
  for immutable ids would change every id the mailbox has kept so far.)

- A copy made by dragging or pasting is a plain appointment; Outlook keeps the attendees as an
  unsent meeting.
- Cut leaves the item where it is until it is pasted (deleting it first would cancel a meeting on
  the server).
- A repeating item cannot be changed between timed and all-day by dragging.
- Send in a new meeting's window sends at once, as in Outlook; Send Update and the keys for
  sending ask first, where Outlook does not.
- The questions about mail to attendees come up in the main window, not in the item's own.
- After declining on a Microsoft 365 account the item is gone from the server, so the answer can't
  be changed from the message.
- An invitation from another time zone, on an account without a server calendar, lands at the
  organizer's clock time (`TZID` is read as written).
- Enter in a one-line field of the window saves or sends; in Outlook it does not.
- None of the Microsoft 365 behaviour has been checked against the real service beyond the
  owner's own use; where the service turns out to differ, the change is refused and put back.
- An item that repeats is not taken to another calendar by dragging or by cut and paste (one
  day of a series can't be in another calendar than the rest): that is said, and Ctrl copies the
  occurrence there instead.
- The mailbox's working hours are shown as they are written, by this computer's clock: no time
  zone is looked up, so a mailbox whose working hours are kept in another zone than the
  computer's shows them unshifted. Working days that are not next to one another (Monday,
  Wednesday, Friday) make a Work Week from the first of them to the last.
- A calendar's tab has our own arrow and cross; the Overlay and Side-by-Side buttons borrow
  icons the ribbon had already. Outlook opens a calendar in a window of its own on request;
  that is not offered.

## Proven and not proven

Everything in "Staying in step" is tested against a stand-in that takes only what the documented
Microsoft Graph v1.0 service takes (`crates/net/src/graph/standin.rs`: addresses, headers, bodies,
ids with `=`, `/` and `+`, a view that lags behind a change). What only the real service can say
is what the owner's Check Calendar Connection shows: whether a change of time is taken as it is
sent, and whether a time said in this computer's zone means the same instant there.

## Revision history

| Date | Change |
|---|---|
| 2026-10-12 | First version, when the owner made the calendar the top priority |
| 2026-10-12 | After a change of time made here didn't show in the owner's Outlook: sent changes are checked against what comes down, nothing is dropped silently, items that are only on this computer say so and can be moved, events of other calendars go through their calendar, and the owner can check the connection on the real account |
| 2026-10-12 | The first wave is in: rows marked Done, and rows corrected where the code does something else than the list said (server series, deleting meetings, the calendar list) |
| 2026-10-12 | The third wave is in: meeting requests in the reading pane, the appointment window's command bar, unsaved-changes question and recurrence dialog, the Scheduling Assistant, calendars by group, side by side and overlaid, working hours from the mailbox |
| 2026-10-12 | The second wave is in: the grid (resize, copy, keys, navigator, time scales), series as series with the question, meetings and answers through Microsoft 365, every calendar, categories, several appointment windows; a list of known differences |
| 2026-10-12 | Of the third wave: the calendar list by group, calendars side by side and overlaid, working hours from the mailbox, a reminder opening its occurrence; calendars other than the account's own arrive unticked |
