**What's new in 0.5.0** (an alpha for testers; sign up in [#11](https://github.com/GITHUB_REPOSITORY/issues/11)). 0.4.3 was never published, so its changes are here too:
- **Updates itself.** A copy installed from the `.msi`, the macOS app or an AppImage downloads a newer release, keeps it only when MailCraft's own release signature (`SHA256SUMS.txt.sig`) checks out, and installs it when you press Restart to update or quit. If the new version fails to start twice, it goes back to the one before. File › Options › Install updates automatically turns it off; an organisation can turn it off for everyone (`DISABLEAUTOMATICUPDATES=1` for the `.msi`, a policy key, or a file: see `PRIVACY.md`). This release is the first one signed this way, so the first copy that updates itself is 0.5.0, updating to the release after it.
- **Microsoft accounts through Microsoft Graph.** An account set up with an earlier version is offered the switch once; new ones use Graph from the start. Sending, drafts, undo after a change reached the server, Send/Receive Progress (status bar or Send / Receive › Show Progress, with Skip and Stop).
- **Mail to keep offline** (Microsoft accounts, File › Info and Account Settings): keep 3 days to All of your mail in full, and older mail as headers and a preview that downloads when opened. At 200,000 messages a 1-month window took the mailbox from 4.9 GB on disk to 184 MB.
- **Faster, lighter big mailboxes.** The mailbox file is a log of changes, attachments and older bodies stay on disk until wanted, and the first save after a large download runs in the background.
- **Search.** Answered from a word index kept beside the mailbox; More results on the server for Microsoft accounts; `has:attachment` finds mail not downloaded yet; keywords such as **From:** show as pills; `from: name` works with a space; a search of All Mailboxes names each result's folder.
- **Reading.** Mail in every WHATWG character set reads properly, and a character that couldn't be read is shown and explained; HTML mail's centred layouts, pictures in a line of text and rows of icons; attachments open with the system's program; contacts in any character set.
- **Icons** redrawn as one set, clear in light and dark.
- **Use system setting** now follows the Linux desktop's light or dark scheme, and changes with it.
- Gmail: Archive moves to All Mail, Delete for good through the Trash.
- An original is marked replied or forwarded once the answer has actually gone. Deleting a contact asks first. Links and sign-in pages bring the browser to the front on Linux.
- Signing in as someone other than the address typed is asked about, never taken silently.
- Many fixes: a token renewed before it runs out mid Send/Receive, preferences saved whole, a message over 16 MB downloads through Graph, quitting on macOS without the Touch Bar crash, the ribbon and File › Info spacing.

**Early alpha.** The foundations are in place and tested, but much of what a daily mail program needs is not there yet, and most of the Microsoft 365 behaviour has only been tested against stand-ins. What works is in the [README](https://github.com/GITHUB_REPOSITORY#what-works-today).

MailCraft for Windows, macOS, Linux, FreeBSD and the web. Every file is listed with its SHA-256 in `SHA256SUMS.txt`.

| Platform | File | |
|---|---|---|
| Windows 10/11 (x64, x86, ARM64) | `mailcraft-<ver>-windows-<arch>.msi` | Installer for all users: Program Files\MailCraft and the Start menu; offers MailCraft for `mailto:` links and for `.eml`, `.ics` and `.vcf` files without taking them over (choose it in Settings › Default apps); uninstall from Installed apps. `mailcraft-cli.exe` is installed next to the app. |
| | `mailcraft-<ver>-windows-<arch>-portable.zip` | Portable: unzip anywhere and run `mailcraft.exe`; includes `mailcraft-cli.exe`. Registers nothing. |
| macOS 11+ (Apple silicon and Intel) | `mailcraft-<ver>-macos-universal.dmg` | Drag MailCraft to Applications. |
| | `mailcraft-cli-<ver>-macos-universal.zip` | The command-line tool and MCP server alone. |
| Linux x86_64 and aarch64 (glibc 2.35+) | `mailcraft-<ver>-linux-<arch>.AppImage` | `chmod +x` and run (if it does not start, add `--appimage-extract-and-run`). The `.zsync` next to it enables delta updates. |
| | `mailcraft-<ver>-linux-<arch>.deb` / `.rpm` | `sudo apt install ./mailcraft-*.deb` or `sudo dnf install ./mailcraft-*.rpm`. |
| | `mailcraft-<ver>-linux-<arch>.flatpak` | `flatpak install --user mailcraft-*.flatpak` (the freedesktop runtime comes from Flathub). |
| | `mailcraft-<ver>-linux-<arch>.tar.gz` | `bin/mailcraft`, `bin/mailcraft-cli`, the desktop entry, metainfo and icons under `share/`. |
| FreeBSD 15 (x86_64) | `mailcraft-<ver>-freebsd-x86_64.tar.gz` | The two programs, the desktop entry and the icons. |
| Any modern browser | `mailcraft-web-<ver>.zip` | The app as a static site (WebAssembly). It cannot reach mail servers from a browser: bring mail in with File › Import. |

MailCraft keeps your mailbox in its settings folder and saves as you go; one window per mailbox. With a Microsoft 365 or Outlook.com account you sign in on Microsoft's page, and the sign-in is kept by the system's credential store (Windows, macOS, and the Secret Service of GNOME Keyring or KWallet on Linux). What works, and how far each part is from Outlook, is in the [README](https://github.com/GITHUB_REPOSITORY#what-works-today) and in `docs/gaps.md`.

The builds are not signed with a publisher certificate yet: Windows SmartScreen and macOS Gatekeeper will ask before running them.
