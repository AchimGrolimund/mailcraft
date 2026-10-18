//! Which kind of copy of MailCraft this is, and so whether it can update itself, and which file
//! of a release it would be updated from.
//!
//! A copy installed from the Windows `.msi`, the macOS app (wherever the owner put it) or a
//! Linux AppImage updates itself; every other copy (`.deb`, `.rpm`, Flatpak, the portable
//! `.zip`, a `.tar.gz`, FreeBSD, one built from source) is told of new versions and nothing
//! more. It is known from where the program runs; when that can't be told, it is the notice only.

use std::ffi::OsStr;
use std::path::{Component, Path};

use mailcraft_model::update::Version;

/// How this copy came to be installed, as far as updating it goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Install {
    /// From the Windows `.msi`: in the place the installer put it, which it registered.
    Msi,
    /// A macOS `MailCraft.app`, run from where the owner keeps it (not from the disk image).
    MacApp,
    /// A Linux AppImage.
    AppImage,
    /// Anything else: a package manager's, a portable copy, one built from source.
    NoticeOnly,
}

impl Install {
    /// The name of this kind's file in release `version`, as the release workflow names it
    /// (`mailcraft-<version>-<os>-<arch>.<ext>`), for a program built for `arch`
    /// (`std::env::consts::ARCH`). `None` where there is none to update from.
    pub fn file_name(self, version: &Version, arch: &str) -> Option<String> {
        let (os, arch, ext) = match (self, arch) {
            (Install::Msi, "x86_64") => ("windows", "x64", "msi"),
            (Install::Msi, "x86") => ("windows", "x86", "msi"),
            (Install::Msi, "aarch64") => ("windows", "arm64", "msi"),
            // One app for both kinds of Mac.
            (Install::MacApp, "x86_64" | "aarch64") => ("macos", "universal", "dmg"),
            (Install::AppImage, "x86_64") => ("linux", "x86_64", "AppImage"),
            (Install::AppImage, "aarch64") => ("linux", "aarch64", "AppImage"),
            _ => return None,
        };
        Some(format!("mailcraft-{version}-{os}-{arch}.{ext}"))
    }
}

/// This copy.
pub fn this_copy() -> Install {
    let Ok(exe) = std::env::current_exe() else { return Install::NoticeOnly };
    #[cfg(target_os = "windows")]
    {
        windows(&exe, registered_path().as_deref())
    }
    #[cfg(target_os = "macos")]
    {
        mac(&exe)
    }
    #[cfg(target_os = "linux")]
    {
        let (image, dir) = (std::env::var_os("APPIMAGE"), std::env::var_os("APPDIR"));
        linux(image.as_deref(), dir.as_deref(), std::env::var_os("FLATPAK_ID").is_some(), &exe)
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    {
        let _ = exe;
        Install::NoticeOnly
    }
}

/// Where the `.msi` says it put `mailcraft.exe` (its App Paths entry); the portable `.zip`
/// registers nothing.
#[cfg(target_os = "windows")]
fn registered_path() -> Option<std::path::PathBuf> {
    use winreg::RegKey;
    use winreg::enums::{HKEY_LOCAL_MACHINE, KEY_READ};
    let key = RegKey::predef(HKEY_LOCAL_MACHINE)
        .open_subkey_with_flags(r"Software\Microsoft\Windows\CurrentVersion\App Paths\mailcraft.exe", KEY_READ)
        .ok()?;
    let path: String = key.get_value("").ok()?;
    let path = path.trim().trim_matches('"');
    (!path.is_empty()).then(|| std::path::PathBuf::from(path))
}

/// An `.msi` copy is the one the installer registered: the program is the file it names.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub fn windows(exe: &Path, registered: Option<&Path>) -> Install {
    let same = |a: &Path, b: &Path| a.as_os_str().to_string_lossy().eq_ignore_ascii_case(&b.as_os_str().to_string_lossy());
    match registered {
        Some(registered) if same(exe, registered) => Install::Msi,
        _ => Install::NoticeOnly,
    }
}

/// A macOS app: the program is `<something>.app/Contents/MacOS/<program>`, run from a place
/// the owner keeps it, not from a disk image (`/Volumes/…`) nor from the copy macOS makes of an
/// app it hasn't been moved yet (App Translocation).
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn mac(exe: &Path) -> Install {
    let parts: Vec<&OsStr> = exe.components().filter_map(|c| if let Component::Normal(part) = c { Some(part) } else { None }).collect();
    let bundle = match parts.as_slice() {
        [.., app, contents, macos, _program] if *contents == "Contents" && *macos == "MacOS" => Path::new(app).extension() == Some(OsStr::new("app")),
        _ => false,
    };
    let from_image = exe.starts_with("/Volumes");
    let translocated = parts.iter().any(|part| *part == "AppTranslocation");
    if bundle && exe.is_absolute() && !from_image && !translocated { Install::MacApp } else { Install::NoticeOnly }
}

