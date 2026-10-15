# R4.5: Rich signatures, with a logo

> **Status:** proposed · **Written:** 2026-10-15 · **Author:** Martin Ball, with Claude · **Change:** major (first version) · **Target:** Microsoft Outlook (classic), and better than it where its signature editor is clunky

The words in this document have the meanings in [`CONTEXT.md`](../../CONTEXT.md). It details task
R4.5 of [the roadmap reset](roadmap-reset.md) (issue #37), from a decision session with Martin Ball
on 2026-10-15. The order of the work is in [`docs/plans/r4-5-signatures.md`](../plans/r4-5-signatures.md).

## Why

Replies are rich text since R4.1–R4.4, but a signature is still plain text
(`Signature { name, text }`): it looks wrong beside them, it can't carry the logo most business
signatures have, and an account has one signature for everything where Outlook has one for new
mail and one for replies. Signatures can't be brought over from Outlook either: Microsoft offers no
way to read roaming signatures (no Graph API, v1.0 or beta), so an owner sets them up again, and
Outlook's own editor for that is clunky. Today, on `main`:

- A signature is a name and plain text; inserted as unstyled text, never swapped when the From
  account changes, and with nothing to say where it is in the message.
- The compose model has no pictures, colours or fonts; paste drops pictures.
- `multipart/related` with `cid:` pictures is already written, for a reply's quoted original.
- The editor is bound to the message being written.
- File › Options › Signatures is a list of plain text boxes.

## Goal

An owner builds a good-looking signature, with their logo, in a minute and without fighting the
editor; every message they write carries the right one, as rich text that recipients see with its
pictures without downloading anything, and as words alone in plain text.

## Requirements

### What a signature is

1. A signature is rich text in the composer's set (S12) and may hold pictures (S6–S9). Signatures
   saved as plain text by earlier versions open unchanged, as rich text without formatting.
2. Signatures live in the mailbox on this computer and nowhere else. The editor says, once, in a
   line: signatures can't be kept in step with Outlook, and on another device are set up there (or
   pasted from Outlook on the web, Settings › Signatures).
3. Each account names a signature for **new messages** and one for **replies and forwards**, either
   of which may be none. An account that had a signature before has it for both, so nothing changes
   for it; a new account starts with the default signature for both.
4. A plain-text message carries a signature's words only: derived from the rich signature, pictures
   left out or replaced by their description (S8). There is no `-- ` separator line, as in Outlook.

### In a message

5. New gets the account's new-message signature; Reply, Reply All and Forward get its
   reply-and-forward signature, directly below the owner's typing space and above the quoted
   original. Choosing a signature from Message › Signature replaces the one in the message, or,
   with none there, goes at the caret. Changing the From account swaps the signature for the new
   account's, unless the owner has edited the signature in this message; then it is left as it is.

### Pictures

6. Formats: PNG, JPEG or GIF, recognised by their own bytes; WebP and anything else are refused
   with a reason (many recipients' mail programs can't show them). Each picture at most 256 KB; a
   signature's pictures at most 512 KB together, and at most 10 of them. The editor shows the
   running total ("143 KB of 512 KB") and says, without stopping the owner, that above 100 KB every
   message gets heavier.
7. Pictures come in three ways: Insert Picture from a file; a file dropped on the editor; a picture
   on the clipboard pasted (kept as PNG, within the same caps). A picture in pasted HTML that is
   only a web address is never fetched (the picture rules of `CLAUDE.md`); the owner is told to save
   it and add it from a file. Pasted HTML otherwise keeps its formatting reduced to the set, so a
   signature copied from Outlook on the web comes over in one paste.
8. A picture is shown at a width the owner sets by dragging a corner or typing it (aspect kept, at
   most 600 px); its bytes are never re-encoded. It has a description (alt text), by default its
   file name without the extension.
9. A signature lays its pictures out in one of two ways: in line with the text, or **picture beside
   text** (the first picture on the left, the text on its right; or above it). Picture beside text
   is written as a two-cell table, the form Outlook shows reliably. No other tables.
10. Sent mail carries a signature's pictures as inline parts (`multipart/related`, `cid:`), so
    recipients see them without downloading anything. A plain-text message carries none.
11. MailCraft ships no pictures for signatures: no social-network icons (other companies' marks).
    Social links are text links; an owner may add icons of their own, within the caps.

### The editor

12. The composer's set gains **text colour** (a palette and a custom hex value) and a **font** from
    a short list of widely available families (Arial/Helvetica, Calibri/Carlito, Georgia, Verdana,
    Segoe UI/the system's), written as font names only: no font is shipped. This is the composer's
    set, so new messages and replies have them too.
13. File › Options › Signatures: the list of signatures on the left (New, Rename, Delete); the rich
    editor on the right, with Insert Picture, the layout (S9), a picture's width and description,
    and the size total; below, for each account, the signature for new messages and for replies
    and forwards.
14. **Create from details:** a form (name, job title, organisation, phone, mobile, email, website,
    address, logo) that makes a well-laid-out signature, with phone numbers, the email address and
    the website as working links (`tel:`, `mailto:`, `https:`). The details are kept: changing one
    lays the signature out again, until the owner edits the signature by hand; after that the form
    says it no longer drives it, and offers to start from the details again.
15. A **live preview** beside the editor: the signature at the end of a sample reply as a recipient
    sees it, in light or dark, and its plain-text form.
16. Everything above is a command, as everywhere in MailCraft (scripts, the CLI, MCP); the dialogs
    only collect input. Programmatic picture additions take bytes or a path, never a web address.

### Safety

17. A signature's HTML never escapes the set plus `<img src="cid:…">` and the picture-beside-text
    table, whatever was pasted (property test). Signature text and pictures are the owner's and
    are never logged. Hostile input (a damaged picture, a huge or malformed paste, a mailbox file
    with junk in its signatures) never panics: it is refused or dropped with a reason, and the
    mailbox still opens.

## Out of scope

Keeping signatures in step with Outlook or between devices (Microsoft offers no way); export and
import of signatures to a file (later, if testers ask); pictures in message bodies other than a
signature's; tables other than picture beside text; social icons shipped with MailCraft; vCard
attachments as signatures; per-folder or per-recipient rules for signatures.

## Risks

| Risk | What we do |
|---|---|
| Making the editor edit a signature as well as a message breaks the composer | One editor over two documents, its behaviour tests run for both; R4.2's tests stay green |
| Colour and fonts make mail unreadable in dark mode or on other programs | Colours checked for contrast in the preview's light and dark; fonts are names with fallbacks |
| Pictures make every message heavy | Caps by size, the running total and the 100 KB note |
| Recipients' programs show the beside-text table badly | The same two-cell form Outlook writes; checked in Outlook on the web from a stand-in round trip |
| Pasted signatures from other programs bring formatting outside the set | Reduced to the set by the same reader as compose paste; property test |

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-10-15 | major | First version, from a decision session with Martin Ball (issue #37) |
