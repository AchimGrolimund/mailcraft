//! Media types and file names.

/// File extensions and their media types. The first row for a type gives its usual extension.
const TYPES: &[(&str, &str)] = &[
    // text
    ("txt", "text/plain"),
    ("text", "text/plain"),
    ("log", "text/plain"),
    ("html", "text/html"),
    ("htm", "text/html"),
    ("css", "text/css"),
    ("csv", "text/csv"),
    ("tsv", "text/tab-separated-values"),
    ("md", "text/markdown"),
    ("markdown", "text/markdown"),
    ("ics", "text/calendar"),
    ("vcf", "text/vcard"),
    ("xml", "application/xml"),
    ("json", "application/json"),
    ("js", "text/javascript"),
    ("rtf", "application/rtf"),
    // mail
    ("eml", "message/rfc822"),
    ("msg", "application/vnd.ms-outlook"),
    ("mbox", "application/mbox"),
    // office
    ("pdf", "application/pdf"),
    ("doc", "application/msword"),
    ("dot", "application/msword"),
    ("docx", "application/vnd.openxmlformats-officedocument.wordprocessingml.document"),
    ("dotx", "application/vnd.openxmlformats-officedocument.wordprocessingml.template"),
    ("xls", "application/vnd.ms-excel"),
    ("xlsx", "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"),
    ("xltx", "application/vnd.openxmlformats-officedocument.spreadsheetml.template"),
    ("ppt", "application/vnd.ms-powerpoint"),
    ("pps", "application/vnd.ms-powerpoint"),
    ("pptx", "application/vnd.openxmlformats-officedocument.presentationml.presentation"),
    ("ppsx", "application/vnd.openxmlformats-officedocument.presentationml.slideshow"),
    ("odt", "application/vnd.oasis.opendocument.text"),
    ("ods", "application/vnd.oasis.opendocument.spreadsheet"),
    ("odp", "application/vnd.oasis.opendocument.presentation"),
    ("epub", "application/epub+zip"),
    // images
    ("png", "image/png"),
    ("jpg", "image/jpeg"),
    ("jpeg", "image/jpeg"),
    ("gif", "image/gif"),
    ("bmp", "image/bmp"),
    ("webp", "image/webp"),
    ("svg", "image/svg+xml"),
    ("tif", "image/tiff"),
    ("tiff", "image/tiff"),
    ("ico", "image/vnd.microsoft.icon"),
    ("heic", "image/heic"),
    ("avif", "image/avif"),
    // archives
    ("zip", "application/zip"),
    ("gz", "application/gzip"),
    ("tgz", "application/gzip"),
    ("tar", "application/x-tar"),
    ("bz2", "application/x-bzip2"),
    ("xz", "application/x-xz"),
    ("7z", "application/x-7z-compressed"),
    ("rar", "application/vnd.rar"),
    // sound and video
    ("mp3", "audio/mpeg"),
    ("wav", "audio/wav"),
    ("ogg", "audio/ogg"),
    ("m4a", "audio/mp4"),
    ("flac", "audio/flac"),
    ("mp4", "video/mp4"),
    ("m4v", "video/mp4"),
    ("mov", "video/quicktime"),
    ("avi", "video/x-msvideo"),
    ("mkv", "video/x-matroska"),
    ("webm", "video/webm"),
    // keys and signatures
    ("p7s", "application/pkcs7-signature"),
    ("p7m", "application/pkcs7-mime"),
    ("asc", "application/pgp-signature"),
    ("pem", "application/x-pem-file"),
];

/// The media type for a file name, by its extension: `report.pdf` is `application/pdf`. Knows the
/// common text, office, image, archive, sound and video types; anything else is
/// `application/octet-stream`.
pub fn media_type_for(file_name: &str) -> &'static str {
    let name = file_name.trim();
    let Some((stem, ext)) = name.rsplit_once('.') else { return "application/octet-stream" };
    if stem.is_empty() || stem.ends_with(['/', '\\']) {
        // `.profile` is a hidden file, not an extension.
        return "application/octet-stream";
    }
    TYPES.iter().find(|(e, _)| e.eq_ignore_ascii_case(ext)).map_or("application/octet-stream", |(_, t)| *t)
}

