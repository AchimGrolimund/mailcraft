//! `cargo xtask icon`: write MailCraft's app icon in every size the platforms want.
//!
//! Two sources, and nothing traced or copied from any other product's icon either way:
//!
//! - **The owner's artwork**, when `assets/app-icon/source/mailcraft-art.png` is there: an
//!   engraving-style figure on one flat colour, the way the Craft apps' icons are made (black ink
//!   line work, a cream figure, the app colour as the field). It is snapped to exactly three
//!   colours and set in the Craft tile: a 512-unit rounded square (`rx` 112) in the app colour,
//!   the figure clipped to a rounded rectangle 6 units in (`rx` 106) so a thin band of the colour
//!   wraps it. The macOS renders sit on Apple's 824/1024 grid, and the 16–64 px renders show the
//!   figure 22 units closer on each side so it still reads. `source/icon.json` may name the app
//!   colour (`{"color": "#RRGGBB"}`); without it the artwork's own field colour is used.
//! - **The placeholder drawn here, in code**, until there is artwork: a rounded square, a white
//!   envelope with its flap, and a small four-point spark.
//!
//! Output (all listed in ATTRIBUTION.md): `assets/app-icon/mailcraft-1024.png`,
//! `mailcraft-macos-512.png` (with the margin macOS icons keep), `mailcraft.ico`, `mailcraft.icns`
//! and the `hicolor` PNG tree for Linux; the placeholder also has `mailcraft.svg`.

use std::path::Path;

/// Samples per pixel edge (4×4 supersampling).
const SS: u32 = 4;

type Rgba = [f32; 4];

fn mix(a: Rgba, b: Rgba, t: f32) -> Rgba {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t, a[3] + (b[3] - a[3]) * t]
}

/// `top` over `under` (straight alpha).
fn over(under: Rgba, top: Rgba) -> Rgba {
    let a = top[3] + under[3] * (1.0 - top[3]);
    if a <= 0.0 {
        return [0.0; 4];
    }
    let c = |i: usize| (top[i] * top[3] + under[i] * under[3] * (1.0 - top[3])) / a;
    [c(0), c(1), c(2), a]
}

fn rgb(hex: u32) -> Rgba {
    [((hex >> 16) & 255) as f32 / 255.0, ((hex >> 8) & 255) as f32 / 255.0, (hex & 255) as f32 / 255.0, 1.0]
}

/// Distance from a rounded rectangle's edge (negative inside); coordinates in the unit square.
fn round_rect(x: f32, y: f32, cx: f32, cy: f32, hw: f32, hh: f32, r: f32) -> f32 {
    let (dx, dy) = ((x - cx).abs() - (hw - r), (y - cy).abs() - (hh - r));
    dx.max(dy).min(0.0) + (dx.max(0.0).powi(2) + dy.max(0.0).powi(2)).sqrt() - r
}

