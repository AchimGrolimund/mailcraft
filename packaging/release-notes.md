**What's new in 0.5.1** (an alpha for testers; sign up in [#11](https://github.com/GITHUB_REPOSITORY/issues/11)): security fixes, and the first release a copy of 0.5.0 updates itself to.
- **Replies and forwards keep the original's look, and nothing that runs or tracks.** The original under your text keeps its layout, tables, styles and pictures, but scripts, style sheets, embedded objects, forms, unsafe links, tracking pixels and text the sender hid no longer go out to your recipients.
- **Safer file names.** Saving an attachment, Save As and exports clean every name that comes from mail: no hidden direction-reversing characters (`invoice‮fdp.exe` showing as a PDF), invisible characters or reserved Windows names. Attachment chips show the cleaned name.
- **Links say where they really go.** A link's tip names the real host first, in plain ASCII (`xn--…` for an international name), so `https://bank.example@evil.example` reads as evil.example. Links with invisible formatting characters are refused, and only web, mail and phone addresses are ever handed to the system.
- **More attachment kinds are saved, never opened:** remote desktop, web query, SYLK, Office add-in, Access shortcut, Outlook profile, help project and Program Manager group files.
- **Private files.** On Linux, macOS and FreeBSD the settings folder, logs and downloaded pictures are readable by you alone.
- **Updating.** A downloaded update is trusted by the digest in the signed list, and the installer is handed a checked copy in a fresh private folder.
- GNOME under Wayland draws the window's title bar (#207, thanks to @ragelfundionctrumnar).
- Every reader of mail, calendars, contacts and server replies is now fuzzed in CI.

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
