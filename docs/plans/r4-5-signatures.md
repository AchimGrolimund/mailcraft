# R4.5: Rich signatures, with a logo: implementation plan

> **Status:** proposed, with [the spec](../specs/r4-5-signatures.md) · **Written:** 2026-10-15 · **Change:** major (first version)

Requirement numbers (S1–S17) are the spec's. Issue #37. Each task is one pull request, reviewed by
a person (`needs-review`), with `cargo xtask ci` green on all three CI platforms; task ids go in
commit messages (`R4.5.1: pictures and colour in the rich model`). UI work is looked at in
screenshots, light and dark. Hours are guesses until a task is done.

## Design choices (the spec leaves these to the plan)

- **Pictures in the rich model.** A picture is a run of one object character (U+FFFC) with a
  picture id, so char offsets, selection and undo keep working as they are; the bytes sit in a
  picture list beside the text (`Picture { id, name, mime, data, width, alt }`), held by the
  signature and, once inserted, by the message being written. Only a signature can bring pictures
  into a message: the composer has no Insert Picture, and its paste still drops them.
- **Colour and font** are two more fields of `Style` (`color: Option<Rgb>`, `font:
  Option<FontFamily>` from a closed enum), written as inline `color` and `font-family`; the HTML
  reader keeps the same two on paste.
- **The signature in a message is a region**: its paragraphs carry a marker with the signature's
  name, its layout and a hash of what was inserted. Replacing (S5) finds the region; "edited by the
  owner" is the hash no longer matching. Picture beside text is the region's layout, written as the
  two-cell table and drawn by the editor as two columns.
- **Storage.** A signature's pictures are kept in the mailbox file with the signature (base64, at
  most 512 KB a signature by S6); in a sent message they are ordinary inline attachments and go to
  the content folder like any other.
- **Content-ID**: `sig-<first 16 hex of the bytes' SHA-256>.<message nonce>@mailcraft.invalid`:
  the same picture twice in one message is one part, and no two messages share an id.
- **One editor, two documents.** The editor and the `compose.*`/`format.*`/`edit.*` commands work on
  the document being edited (`Doc::Compose`, or `Doc::Signature` while the Options page has one
  open), so the signature page reuses everything the composer can do; `signature.open`/`.save`
  bracket an edit and undo stays inside it.
- **Details** (S14) are kept on the signature with the hash of the layout they made; a hand edit is
  the hash no longer matching.
- **Commands**: `signature.set` (rich, or text for back compatibility), `signature.rename`,
  `signature.remove`, `signature.defaults {account, new?, reply?}`, `signature.picture.add {path |
  data}`, `signature.picture.set {id, width?, alt?}`, `signature.layout`, `signature.details`,
  `signature.open`, `signature.save`; `compose.signature` replaces (S5); `format.color`,
  `format.font`.

## R4.5.1 The model: pictures, colour, font, two defaults (8–12 h)

| Part | What |
|---|---|
| Rich model | Picture runs and the picture list; `Style` colour and font; HTML out (`<img src="cid:…">` with width and alt; the beside-text table) and in (paste keeps colour and font; `cid:` and file pictures only, never a web address) |
| Signature | Rich body, pictures, layout, details; old `{name, text}` files open (S1); the caps and format checks of S6, by the pictures' own bytes |
| Accounts | `signature` for new and `reply_signature`, migrated so an existing account has the same one for both (S3) |
| Plain text | Derived form: words, descriptions for pictures, no `-- ` (S4); fix the comment in `compose.rs` that says there is one |
| Commands | `signature.*` and `format.color`/`format.font`, with `hostile_params_never_panic` covering them |

**Done when:** a mailbox file of 0.3.0 opens with its signatures and defaults as they were; a rich
signature with two pictures round-trips through save and open; a property test that a signature's
HTML out stays within the set plus `cid:` pictures and the table; refused formats and sizes say why.

## R4.5.2 Into the message and out (6–10 h)

| Part | What |
|---|---|
| Insertion | New, Reply, Reply All, Forward with the right default (S5); Message › Signature replaces the region or goes at the caret; From swaps unless edited |
| Sending | The region's pictures as inline parts in `multipart/related` (S10), Content-IDs as above; plain text derived (S4); MCP `send_message` keeps the new-message signature |

**Done when:** a message with a logo signature is written as `multipart/related` with the picture
inline and reads back with it shown; a reply from account A switched to B carries B's signature,
and one edited first keeps the edit; a plain-text message has the words and no picture part.

## R4.5.3 The signature page and the editor (14–22 h)

| Part | What |
|---|---|
| Editor | One editor over `Doc::Compose` and `Doc::Signature`; pictures drawn, selected, resized by a corner (S8); the beside-text layout drawn as two columns; colour and font in the composer's ribbon and the signature page's bar |
| Page | File › Options › Signatures as in S13: list with New, Rename, Delete; editor; picture width, description, layout; size total and the 100 KB note (S6); per-account defaults; the one-line note of S2 |
| Pictures in | Insert Picture (file dialog), a dropped file, a pasted picture (the host's clipboard gains reading an image, kept as PNG) (S7); the refusal for web-address pictures |

**Done when:** keyboard and mouse tests for editing a signature as for a message; R4.2's tests green;
a pasted signature from an Outlook-on-the-web-style HTML sample keeps its formatting; screenshots
of the page with a logo, light and dark.

## R4.5.4 Create from details, and the preview (6–10 h)

| Part | What |
|---|---|
| Details | The form of S14, its layout (beside-text with a logo, else in line), links made from the fields, laid out again while not hand-edited |
| Preview | The sample reply as a recipient sees it, light and dark, and the plain-text form (S15) |

**Done when:** a signature made from details has working `tel:`, `mailto:` and `https:` links;
changing the job title lays it out again, and after a hand edit doesn't; screenshots of the form and
the preview, light and dark.

## Order

```
R4.5.1 ─ R4.5.2 ─ R4.5.3 ─ R4.5.4
```

Total: about 34–54 hours.

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-10-15 | major | First version |