/// Distance from a line segment.
fn segment(x: f32, y: f32, ax: f32, ay: f32, bx: f32, by: f32) -> f32 {
    let (px, py, dx, dy) = (x - ax, y - ay, bx - ax, by - ay);
    let t = ((px * dx + py * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0);
    ((px - dx * t).powi(2) + (py - dy * t).powi(2)).sqrt()
}

/// The icon's colour at a point of the unit square.
fn sample(x: f32, y: f32) -> Rgba {
    let mut c = [0.0; 4];
    // The tile: a rounded square, lighter at the top.
    if round_rect(x, y, 0.5, 0.5, 0.5, 0.5, 0.225) <= 0.0 {
        c = mix(rgb(0x14_9A_CC), rgb(0x08_62_88), y);
    } else {
        return c;
    }
    // The envelope, with a soft shadow under it.
    let (ex, ey, ew, eh, er) = (0.5, 0.545, 0.31, 0.215, 0.045);
    if round_rect(x, y - 0.018, ex, ey, ew, eh, er) <= 0.0 {
        c = over(c, [0.0, 0.16, 0.24, 0.22]);
    }
    let body = round_rect(x, y, ex, ey, ew, eh, er);
    if body <= 0.0 {
        c = rgb(0xFF_FF_FF);
        // The flap: two strokes from the top corners to the middle.
        let (top, mid) = (ey - eh + 0.03, ey + 0.03);
        let flap = segment(x, y, ex - ew + 0.035, top, ex, mid).min(segment(x, y, ex + ew - 0.035, top, ex, mid));
        if flap <= 0.019 {
            c = rgb(0x0B_7F_AB);
        }
    }
    // The spark: a four-point star, top right.
    let (sx, sy, sr) = ((x - 0.755).abs(), (y - 0.235).abs(), 0.118f32);
    if sx.sqrt() + sy.sqrt() <= sr.sqrt() {
        c = rgb(0xFF_C4_4D);
    }
    c
}

/// The icon as `size`×`size` RGBA8 pixels, drawn inside `scale` of the canvas (1.0 fills it).
fn render(size: u32, scale: f32) -> Vec<u8> {
    render_with(size, scale, &sample)
}

/// `sample` over the unit square as `size`×`size` RGBA8 pixels, inside `scale` of the canvas.
fn render_with(size: u32, scale: f32, sample: &dyn Fn(f32, f32) -> Rgba) -> Vec<u8> {
    let mut out = Vec::with_capacity((size * size * 4) as usize);
    let n = (SS * SS) as f32;
    for py in 0..size {
        for px in 0..size {
            let mut acc = [0.0f32; 4];
            for sy in 0..SS {
                for sx in 0..SS {
                    let u = (px as f32 + (sx as f32 + 0.5) / SS as f32) / size as f32;
                    let v = (py as f32 + (sy as f32 + 0.5) / SS as f32) / size as f32;
                    // Shrink about the centre for icons that keep a margin.
                    let (u, v) = ((u - 0.5) / scale + 0.5, (v - 0.5) / scale + 0.5);
                    let s = if (0.0..=1.0).contains(&u) && (0.0..=1.0).contains(&v) { sample(u, v) } else { [0.0; 4] };
                    // Premultiplied while averaging, so edges don't darken.
                    acc[0] += s[0] * s[3];
                    acc[1] += s[1] * s[3];
                    acc[2] += s[2] * s[3];
                    acc[3] += s[3];
                }
            }
            let a = acc[3] / n;
            for ch in acc.iter().take(3) {
                out.push(if a > 0.0 { ((ch / n / a).clamp(0.0, 1.0) * 255.0).round() as u8 } else { 0 });
            }
            out.push((a.clamp(0.0, 1.0) * 255.0).round() as u8);
        }
    }
    out
}

fn png(size: u32, scale: f32) -> Result<Vec<u8>, String> {
    encode(size, render(size, scale))
}

fn encode(size: u32, pixels: Vec<u8>) -> Result<Vec<u8>, String> {
    let img = image::RgbaImage::from_raw(size, size, pixels).ok_or("icon buffer has the wrong size")?;
    let mut bytes = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(img).write_to(&mut bytes, image::ImageFormat::Png).map_err(|e| format!("encoding the icon: {e}"))?;
    Ok(bytes.into_inner())
}

/// An Apple `.icns` holding PNG entries.
fn icns(entries: &[(&[u8; 4], Vec<u8>)]) -> Vec<u8> {
    let total: usize = 8 + entries.iter().map(|(_, d)| 8 + d.len()).sum::<usize>();
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(b"icns");
    out.extend_from_slice(&(total as u32).to_be_bytes());
    for (kind, data) in entries {
        out.extend_from_slice(*kind);
        out.extend_from_slice(&((8 + data.len()) as u32).to_be_bytes());
        out.extend_from_slice(data);
    }
    out
}

/// The same drawing as SVG (for the Linux scalable icon and for people who want to edit it).
const SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1024 1024">
  <defs>
    <linearGradient id="tile" x1="0" y1="0" x2="0" y2="1">
      <stop offset="0" stop-color="#149ACC"/>
      <stop offset="1" stop-color="#086288"/>
    </linearGradient>
  </defs>
  <rect width="1024" height="1024" rx="230" fill="url(#tile)"/>
  <rect x="194" y="356" width="636" height="440" rx="46" fill="#00293D" fill-opacity="0.22"/>
  <rect x="194" y="338" width="636" height="440" rx="46" fill="#FFFFFF"/>
  <path d="M230 369 L512 589 L794 369" fill="none" stroke="#0B7FAB" stroke-width="39" stroke-linecap="round" stroke-linejoin="round"/>
  <path d="M773 120 Q788 226 894 241 Q788 256 773 362 Q758 256 652 241 Q758 226 773 120 Z" fill="#FFC44D"/>
</svg>
"##;

// ---- the owner's artwork, in the Craft tile ----

/// The reverse-DNS name the Linux icon files carry.
const APP_ID: &str = "io.github.billsecond.mailcraft";
/// Where the artwork and the note about it are looked for, under `assets/app-icon`.
const ART: &str = "source/mailcraft-art.png";
const ART_NOTE: &str = "source/icon.json";
/// The two colours every Craft icon shares; the third is the app's own.
const INK: u32 = 0x0B_0B_0C;
const PAPER: u32 = 0xEF_E9_DC;
/// The tile, in units of a 512 square: its corner radius, how far in the figure is clipped and
/// that clip's radius, and how much closer the small sizes show the figure.
const TILE_R: f32 = 112.0 / 512.0;
const INSET: f32 = 6.0 / 512.0;
const INSET_R: f32 = 106.0 / 512.0;
const SMALL_CROP: f32 = 22.0 / 512.0;
/// The artwork is worked on at this many pixels a side at most (and at least half of it).
const ART_MAX: u32 = 2048;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Part {
    Field,
    Paper,
    Ink,
}

/// Artwork snapped to its three parts.
#[derive(Debug)]
struct Art {
    side: u32,
    parts: Vec<Part>,
    /// The colour its field had.
    field: [u8; 3],
}

fn distance(a: [u8; 3], b: [u8; 3]) -> i32 {
    (0..3).map(|i| (i32::from(a[i]) - i32::from(b[i])).pow(2)).sum()
}

fn bytes(hex: u32) -> [u8; 3] {
    [(hex >> 16) as u8, (hex >> 8) as u8, hex as u8]
}

/// `#RRGGBB` (or `RRGGBB`) as a colour.
fn parse_colour(text: &str) -> Option<u32> {
    let hex = text.trim().trim_start_matches('#');
    if hex.len() == 6 { u32::from_str_radix(hex, 16).ok() } else { None }
}

impl Art {
    /// Snap a picture to field, paper and ink. The field colour is the commonest strongly
    /// coloured one; see-through pixels are field; and so is whatever white a tile that was
    /// already rounded off has outside its corners (found by walking in from the picture's
    /// corners, so white inside the figure stays paper).
    fn snap(img: &image::RgbaImage) -> Result<Art, String> {
        let side = img.width().min(img.height());
        if side < 64 {
            return Err("the artwork is smaller than 64 px".to_string());
        }
        // The middle square of it.
        let (x0, y0) = ((img.width() - side) / 2, (img.height() - side) / 2);
        let mut square = image::imageops::crop_imm(img, x0, y0, side, side).to_image();

        // The field: the fullest bucket of colours that are neither grey nor nearly so.
        let mut buckets = std::collections::HashMap::<(u8, u8, u8), (u64, [u64; 3])>::new();
        for px in square.pixels() {
            let [r, g, b, a] = px.0;
            if a >= 128 && r.max(g).max(b) - r.min(g).min(b) >= 48 {
                let entry = buckets.entry((r >> 4, g >> 4, b >> 4)).or_default();
                entry.0 += 1;
                for (sum, v) in entry.1.iter_mut().zip([r, g, b]) {
                    *sum += u64::from(v);
                }
            }
        }
        let (count, sums) = buckets.values().max_by_key(|(count, _)| *count).copied().ok_or("the artwork has no flat colour for a field")?;
        if count * 20 < u64::from(side) * u64::from(side) {
            return Err("the artwork's most common colour covers less than a twentieth of it: it needs one flat colour as its field".to_string());
        }
        let field = [(sums[0] / count) as u8, (sums[1] / count) as u8, (sums[2] / count) as u8];

        // See-through is field. Filled in before the picture is scaled, so that whatever colour
        // hides under a see-through pixel (black, as a rule) doesn't bleed into its neighbours.
        for px in square.pixels_mut() {
            if px.0[3] < 128 {
                px.0 = [field[0], field[1], field[2], 255];
            } else {
                px.0[3] = 255;
            }
        }
        // At a size worth working at.
        let work = side.clamp(ART_MAX / 2, ART_MAX);
        let square = if work == side { square } else { image::imageops::resize(&square, work, work, image::imageops::FilterType::CatmullRom) };
        let side = work;

        let (ink, paper) = (bytes(INK), bytes(PAPER));
        let mut parts: Vec<Part> = square
            .pixels()
            .map(|px| {
                let [r, g, b, _] = px.0;
                let c = [r, g, b];
                let (to_field, to_paper, to_ink) = (distance(c, field), distance(c, paper), distance(c, ink));
                if to_field <= to_paper && to_field <= to_ink {
                    Part::Field
                } else if to_ink < to_paper {
                    Part::Ink
                } else {
                    Part::Paper
                }
            })
            .collect();
        // White (lighter than the paper colour) reached from a corner is outside the tile the
        // artwork came in.
        let outside = |i: usize| square.as_raw().get(i * 4..i * 4 + 3).is_some_and(|px| px.iter().all(|v| *v >= 236));
        let at = |x: u32, y: u32| (y * side + x) as usize;
        let mut queue: Vec<(u32, u32)> = vec![(0, 0), (side - 1, 0), (0, side - 1), (side - 1, side - 1)];
        let mut seen = vec![false; parts.len()];
        while let Some((x, y)) = queue.pop() {
            let i = at(x, y);
            if seen.get(i).copied().unwrap_or(true) || !outside(i) {
                continue;
            }
            if let (Some(mark), Some(part)) = (seen.get_mut(i), parts.get_mut(i)) {
                *mark = true;
                *part = Part::Field;
            }
            for (nx, ny) in [(x.wrapping_sub(1), y), (x + 1, y), (x, y.wrapping_sub(1)), (x, y + 1)] {
                if nx < side && ny < side {
                    queue.push((nx, ny));
                }
            }
        }
        // The soft edge between that white and the tile's colour would snap to paper: the field
        // takes a few pixels more.
        for _ in 0..4 {
            let mut grown = Vec::new();
            for y in 0..side {
                for x in 0..side {
                    let i = at(x, y);
                    if seen.get(i).copied().unwrap_or(true) {
                        continue;
                    }
                    let near = [(x.wrapping_sub(1), y), (x + 1, y), (x, y.wrapping_sub(1)), (x, y + 1)]
                        .into_iter()
                        .any(|(nx, ny)| nx < side && ny < side && seen.get(at(nx, ny)).copied().unwrap_or(false));
                    if near {
                        grown.push(i);
                    }
                }
            }
            for i in grown {
                if let (Some(mark), Some(part)) = (seen.get_mut(i), parts.get_mut(i)) {
                    *mark = true;
                    *part = Part::Field;
                }
            }
        }
        Ok(Art { side, parts, field })
    }

    /// The part at a point of the unit square.
    fn part(&self, u: f32, v: f32) -> Part {
        let pick = |t: f32| ((t.clamp(0.0, 1.0) * self.side as f32) as u32).min(self.side - 1);
        self.parts.get((pick(v) * self.side + pick(u)) as usize).copied().unwrap_or(Part::Field)
    }
}

/// The tile's colour at a point of the unit square, with the figure `crop` closer on each side.
fn tile(art: &Art, colour: u32, crop: f32, x: f32, y: f32) -> Rgba {
    if round_rect(x, y, 0.5, 0.5, 0.5, 0.5, TILE_R) > 0.0 {
        return [0.0; 4];
    }
    // The band of the app colour round the figure.
    if round_rect(x, y, 0.5, 0.5, 0.5 - INSET, 0.5 - INSET, INSET_R) > 0.0 {
        return rgb(colour);
    }
    let reach = 1.0 - 2.0 * crop;
    match art.part(crop + x * reach, crop + y * reach) {
        Part::Field => rgb(colour),
        Part::Paper => rgb(PAPER),
        Part::Ink => rgb(INK),
    }
}

/// The app colour the note beside the artwork names, if there is a note and it names one.
fn noted_colour(dir: &Path) -> Result<Option<u32>, String> {
    let path = dir.join(ART_NOTE);
    let Ok(text) = std::fs::read_to_string(&path) else { return Ok(None) };
    let note: serde_json::Value = serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    match note.get("color").and_then(serde_json::Value::as_str) {
        Some(c) => parse_colour(c).map(Some).ok_or_else(|| format!("{}: `color` is #RRGGBB", path.display())),
        None => Ok(None),
    }
}

/// Writes one icon file, named from `assets/app-icon`.
type Write<'a> = &'a dyn Fn(&str, &[u8]) -> Result<(), String>;

/// Every icon file, from the artwork.
fn run_art(dir: &Path, write: Write<'_>) -> Result<(), String> {
    let path = dir.join(ART);
    let img = image::open(&path).map_err(|e| format!("{}: {e}", path.display()))?.to_rgba8();
    let art = Art::snap(&img)?;
    let colour = match noted_colour(dir)? {
        Some(c) => c,
        None => (u32::from(art.field[0]) << 16) | (u32::from(art.field[1]) << 8) | u32::from(art.field[2]),
    };
    println!("artwork {}: field #{:02x}{:02x}{:02x}, app colour #{colour:06x}", path.display(), art.field[0], art.field[1], art.field[2]);
    let draw = |size: u32, scale: f32| {
        let crop = if size <= 64 && scale >= 1.0 { SMALL_CROP } else { 0.0 };
        encode(size, render_with(size, scale, &|x, y| tile(&art, colour, crop, x, y)))
    };
    let mut by_size = std::collections::BTreeMap::new();
    for size in [16u32, 24, 32, 48, 64, 128, 256, 512, 1024] {
        by_size.insert(size, draw(size, 1.0)?);
    }
    let get = |size: u32| by_size.get(&size).cloned().ok_or_else(|| format!("no {size} px icon"));
    for size in [16u32, 24, 32, 48, 64, 128, 256, 512] {
        write(&format!("hicolor/{size}x{size}/apps/{APP_ID}.png"), &get(size)?)?;
    }
    write("mailcraft-1024.png", &get(1024)?)?;
    write("mailcraft-macos-512.png", &draw(512, MAC_SCALE)?)?;
    write("mailcraft.ico", &crate::ico::pack(&[get(16)?, get(24)?, get(32)?, get(48)?, get(64)?, get(128)?, get(256)?])?)?;
    let mac = |size: u32| draw(size, MAC_SCALE);
    let entries: Vec<(&[u8; 4], Vec<u8>)> = vec![
        (b"icp4", mac(16)?),
        (b"icp5", mac(32)?),
        (b"icp6", mac(64)?),
        (b"ic07", mac(128)?),
        (b"ic08", mac(256)?),
        (b"ic09", mac(512)?),
        (b"ic10", mac(1024)?),
        (b"ic11", mac(32)?),
        (b"ic12", mac(64)?),
        (b"ic13", mac(256)?),
        (b"ic14", mac(512)?),
    ];
    write("mailcraft.icns", &icns(&entries))?;
    // The placeholder's drawing is not this icon: it goes, so nothing shows the old one.
    for stale in ["mailcraft.svg".to_string(), format!("hicolor/scalable/apps/{APP_ID}.svg")] {
        let path = dir.join(&stale);
        if path.exists() {
            std::fs::remove_file(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            println!("removed {} (the placeholder's drawing)", path.display());
        }
    }
    Ok(())
}

/// macOS icons sit on Apple's grid: an 824 px body on a 1024 px canvas.
const MAC_SCALE: f32 = 824.0 / 1024.0;

pub fn run(root: &Path) -> Result<(), String> {
    let dir = root.join("assets/app-icon");
    let write = |rel: &str, bytes: &[u8]| -> Result<(), String> {
        let path = dir.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        std::fs::write(&path, bytes).map_err(|e| format!("{}: {e}", path.display()))?;
        println!("wrote {}", path.display());
        Ok(())
    };
    if dir.join(ART).exists() {
        return run_art(&dir, &write);
    }
    write("mailcraft.svg", SVG.as_bytes())?;
    write(&format!("hicolor/scalable/apps/{APP_ID}.svg"), SVG.as_bytes())?;
    let mut by_size = std::collections::BTreeMap::new();
    for size in [16u32, 24, 32, 48, 64, 128, 256, 512, 1024] {
        by_size.insert(size, png(size, 1.0)?);
    }
    let get = |size: u32| by_size.get(&size).cloned().ok_or_else(|| format!("no {size} px icon"));
    for size in [16u32, 24, 32, 48, 64, 128, 256, 512] {
        write(&format!("hicolor/{size}x{size}/apps/{APP_ID}.png"), &get(size)?)?;
    }
    write("mailcraft-1024.png", &get(1024)?)?;
    // macOS icons sit inside the canvas with a margin (about 10% each side).
    write("mailcraft-macos-512.png", &png(512, MAC_SCALE)?)?;
    let ico = crate::ico::pack(&[get(16)?, get(24)?, get(32)?, get(48)?, get(64)?, get(128)?, get(256)?])?;
    write("mailcraft.ico", &ico)?;
    let mac = |size: u32| png(size, MAC_SCALE);
    let entries: Vec<(&[u8; 4], Vec<u8>)> = vec![
        (b"icp4", mac(16)?),
        (b"icp5", mac(32)?),
        (b"icp6", mac(64)?),
        (b"ic07", mac(128)?),
        (b"ic08", mac(256)?),
        (b"ic09", mac(512)?),
        (b"ic10", mac(1024)?),
        (b"ic11", mac(32)?),
        (b"ic12", mac(64)?),
        (b"ic13", mac(256)?),
        (b"ic14", mac(512)?),
    ];
    write("mailcraft.icns", &icns(&entries))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_icon_has_its_parts() {
        // Outside the rounded corner: transparent. The tile, the envelope, the flap, the spark.
        assert_eq!(sample(0.01, 0.01)[3], 0.0);
        assert!(sample(0.5, 0.08)[2] > 0.5 && sample(0.5, 0.08)[0] < 0.2);
        assert_eq!(sample(0.3, 0.7), rgb(0xFF_FF_FF));
        assert_eq!(sample(0.5, 0.57), rgb(0x0B_7F_AB));
        assert_eq!(sample(0.755, 0.235), rgb(0xFF_C4_4D));
    }

    /// Artwork the way an image model hands it over: a rounded tile in one colour on white,
    /// a cream figure with a black outline and a white glint in it, soft edges everywhere.
    fn made_up_artwork(side: u32) -> image::RgbaImage {
        let field = [0x73u8, 0x50, 0xD8];
        image::RgbaImage::from_fn(side, side, |px, py| {
            let (x, y) = ((px as f32 + 0.5) / side as f32, (py as f32 + 0.5) / side as f32);
            let from_middle = ((x - 0.5).powi(2) + (y - 0.55).powi(2)).sqrt();
            let glint = ((x - 0.45).powi(2) + (y - 0.5).powi(2)).sqrt() < 0.03;
            let c = if glint || round_rect(x, y, 0.5, 0.5, 0.5, 0.5, 0.2) > 0.0 {
                [255, 255, 255]
            } else if from_middle < 0.27 {
                [0xF2, 0xEC, 0xDE]
            } else if from_middle < 0.31 {
                [0x12, 0x10, 0x14]
            } else {
                // A field that isn't perfectly flat.
                [field[0] + (px % 3) as u8, field[1], field[2] - (py % 2) as u8]
            };
            image::Rgba([c[0], c[1], c[2], 255])
        })
    }

    #[test]
    fn artwork_is_snapped_to_three_parts_and_set_in_the_tile() {
        let art = Art::snap(&made_up_artwork(512)).unwrap();
        assert_eq!(art.side, 1024, "worked on at a size worth it");
        assert!(distance(art.field, [0x73, 0x50, 0xD8]) < 30, "{:?}", art.field);
        // The white outside the artwork's own corners is field; the glint in the figure is paper.
        assert_eq!(art.part(0.004, 0.004), Part::Field);
        assert_eq!(art.part(0.996, 0.996), Part::Field);
        assert_eq!(art.part(0.45, 0.5), Part::Paper);
        assert_eq!(art.part(0.5, 0.7), Part::Paper);
        assert_eq!(art.part(0.5, 0.55 - 0.29), Part::Ink);
        assert_eq!(art.part(0.5, 0.1), Part::Field);
        // Exactly three colours come out, and nothing where the tile's corner is cut away.
        let colour = 0x6F_4F_D8;
        assert_eq!(tile(&art, colour, 0.0, 0.01, 0.01)[3], 0.0);
        assert_eq!(tile(&art, colour, 0.0, 0.5, 0.004), rgb(colour), "the band round the figure");
        assert_eq!(tile(&art, colour, 0.0, 0.5, 0.1), rgb(colour));
        assert_eq!(tile(&art, colour, 0.0, 0.5, 0.7), rgb(PAPER));
        assert_eq!(tile(&art, colour, 0.0, 0.5, 0.55 - 0.29), rgb(INK));
        let px = render_with(64, 1.0, &|x, y| tile(&art, colour, SMALL_CROP, x, y));
        let mut seen = std::collections::BTreeSet::new();
        for p in px.as_chunks::<4>().0.iter().filter(|p| p[3] == 255) {
            seen.insert([p[0], p[1], p[2]]);
        }
        for want in [bytes(colour), bytes(PAPER), bytes(INK)] {
            assert!(seen.contains(&want), "{want:?} is in the small icon");
        }
        // The small sizes show the figure closer: its outline reaches further out.
        let first_ink = |crop: f32| (0..400).find(|i| tile(&art, colour, crop, 0.5, *i as f32 / 400.0) == rgb(INK)).unwrap();
        assert!(first_ink(SMALL_CROP) < first_ink(0.0));
    }

    #[test]
    fn artwork_that_cannot_be_an_icon_is_refused() {
        let grey = image::RgbaImage::from_pixel(256, 256, image::Rgba([128, 128, 128, 255]));
        assert!(Art::snap(&grey).unwrap_err().contains("flat colour"));
        let tiny = image::RgbaImage::from_pixel(32, 32, image::Rgba([200, 30, 30, 255]));
        assert!(Art::snap(&tiny).unwrap_err().contains("64"));
        // A rectangle is taken by its middle square; see-through is field.
        let mut wide = image::RgbaImage::from_pixel(600, 300, image::Rgba([30, 120, 200, 255]));
        wide.put_pixel(300, 150, image::Rgba([0, 0, 0, 0]));
        let art = Art::snap(&wide).unwrap();
        assert!(art.parts.iter().all(|p| *p == Part::Field));
        assert_eq!(parse_colour("#6f4fd8"), Some(0x6F4FD8));
        assert_eq!(parse_colour("6F4FD8 "), Some(0x6F4FD8));
        assert_eq!(parse_colour("purple"), None);
        assert_eq!(parse_colour("#fff"), None);
    }

    #[test]
    fn renders_and_packs() {
        let px = render(16, 1.0);
        assert_eq!(px.len(), 16 * 16 * 4);
        // The corner pixel is see-through, the middle isn't.
        assert_eq!(px[3], 0);
        assert_eq!(px[(8 * 16 + 8) * 4 + 3], 255);
        let data = png(32, 1.0).unwrap();
        assert_eq!(crate::ico::png_size(&data).unwrap(), (32, 32));
        let icns = icns(&[(b"icp5", data.clone())]);
        assert_eq!(&icns[..4], b"icns");
        assert_eq!(u32::from_be_bytes([icns[4], icns[5], icns[6], icns[7]]) as usize, icns.len());
        assert_eq!(icns.len(), 16 + data.len());
    }
}