/// The usual extension for a media type, without the dot.
pub(crate) fn extension_for(media_type: &str) -> Option<&'static str> {
    TYPES.iter().find(|(_, t)| t.eq_ignore_ascii_case(media_type)).map(|(e, _)| *e)
}

/// A name for an attachment that came without one.
pub(crate) fn default_name(media_type: &str) -> String {
    match media_type {
        "text/calendar" | "application/ics" => "invite.ics".to_string(),
        "message/rfc822" => "message.eml".to_string(),
        other => {
            let stem = if other.starts_with("image/") { "image" } else { "attachment" };
            match extension_for(other) {
                Some(ext) => format!("{stem}.{ext}"),
                None if other.starts_with("text/") => format!("{stem}.txt"),
                None => stem.to_string(),
            }
        }
    }
}

/// A media type that is safe to write for a leaf part: `type/subtype` in lower case. Anything
/// malformed, and `multipart/*` (which would make readers look for parts), becomes
/// `application/octet-stream`.
pub(crate) fn safe_media_type(media_type: &str) -> String {
    let t = media_type.trim().to_ascii_lowercase();
    let token = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric() || b"!#$&^_.+-".contains(&b));
    match t.split_once('/') {
        Some((kind, sub)) if token(kind) && token(sub) && kind != "multipart" => t,
        _ => "application/octet-stream".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn common_types() {
        assert_eq!(media_type_for("report.pdf"), "application/pdf");
        assert_eq!(media_type_for("Photo.JPG"), "image/jpeg");
        assert_eq!(media_type_for("notes.txt"), "text/plain");
        assert_eq!(media_type_for("page.html"), "text/html");
        assert_eq!(media_type_for("data.csv"), "text/csv");
        assert_eq!(media_type_for("archive.tar.gz"), "application/gzip");
        assert_eq!(media_type_for("files.zip"), "application/zip");
        assert_eq!(media_type_for("invite.ics"), "text/calendar");
        assert_eq!(media_type_for("card.vcf"), "text/vcard");
        assert_eq!(media_type_for("letter.docx"), "application/vnd.openxmlformats-officedocument.wordprocessingml.document");
        assert_eq!(media_type_for("sheet.xlsx"), "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet");
        assert_eq!(media_type_for("deck.pptx"), "application/vnd.openxmlformats-officedocument.presentationml.presentation");
        assert_eq!(media_type_for("mail.eml"), "message/rfc822");
        assert_eq!(media_type_for("  logo.png  "), "image/png");
        assert_eq!(media_type_for("résumé.PDF"), "application/pdf");
    }

    #[test]
    fn unknown_types() {
        for name in ["", "README", "file.", ".profile", "x.unknownext", "dir.d/.pdf", "a.b.c.zzz", "."] {
            assert_eq!(media_type_for(name), "application/octet-stream", "{name}");
        }
    }

    #[test]
    fn default_names() {
        assert_eq!(default_name("text/calendar"), "invite.ics");
        assert_eq!(default_name("message/rfc822"), "message.eml");
        assert_eq!(default_name("image/png"), "image.png");
        assert_eq!(default_name("image/x-unknown"), "image");
        assert_eq!(default_name("application/pdf"), "attachment.pdf");
        assert_eq!(default_name("text/x-diff"), "attachment.txt");
        assert_eq!(default_name("application/octet-stream"), "attachment");
        assert_eq!(default_name(""), "attachment");
    }

    #[test]
    fn every_extension_maps_back_to_a_type_with_that_extension() {
        for (ext, media_type) in TYPES {
            assert_eq!(media_type_for(&format!("file.{ext}")), *media_type);
            assert!(extension_for(media_type).is_some());
            assert_eq!(safe_media_type(media_type), *media_type);
        }
    }

    #[test]
    fn media_types_are_made_safe() {
        assert_eq!(safe_media_type(" Application/PDF "), "application/pdf");
        assert_eq!(safe_media_type("image/svg+xml"), "image/svg+xml");
        for bad in ["", "pdf", "a/b/c", "multipart/mixed", "MULTIPART/related", "text/plain; charset=x", "a b/c", "text/pl\r\nain", "/x", "x/"] {
            assert_eq!(safe_media_type(bad), "application/octet-stream", "{bad}");
        }
    }
}