/// An AppImage: its runtime says where the image is (`APPIMAGE`, an absolute path to a file
/// named `….AppImage`) and where it is mounted (`APPDIR`), and the program runs from inside
/// that. A Flatpak is never one.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn linux(image: Option<&OsStr>, dir: Option<&OsStr>, flatpak: bool, exe: &Path) -> Install {
    let (Some(image), Some(dir)) = (image.map(Path::new), dir.map(Path::new)) else { return Install::NoticeOnly };
    let named = image.extension() == Some(OsStr::new("AppImage"));
    if !flatpak && named && image.is_absolute() && dir.is_absolute() && exe.starts_with(dir) { Install::AppImage } else { Install::NoticeOnly }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(text: &str) -> Version {
        Version::parse(text).unwrap()
    }

    #[test]
    fn the_file_for_each_copy() {
        let version = v("0.5.0");
        for (install, arch, name) in [
            (Install::Msi, "x86_64", Some("mailcraft-0.5.0-windows-x64.msi")),
            (Install::Msi, "x86", Some("mailcraft-0.5.0-windows-x86.msi")),
            (Install::Msi, "aarch64", Some("mailcraft-0.5.0-windows-arm64.msi")),
            (Install::MacApp, "aarch64", Some("mailcraft-0.5.0-macos-universal.dmg")),
            (Install::MacApp, "x86_64", Some("mailcraft-0.5.0-macos-universal.dmg")),
            (Install::AppImage, "x86_64", Some("mailcraft-0.5.0-linux-x86_64.AppImage")),
            (Install::AppImage, "aarch64", Some("mailcraft-0.5.0-linux-aarch64.AppImage")),
            (Install::AppImage, "riscv64", None),
            (Install::Msi, "powerpc", None),
            (Install::NoticeOnly, "x86_64", None),
        ] {
            assert_eq!(install.file_name(&version, arch).as_deref(), name, "{install:?} {arch}");
        }
        assert_eq!(Install::Msi.file_name(&v("1.2.3-rc.1"), "x86_64").as_deref(), Some("mailcraft-1.2.3-rc.1-windows-x64.msi"));
        // Every name is one a release's file may have.
        for install in [Install::Msi, Install::MacApp, Install::AppImage] {
            for arch in ["x86_64", "x86", "aarch64"] {
                if let Some(name) = install.file_name(&v("10.20.30"), arch) {
                    assert!(mailcraft_model::update::is_file_name(&name), "{name}");
                }
            }
        }
    }

    #[test]
    fn an_msi_copy_is_the_one_the_installer_registered() {
        let exe = Path::new(r"C:\Program Files\MailCraft\mailcraft.exe");
        assert_eq!(windows(exe, Some(Path::new(r"C:\Program Files\MailCraft\mailcraft.exe"))), Install::Msi);
        assert_eq!(windows(exe, Some(Path::new(r"c:\program files\mailcraft\MAILCRAFT.EXE"))), Install::Msi);
        // The portable zip, anywhere, while an installed copy is registered elsewhere; or none.
        let portable = Path::new(r"C:\Users\a\Downloads\mailcraft\mailcraft.exe");
        assert_eq!(windows(portable, Some(exe)), Install::NoticeOnly);
        assert_eq!(windows(exe, None), Install::NoticeOnly);
    }

    #[test]
    fn a_mac_app_where_the_owner_keeps_it() {
        assert_eq!(mac(Path::new("/Applications/MailCraft.app/Contents/MacOS/mailcraft")), Install::MacApp);
        assert_eq!(mac(Path::new("/Users/a/Apps/MailCraft.app/Contents/MacOS/mailcraft")), Install::MacApp);
        for not in [
            "/Volumes/MailCraft/MailCraft.app/Contents/MacOS/mailcraft",
            "/private/var/folders/x/T/AppTranslocation/1234/d/MailCraft.app/Contents/MacOS/mailcraft",
            "/usr/local/bin/mailcraft",
            "/Users/a/mailcraft/target/release/mailcraft",
            "/Applications/MailCraft.app/Contents/Resources/mailcraft",
            "MailCraft.app/Contents/MacOS/mailcraft",
        ] {
            assert_eq!(mac(Path::new(not)), Install::NoticeOnly, "{not}");
        }
    }

    #[test]
    fn an_appimage_by_what_its_runtime_says() {
        let image = Some(OsStr::new("/home/a/Apps/mailcraft-0.5.0-linux-x86_64.AppImage"));
        let dir = Some(OsStr::new("/tmp/.mount_mailcrXYZ"));
        let exe = Path::new("/tmp/.mount_mailcrXYZ/usr/bin/mailcraft");
        assert_eq!(linux(image, dir, false, exe), Install::AppImage);
        assert_eq!(linux(image, dir, true, exe), Install::NoticeOnly);
        assert_eq!(linux(None, dir, false, exe), Install::NoticeOnly);
        assert_eq!(linux(image, None, false, exe), Install::NoticeOnly);
        assert_eq!(linux(image, dir, false, Path::new("/usr/bin/mailcraft")), Install::NoticeOnly);
        assert_eq!(linux(Some(OsStr::new("relative.AppImage")), dir, false, exe), Install::NoticeOnly);
        assert_eq!(linux(Some(OsStr::new("/home/a/mailcraft.tar.gz")), dir, false, exe), Install::NoticeOnly);
    }
}
