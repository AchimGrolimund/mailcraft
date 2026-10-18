//! MailCraft's icon set, drawn in code (original artwork; no external icon assets).
//!
//! Icons are designed on a 20×20 grid in two colours: line work (`c`) and an accent (`a`).

use egui::epaint::{Mesh, TextShape};
use egui::{Align2, Color32, FontId, Painter, Pos2, Rect, Shape, Stroke, Vec2, pos2, vec2};

/// Draws on the 20-unit grid of one icon, fitted to the screen's pixels.
///
/// Everything is worked out in device pixels: the grid's origin is a whole pixel, a stroke is a
/// whole number of pixels wide (one unit, and never less than one pixel), a straight stroke runs
/// down the middle of a row or column of pixels, and the edges of a fill lie between pixels. So
/// a 20 pt icon at 100 % is drawn in one-pixel lines that are sharp, not two grey half-lines.
struct Pen<'a> {
    p: &'a Painter,
    /// Device pixels per point.
    ppp: f32,
    /// The grid's (0, 0), in device pixels.
    o: Pos2,
    /// Device pixels per grid unit.
    k: f32,
    /// The stroke's width in device pixels.
    w: f32,
    c: Color32,
    a: Color32,
}

/// A line being drawn, in device pixels, from straight pieces and curves.
struct Path {
    pts: Vec<Pos2>,
    /// The last piece was an arc, whose end the next straight piece keeps to.
    after_arc: bool,
}

impl<'a> Pen<'a> {
    fn new(p: &'a Painter, r: Rect, c: Color32, a: Color32) -> Option<Self> {
        let ppp = p.pixels_per_point();
        let size = r.width().min(r.height());
        if !(size > 0.0 && size.is_finite() && ppp > 0.0 && ppp.is_finite()) {
            return None;
        }
        let k = size * ppp / 20.0;
        let corner = r.center() - vec2(size, size) / 2.0;
        let o = pos2((corner.x * ppp).round(), (corner.y * ppp).round());
        Some(Pen { p, ppp, o, k, w: k.round().max(1.0), c, a })
    }
    /// One grid unit in points.
    fn s(&self) -> f32 {
        self.k / self.ppp
    }
    /// A grid point in device pixels, as it falls.
    fn d(&self, x: f32, y: f32) -> Pos2 {
        pos2(self.o.x + x * self.k, self.o.y + y * self.k)
    }
    /// A device-pixel coordinate moved to where a stroke of this width is sharp: the middle of a
    /// pixel for an odd width, the line between two for an even one.
    fn sv(&self, v: f32) -> f32 {
        if self.w as i32 % 2 == 1 { (v - 0.5).round() + 0.5 } else { v.round() }
    }
    /// A grid point for a stroke, in device pixels, snapped.
    fn sp(&self, x: f32, y: f32) -> Pos2 {
        let q = self.d(x, y);
        pos2(self.sv(q.x), self.sv(q.y))
    }
    /// A grid point for the edge of a fill, in device pixels: on the line between pixels.
    fn fp(&self, x: f32, y: f32) -> Pos2 {
        let q = self.d(x, y);
        pos2(q.x.round(), q.y.round())
    }
    /// Device pixels to points.
    fn out(&self, q: Pos2) -> Pos2 {
        pos2(q.x / self.ppp, q.y / self.ppp)
    }
    /// A grid point in points (not snapped).
    fn pt(&self, x: f32, y: f32) -> Pos2 {
        self.out(self.d(x, y))
    }
    fn stroke(&self, c: Color32) -> Stroke {
        Stroke::new(self.w / self.ppp, c)
    }
    /// Stroke a line given in device pixels. Its ends and its corners are round: a stroke of
    /// one pixel is too thin to show it, a wider one has a dot of its width at each.
    fn stroke_px(&self, pts: &[Pos2], closed: bool, c: Color32) {
        if pts.len() < 2 {
            return;
        }
        let v: Vec<Pos2> = pts.iter().map(|q| self.out(*q)).collect();
        if self.w >= 2.0 && c.a() == 255 {
            let r = self.w / 2.0 / self.ppp;
            let n = v.len();
            for (i, q) in v.iter().enumerate() {
                let end = !closed && (i == 0 || i + 1 == n);
                let corner = || {
                    let (Some(a), Some(b)) = (v.get((i + n - 1) % n), v.get((i + 1) % n)) else { return false };
                    let (d1, d2) = (*q - *a, *b - *q);
                    let (l1, l2) = (d1.length(), d2.length());
                    l1 > 0.0 && l2 > 0.0 && d1.dot(d2) / (l1 * l2) < 0.94
                };
                if end || ((closed || (i > 0 && i + 1 < n)) && corner()) {
                    self.p.circle_filled(*q, r, c);
                }
            }
        }
        self.p.add(if closed { Shape::closed_line(v, self.stroke(c)) } else { Shape::line(v, self.stroke(c)) });
    }
    /// A polyline of straight strokes, its corners sharp.
    fn line(&self, pts: &[(f32, f32)]) {
        self.line_c(pts, self.c);
    }
    fn line_c(&self, pts: &[(f32, f32)], c: Color32) {
        let v: Vec<Pos2> = pts.iter().map(|&(x, y)| self.sp(x, y)).collect();
        self.stroke_px(&v, false, c);
    }
    /// A closed outline, its corners sharp.
    fn closed(&self, pts: &[(f32, f32)], c: Color32) {
        let v: Vec<Pos2> = pts.iter().map(|&(x, y)| self.sp(x, y)).collect();
        self.stroke_px(&v, true, c);
    }
    /// A polyline whose corners are rounded with radius `r` units (a box's one unit).
    fn rounded(&self, pts: &[(f32, f32)], closed: bool, r: f32, c: Color32) {
        let v: Vec<Pos2> = pts.iter().map(|&(x, y)| self.sp(x, y)).collect();
        self.stroke_px(&round_corners(&v, closed, r * self.k), closed, c);
    }
    /// A box, its corners rounded by one unit.
    fn rect(&self, x0: f32, y0: f32, x1: f32, y1: f32, c: Color32) {
        self.rounded(&[(x0, y0), (x1, y0), (x1, y1), (x0, y1)], true, 1.0, c);
    }
    /// A filled box, its edges between pixels; its corners rounded by `r` units.
    fn frect_r(&self, x0: f32, y0: f32, x1: f32, y1: f32, r: f32, c: Color32) {
        let (a, b) = (self.out(self.fp(x0, y0)), self.out(self.fp(x1, y1)));
        let rect = Rect::from_min_max(a, b);
        if rect.width() <= 0.0 || rect.height() <= 0.0 {
            return;
        }
        let radius = (r * self.s()).min(rect.width().min(rect.height()) / 2.0);
        self.p.rect_filled(rect, radius, c);
    }
    fn frect(&self, x0: f32, y0: f32, x1: f32, y1: f32, c: Color32) {
        self.frect_r(x0, y0, x1, y1, 0.0, c);
    }
    /// A filled convex polygon, its corners on whole pixels.
    fn fill(&self, pts: &[(f32, f32)], c: Color32) {
        let v: Vec<Pos2> = pts.iter().map(|&(x, y)| self.out(self.fp(x, y))).collect();
        self.p.add(Shape::convex_polygon(v, c, Stroke::NONE));
    }
    /// A filled polygon that every corner of can be seen from `centre` (a flag, a star, a
    /// plane): one mesh, so its inside has no seams, with its outline stroked over the mesh's
    /// hard edge in the same colour.
    fn fill_shape(&self, centre: (f32, f32), pts: &[(f32, f32)], c: Color32) {
        let v: Vec<Pos2> = pts.iter().map(|&(x, y)| self.sp(x, y)).collect();
        let mut mesh = Mesh::default();
        mesh.colored_vertex(self.out(self.d(centre.0, centre.1)), c);
        for q in &v {
            mesh.colored_vertex(self.out(*q), c);
        }
        let n = v.len() as u32;
        for i in 0..n {
            mesh.add_triangle(0, 1 + i, 1 + (i + 1) % n);
        }
        self.p.add(Shape::mesh(mesh));
        self.stroke_px(&v, true, c);
    }
    /// Where a circle centred at (`x`, `y`) of radius `r` units is drawn: its centre snapped for
    /// a stroke, its radius a whole number of pixels.
    fn arc_geom(&self, x: f32, y: f32, r: f32) -> (Pos2, f32) {
        let q = self.d(x, y);
        (pos2(self.sv(q.x), self.sv(q.y)), (r * self.k).round().max(1.0))
    }
    fn circle(&self, x: f32, y: f32, r: f32, c: Color32) {
        let (q, rr) = self.arc_geom(x, y, r);
        self.p.circle_stroke(self.out(q), rr / self.ppp, self.stroke(c));
    }
    /// A dot. A tiny one sits on a pixel; a bigger one has its edge between pixels.
    fn fcircle(&self, x: f32, y: f32, r: f32, c: Color32) {
        let q = self.d(x, y);
        let rr = r * self.k;
        let (q, rr) = if rr >= 1.25 { (pos2(q.x.round(), q.y.round()), rr.round()) } else { (pos2(q.x.floor() + 0.5, q.y.floor() + 0.5), rr.max(0.6)) };
        self.p.circle_filled(self.out(q), rr / self.ppp, c);
    }
    /// An arc of a circle from `from` to `to` degrees (0 = east, clockwise; either way round).
    fn arc(&self, x: f32, y: f32, r: f32, from: f32, to: f32, c: Color32) {
        self.path().a(self, x, y, r, from, to).draw(self, c);
    }
    /// An ellipse, or an arc of one, from `from` to `to` degrees (all curve, so not snapped but
    /// for its centre).
    #[allow(clippy::too_many_arguments)]
    fn ellipse_arc(&self, x: f32, y: f32, rx: f32, ry: f32, from: f32, to: f32, c: Color32) {
        let steps = (((to - from).abs() / 360.0) * (rx.max(ry) * self.k * 3.0)).clamp(8.0, 64.0) as usize;
        let q = self.d(x, y);
        let centre = pos2(self.sv(q.x), self.sv(q.y));
        let (rx, ry) = ((rx * self.k).round().max(1.0), (ry * self.k).round().max(1.0));
        let v: Vec<Pos2> = (0..=steps)
            .map(|i| {
                let t = (from + (to - from) * i as f32 / steps as f32).to_radians();
                centre + vec2(rx * t.cos(), ry * t.sin())
            })
            .collect();
        self.stroke_px(&v, false, c);
    }
    fn path(&self) -> Path {
        Path { pts: Vec::new(), after_arc: false }
    }
    /// Text in the interface's semibold face, centred on (`x`, `y`), `size` units high, turned
    /// clockwise by `angle` radians.
    #[allow(clippy::too_many_arguments)]
    fn text(&self, x: f32, y: f32, size: f32, t: &str, c: Color32, angle: f32) {
        let font: FontId = crate::theme::semibold(size * self.s());
        let galley = self.p.layout_no_wrap(t.to_string(), font, c);
        let at = self.pt(x, y) - galley.size() / 2.0;
        self.p.add(Shape::Text(TextShape::new(at, galley, c).with_angle_and_anchor(angle, Align2::CENTER_CENTER)));
    }
    /// An open arrowhead (a chevron) with its tip at `tip` (device pixels), pointing at `dir`
    /// degrees, its arms `len` units long.
    fn head_px(&self, tip: Pos2, dir: f32, len: f32, c: Color32) {
        let arm = |turn: f32| {
            let t = (dir + 180.0 + turn).to_radians();
            let q = tip + vec2(t.cos(), t.sin()) * len * self.k;
            pos2(self.sv(q.x), self.sv(q.y))
        };
        self.stroke_px(&[arm(-45.0), tip, arm(45.0)], false, c);
    }
    fn head(&self, x: f32, y: f32, dir: f32, len: f32, c: Color32) {
        self.head_px(self.sp(x, y), dir, len, c);
    }
}

impl Path {
    /// Start at a grid point.
    fn m(mut self, pen: &Pen<'_>, x: f32, y: f32) -> Self {
        self.pts.push(pen.sp(x, y));
        self.after_arc = false;
        self
    }
    /// A straight piece to a grid point. After an arc it keeps to the arc's end along the axis it
    /// nearly shares, so the join has no kink.
    fn l(mut self, pen: &Pen<'_>, x: f32, y: f32) -> Self {
        let mut q = pen.sp(x, y);
        if self.after_arc
            && let Some(prev) = self.pts.last()
        {
            if (q.x - prev.x).abs() < 1.5 {
                q.x = prev.x;
            }
            if (q.y - prev.y).abs() < 1.5 {
                q.y = prev.y;
            }
        }
        self.pts.push(q);
        self.after_arc = false;
        self
    }
    /// An arc from `from` to `to` degrees (0 = east, clockwise), joined to what came before. The
    /// point before it moves onto the arc's start along the axis it nearly shares.
    #[allow(clippy::too_many_arguments)]
    fn a(mut self, pen: &Pen<'_>, x: f32, y: f32, r: f32, from: f32, to: f32) -> Self {
        let (centre, rr) = pen.arc_geom(x, y, r);
        let at = |deg: f32| {
            let t = deg.to_radians();
            centre + vec2(rr * t.cos(), rr * t.sin())
        };
        let start = at(from);
        if let Some(last) = self.pts.last_mut() {
            if (last.x - start.x).abs() < 1.5 {
                last.x = start.x;
            }
            if (last.y - start.y).abs() < 1.5 {
                last.y = start.y;
            }
        }
        let steps = (((to - from).abs() / 360.0) * rr * 3.0).clamp(4.0, 64.0) as usize;
        for i in 0..=steps {
            self.pts.push(at(from + (to - from) * i as f32 / steps as f32));
        }
        self.after_arc = true;
        self
    }
    /// A cubic curve from where the line is to the grid point (`x`, `y`).
    fn c(mut self, pen: &Pen<'_>, c1: (f32, f32), c2: (f32, f32), x: f32, y: f32) -> Self {
        let Some(&p0) = self.pts.last() else { return self.m(pen, x, y) };
        let (p1, p2, p3) = (pen.d(c1.0, c1.1), pen.d(c2.0, c2.1), pen.sp(x, y));
        let steps = ((p3 - p0).length() / 1.5).clamp(6.0, 32.0) as usize;
        for i in 1..=steps {
            let t = i as f32 / steps as f32;
            let u = 1.0 - t;
            let q = p0.to_vec2() * (u * u * u) + p1.to_vec2() * (3.0 * u * u * t) + p2.to_vec2() * (3.0 * u * t * t) + p3.to_vec2() * (t * t * t);
            self.pts.push(q.to_pos2());
        }
        self.after_arc = false;
        self
    }
    fn first(&self) -> Option<Pos2> {
        self.pts.first().copied()
    }
    fn last(&self) -> Option<Pos2> {
        self.pts.last().copied()
    }
    fn draw(&self, pen: &Pen<'_>, c: Color32) {
        pen.stroke_px(&self.pts, false, c);
    }
    fn draw_closed(&self, pen: &Pen<'_>, c: Color32) {
        pen.stroke_px(&self.pts, true, c);
    }
}

/// Round a polyline's corners (device pixels) with radius `r` pixels.
fn round_corners(pts: &[Pos2], closed: bool, r: f32) -> Vec<Pos2> {
    let n = pts.len();
    if r < 0.5 || n < 3 {
        return pts.to_vec();
    }
    let mut out = Vec::with_capacity(n * 5);
    for (i, &v) in pts.iter().enumerate() {
        if !closed && (i == 0 || i + 1 == n) {
            out.push(v);
            continue;
        }
        let (Some(&prev), Some(&next)) = (pts.get((i + n - 1) % n), pts.get((i + 1) % n)) else { continue };
        let (d1, d2): (Vec2, Vec2) = (prev - v, next - v);
        let (l1, l2) = (d1.length(), d2.length());
        if l1 < 0.01 || l2 < 0.01 {
            out.push(v);
            continue;
        }
        let t = r.min(l1 / 2.0).min(l2 / 2.0);
        let (p1, p2) = (v + d1 / l1 * t, v + d2 / l2 * t);
        for s in 0..=4 {
            let u = s as f32 / 4.0;
            let q = p1.to_vec2() * ((1.0 - u) * (1.0 - u)) + v.to_vec2() * (2.0 * u * (1.0 - u)) + p2.to_vec2() * (u * u);
            out.push(q.to_pos2());
        }
    }
    out
}

impl Pen<'_> {
    /// An envelope: a box with its flap.
    fn envelope(&self, x0: f32, y0: f32, x1: f32, y1: f32, c: Color32) {
        self.rect(x0, y0, x1, y1, c);
        let mid = (x0 + x1) / 2.0;
        self.line_c(&[(x0, y0 + 0.4), (mid, y0 + (y1 - y0) * 0.55), (x1, y0 + 0.4)], c);
    }
    /// A folder outline.
    fn folder(&self, c: Color32) {
        self.closed(&[(2.5, 5.0), (8.0, 5.0), (9.5, 7.0), (17.5, 7.0), (17.5, 15.5), (2.5, 15.5)], c);
    }
    /// A tray (inbox, outbox).
    fn tray(&self, c: Color32) {
        self.closed(&[(3.0, 11.0), (5.5, 5.0), (14.5, 5.0), (17.0, 11.0), (17.0, 15.5), (3.0, 15.5)], c);
        self.line_c(&[(3.0, 11.0), (7.0, 11.0), (8.0, 13.0), (12.0, 13.0), (13.0, 11.0), (17.0, 11.0)], c);
    }
    /// A bin.
    fn bin(&self, c: Color32) {
        self.line_c(&[(4.0, 6.0), (16.0, 6.0)], c);
        self.line_c(&[(8.0, 6.0), (8.0, 4.0), (12.0, 4.0), (12.0, 6.0)], c);
        self.line_c(&[(5.5, 6.0), (6.3, 16.5), (13.7, 16.5), (14.5, 6.0)], c);
    }
    /// A calendar page with its binding.
    fn calendar(&self, c: Color32) {
        self.rect(3.0, 4.5, 17.0, 16.5, c);
        self.frect(3.0, 4.5, 17.0, 7.5, self.a);
        self.line_c(&[(6.5, 3.0), (6.5, 5.5)], c);
        self.line_c(&[(13.5, 3.0), (13.5, 5.5)], c);
    }
    /// A head and shoulders.
    fn person(&self, x: f32, y: f32, k: f32, c: Color32) {
        self.circle(x, y - 3.2 * k, 2.6 * k, c);
        self.arc(x, y + 5.2 * k, 5.4 * k, 205.0, 335.0, c);
    }
    /// A looping arrow (sync, repeat).
    fn loop_arrows(&self, c: Color32, heads: Color32) {
        self.arc(10.0, 10.0, 6.0, 200.0, 340.0, c);
        self.arc(10.0, 10.0, 6.0, 20.0, 160.0, c);
        self.fill(&[(15.0, 5.2), (18.0, 8.8), (13.6, 9.2)], heads);
        self.fill(&[(5.0, 14.8), (2.0, 11.2), (6.4, 10.8)], heads);
    }
    fn check(&self, x: f32, y: f32, k: f32, c: Color32) {
        self.line_c(&[(x - 3.5 * k, y), (x - 1.0 * k, y + 2.6 * k), (x + 3.8 * k, y - 3.0 * k)], c);
    }
    fn plus(&self, x: f32, y: f32, k: f32, c: Color32) {
        self.line_c(&[(x - k, y), (x + k, y)], c);
        self.line_c(&[(x, y - k), (x, y + k)], c);
    }
}

/// Paint icon `name` into `r`. `c` is the line colour; the accent comes from the theme.
pub fn paint(p: &Painter, r: Rect, name: &str, c: Color32, accent: Color32) {
    let Some(pen) = Pen::new(p, r, c, accent) else { return };
    let a = accent;
    let red = Color32::from_rgb(0xD1, 0x3B, 0x3B);
    let yellow = Color32::from_rgb(0xD9, 0xA4, 0x0C);
    let green = Color32::from_rgb(0x2E, 0x8B, 0x57);
    let blue = Color32::from_rgb(0x2F, 0x6F, 0xD0);
    let soft = a.linear_multiply(0.18);
    match name {
        // Mail
        "mail" | "email" | "unread" | "markUnread" => {
            pen.envelope(2.5, 5.0, 17.5, 15.0, c);
            if name.ends_with("nread") {
                pen.fcircle(16.5, 5.5, 2.6, a);
            }
        }
        "new" | "newMail" => {
            pen.envelope(2.5, 6.0, 15.5, 16.0, c);
            pen.fcircle(15.5, 5.5, 3.6, a);
            pen.plus(15.5, 5.5, 2.0, Color32::WHITE);
        }
        "read" | "markRead" | "toggleRead" => {
            pen.closed(&[(3.0, 9.0), (10.0, 4.0), (17.0, 9.0), (17.0, 16.5), (3.0, 16.5)], c);
            pen.line(&[(3.0, 9.0), (10.0, 13.5), (17.0, 9.0)]);
        }
        "markAllRead" => {
            pen.closed(&[(3.0, 9.0), (10.0, 4.0), (17.0, 9.0), (17.0, 16.5), (3.0, 16.5)], c);
            pen.check(10.0, 11.5, 1.0, a);
        }
        "reply" => {
            pen.line_c(&[(8.5, 5.0), (3.5, 10.0), (8.5, 15.0)], a);
            pen.line_c(&[(3.8, 10.0), (11.0, 10.0), (14.5, 11.5), (16.5, 15.5)], a);
        }
        "replyAll" => {
            pen.line_c(&[(7.0, 5.0), (2.0, 10.0), (7.0, 15.0)], c);
            pen.line_c(&[(11.0, 5.0), (6.0, 10.0), (11.0, 15.0)], a);
            pen.line_c(&[(6.3, 10.0), (13.0, 10.0), (16.0, 11.5), (17.8, 15.5)], a);
        }
        "forward" => {
            pen.line_c(&[(11.5, 5.0), (16.5, 10.0), (11.5, 15.0)], a);
            pen.line_c(&[(16.2, 10.0), (9.0, 10.0), (5.5, 11.5), (3.5, 15.5)], a);
        }
        "delete" | "deleted" | "discard" | "deletePermanently" | "remove" => {
            pen.bin(if name == "deletePermanently" { red } else { c });
            pen.line(&[(8.4, 9.0), (8.7, 14.0)]);
            pen.line(&[(11.6, 9.0), (11.3, 14.0)]);
        }
        "empty" | "emptyFolder" => {
            pen.bin(c);
            pen.line_c(&[(8.0, 9.5), (12.0, 13.5)], red);
            pen.line_c(&[(12.0, 9.5), (8.0, 13.5)], red);
        }
        "archive" => {
            pen.rect(3.0, 4.0, 17.0, 8.0, c);
            pen.line(&[(4.5, 8.0), (4.5, 16.0), (15.5, 16.0), (15.5, 8.0)]);
            pen.line_c(&[(8.0, 11.0), (12.0, 11.0)], a);
        }
        "junk" | "notJunk" => {
            pen.circle(10.0, 10.0, 6.5, if name == "junk" { red } else { c });
            pen.line_c(&[(5.4, 5.4), (14.6, 14.6)], if name == "junk" { red } else { c });
            if name == "notJunk" {
                pen.check(14.5, 14.5, 0.8, green);
            }
        }
        "move" => {
            pen.folder(c);
            pen.line_c(&[(6.5, 11.3), (13.0, 11.3)], a);
            pen.line_c(&[(10.8, 9.0), (13.2, 11.3), (10.8, 13.6)], a);
        }
        "copy" => {
            pen.rect(3.5, 3.0, 12.5, 14.0, c);
            pen.frect(7.5, 6.5, 16.5, 17.5, soft);
            pen.rect(7.5, 6.5, 16.5, 17.5, a);
        }
        "flag" | "toggleFlag" | "followUp" => {
            pen.line(&[(5.0, 3.0), (5.0, 17.5)]);
            pen.fill(&[(5.0, 4.0), (15.5, 4.0), (15.5, 11.0), (5.0, 11.0)], red);
        }
        "clearFlag" => {
            pen.line(&[(5.0, 3.0), (5.0, 17.5)]);
            pen.closed(&[(5.0, 4.0), (15.5, 4.0), (15.5, 11.0), (5.0, 11.0)], c);
        }
        "complete" | "check" | "accept" => pen.check(10.0, 10.5, 1.5, green),
        "categorize" | "category" | "categories" => {
            pen.frect(3.5, 3.5, 9.0, 9.0, red);
            pen.frect(11.0, 3.5, 16.5, 9.0, yellow);
            pen.frect(3.5, 11.0, 9.0, 16.5, green);
            pen.frect(11.0, 11.0, 16.5, 16.5, blue);
        }
        "pin" => {
            pen.fill(&[(7.0, 3.0), (13.0, 3.0), (12.0, 8.5), (8.0, 8.5)], a);
            pen.line_c(&[(5.5, 9.0), (14.5, 9.0)], a);
            pen.line(&[(10.0, 9.0), (10.0, 17.5)]);
        }
        "rules" | "createRule" => {
            pen.envelope(2.5, 3.5, 11.5, 9.5, c);
            pen.line_c(&[(7.0, 11.5), (7.0, 15.0), (12.5, 15.0)], a);
            pen.line_c(&[(10.5, 13.0), (12.7, 15.0), (10.5, 17.0)], a);
            pen.folder_small(13.0, 11.5, c);
        }
        "attach" | "attachment" => pen.line(&[
            (13.0, 5.0),
            (6.5, 11.5),
            (6.5, 14.0),
            (8.5, 16.0),
            (11.0, 16.0),
            (16.0, 11.0),
            (16.0, 8.5),
            (14.0, 7.0),
            (9.0, 12.0),
            (9.5, 13.3),
        ]),
        "send" => {
            pen.fill(&[(3.0, 4.0), (17.5, 10.0), (3.0, 10.0)], a);
            pen.fill(&[(3.0, 10.0), (17.5, 10.0), (3.0, 16.0)], a.linear_multiply(0.7));
            pen.line_c(&[(3.0, 10.0), (10.0, 10.0)], Color32::WHITE);
        }
        "sent" => {
            pen.closed(&[(3.0, 4.5), (17.0, 10.0), (3.0, 15.5), (5.5, 10.0)], c);
            pen.line(&[(5.5, 10.0), (12.0, 10.0)]);
        }
        "drafts" | "editDraft" | "edit" | "rename" => {
            pen.line(&[(4.0, 16.0), (4.5, 12.5), (13.0, 4.0), (16.0, 7.0), (7.5, 15.5), (4.0, 16.0)]);
            pen.line_c(&[(11.3, 5.7), (14.3, 8.7)], a);
        }
        "inbox" => {
            pen.tray(c);
            pen.line_c(&[(10.0, 2.5), (10.0, 8.5)], a);
            pen.line_c(&[(7.8, 6.5), (10.0, 8.8), (12.2, 6.5)], a);
        }
        "outbox" | "export" => {
            pen.tray(c);
            pen.line_c(&[(10.0, 9.0), (10.0, 3.0)], a);
            pen.line_c(&[(7.8, 5.2), (10.0, 2.9), (12.2, 5.2)], a);
        }
        "import" | "open" => {
            pen.line(&[(3.5, 12.0), (3.5, 16.0), (16.5, 16.0), (16.5, 12.0)]);
            pen.line_c(&[(10.0, 3.0), (10.0, 12.0)], a);
            pen.line_c(&[(6.8, 9.0), (10.0, 12.2), (13.2, 9.0)], a);
        }
        "folder" | "custom" | "select" => pen.folder(c),
        "folderNew" | "newFolder" => {
            pen.folder(c);
            pen.plus(13.0, 11.3, 2.4, a);
        }
        "favorite" | "star" | "starOn" => {
            let pts =
                [(10.0, 2.8), (12.1, 7.6), (17.3, 8.1), (13.4, 11.6), (14.5, 16.7), (10.0, 14.0), (5.5, 16.7), (6.6, 11.6), (2.7, 8.1), (7.9, 7.6)];
            if name == "starOn" {
                for i in 0..5 {
                    let (tip, l, r) = (pts[i * 2], pts[(i * 2 + 9) % 10], pts[(i * 2 + 1) % 10]);
                    pen.fill(&[tip, r, (10.0, 10.2), l], yellow);
                }
            }
            pen.closed(&pts, if name == "starOn" { yellow } else { c });
        }
        // Compose
        "save" | "saveDraft" => {
            pen.closed(&[(3.5, 3.5), (13.5, 3.5), (16.5, 6.5), (16.5, 16.5), (3.5, 16.5)], c);
            pen.rect(6.5, 3.5, 12.0, 7.5, c);
            pen.frect(6.0, 11.0, 14.0, 16.5, soft);
            pen.rect(6.0, 11.0, 14.0, 16.5, a);
        }
        "highImportance" | "importanceHigh" => {
            pen.frect(9.0, 3.0, 11.0, 12.0, red);
            pen.fcircle(10.0, 15.5, 1.5, red);
        }
        "lowImportance" | "importanceLow" => {
            pen.line_c(&[(10.0, 3.5), (10.0, 15.5)], blue);
            pen.line_c(&[(5.5, 11.0), (10.0, 15.8), (14.5, 11.0)], blue);
        }
        "signature" => {
            pen.line_c(&[(3.0, 13.0), (5.0, 8.0), (7.0, 13.0), (9.0, 9.0), (11.0, 13.0), (14.0, 11.0)], a);
            pen.line(&[(3.0, 16.5), (17.0, 16.5)]);
            pen.line(&[(13.5, 8.5), (17.0, 3.5)]);
        }
        "checkNames" => {
            pen.person(8.0, 9.5, 1.0, c);
            pen.check(15.0, 13.5, 0.9, green);
        }
        "close" | "decline" | "cancel" => {
            let k = if name == "decline" { red } else { c };
            pen.line_c(&[(5.0, 5.0), (15.0, 15.0)], k);
            pen.line_c(&[(15.0, 5.0), (5.0, 15.0)], k);
        }
        "tentative" => pen.text(10.0, 10.0, 15.0, "?", c, 0.0),
        // Modules and navigation
        "calendar" | "appointment" => {
            pen.calendar(c);
            for (x, y) in [(6.5, 10.5), (10.0, 10.5), (13.5, 10.5), (6.5, 13.5), (10.0, 13.5)] {
                pen.fcircle(x, y, 0.9, c);
            }
        }
        "newAppointment" | "newEvent" => {
            pen.calendar(c);
            pen.plus(10.0, 12.0, 2.6, a);
        }
        "today" => {
            pen.calendar(c);
            pen.fcircle(10.0, 12.0, 2.3, a);
        }
        "day" | "workWeek" | "week" | "month" => {
            pen.rect(3.0, 4.0, 17.0, 16.0, c);
            pen.line(&[(3.0, 7.0), (17.0, 7.0)]);
            match name {
                "day" => pen.frect(5.0, 9.0, 15.0, 14.0, a),
                "workWeek" => {
                    for i in 0..5 {
                        let x = 4.4 + i as f32 * 2.4;
                        pen.frect(x, 8.5, x + 1.6, 14.5, if i == 2 { a } else { c.linear_multiply(0.45) });
                    }
                }
                "week" => {
                    for i in 0..7 {
                        let x = 4.1 + i as f32 * 1.75;
                        pen.frect(x, 8.5, x + 1.1, 14.5, if i == 3 { a } else { c.linear_multiply(0.45) });
                    }
                }
                _ => {
                    for row in 0..3 {
                        for col in 0..4 {
                            let (x, y) = (5.0 + col as f32 * 3.0, 8.4 + row as f32 * 2.5);
                            pen.frect(x, y, x + 1.6, y + 1.4, if (row, col) == (1, 2) { a } else { c.linear_multiply(0.45) });
                        }
                    }
                }
            }
        }
        "people" | "contact" | "account" | "newContact" => {
            pen.person(10.0, 10.0, 1.15, c);
            if name == "newContact" {
                pen.plus(15.5, 5.5, 2.2, a);
            }
        }
        "meeting" | "newMeeting" | "fromMessage" => {
            pen.person(7.0, 10.5, 0.85, c);
            pen.person(13.5, 10.5, 0.85, a);
        }
        // A calendar page that came by mail: an invitation (the flap of its envelope), its
        // cancellation, or somebody's answer to one.
        "meetingRequest" | "meetingCancelled" | "meetingAccepted" | "meetingTentative" | "meetingDeclined" => {
            pen.calendar(c);
            match name {
                "meetingRequest" => pen.line_c(&[(5.5, 10.0), (10.0, 13.6), (14.5, 10.0)], c),
                "meetingAccepted" => pen.check(10.0, 12.2, 0.95, green),
                "meetingTentative" => pen.text(10.0, 12.2, 9.5, "?", c, 0.0),
                // Called off: struck through.
                "meetingCancelled" => pen.line_c(&[(3.5, 17.0), (16.5, 6.0)], red),
                _ => {
                    pen.line_c(&[(7.3, 9.6), (12.7, 15.0)], red);
                    pen.line_c(&[(12.7, 9.6), (7.3, 15.0)], red);
                }
            }
        }
        "tasks" | "task" | "newTask" => {
            pen.rect(3.5, 3.5, 16.5, 16.5, c);
            pen.check(10.0, 10.3, 1.05, a);
        }
        "next" | "forwardPeriod" => pen.line(&[(7.5, 4.5), (13.0, 10.0), (7.5, 15.5)]),
        "previous" | "back" => pen.line(&[(12.5, 4.5), (7.0, 10.0), (12.5, 15.5)]),
        "reminder" | "bell" => {
            pen.line(&[
                (4.0, 14.0),
                (5.5, 12.0),
                (5.5, 8.5),
                (7.5, 5.5),
                (10.0, 4.8),
                (12.5, 5.5),
                (14.5, 8.5),
                (14.5, 12.0),
                (16.0, 14.0),
                (4.0, 14.0),
            ]);
            pen.line_c(&[(8.5, 16.0), (10.0, 17.0), (11.5, 16.0)], a);
        }
        "private" | "lock" => {
            pen.rect(5.0, 9.0, 15.0, 16.5, c);
            pen.arc(10.0, 9.0, 3.2, 180.0, 360.0, c);
            pen.fcircle(10.0, 12.7, 1.2, a);
        }
        "recurrence" | "repeat" => pen.loop_arrows(c, c),
        // People's time in rows, and the meeting's own time down across them.
        "scheduling" => {
            pen.frect(8.5, 3.0, 13.0, 17.0, soft);
            pen.line_c(&[(8.5, 3.0), (8.5, 17.0)], a);
            pen.line_c(&[(13.0, 3.0), (13.0, 17.0)], a);
            pen.frect(2.5, 5.0, 7.0, 7.0, c);
            pen.frect(14.5, 9.0, 17.5, 11.0, c);
            pen.frect(3.5, 13.0, 6.0, 15.0, c);
            pen.frect(14.0, 13.0, 16.0, 15.0, c);
        }
        "sync" | "request" | "sendReceive" => pen.loop_arrows(c, a),
        // Send/Receive Progress: accounts one under another, how far each has got.
        "syncProgress" => {
            for (n, done) in [(0.0, 13.0), (1.0, 8.0), (2.0, 4.5)] {
                let y = 5.0 + n * 5.0;
                pen.rect(3.0, y, 17.0, y + 2.5, c);
                pen.frect(3.0, y, done, y + 2.5, a);
            }
        }
        "showAs" => {
            pen.rect(3.5, 4.0, 16.5, 16.0, c);
            pen.frect(3.5, 4.0, 7.0, 16.0, a);
        }
        // View
        "search" | "find" | "findRelated" => {
            pen.circle(8.5, 8.5, 5.0, c);
            pen.line_c(&[(12.3, 12.3), (17.0, 17.0)], a);
        }
        "filter" => pen.closed(&[(3.0, 4.0), (17.0, 4.0), (11.5, 10.5), (11.5, 16.5), (8.5, 14.8), (8.5, 10.5)], c),
        "readingPane" => {
            pen.rect(3.0, 4.0, 17.0, 16.0, c);
            pen.frect(10.5, 4.0, 17.0, 16.0, soft);
            pen.line_c(&[(10.5, 4.0), (10.5, 16.0)], a);
        }
        "folderPane" => {
            pen.rect(3.0, 4.0, 17.0, 16.0, c);
            pen.frect(3.0, 4.0, 7.5, 16.0, soft);
            pen.line_c(&[(7.5, 4.0), (7.5, 16.0)], a);
        }
        "conversations" => {
            pen.envelope(2.5, 3.5, 12.5, 10.0, c);
            pen.envelope(7.5, 10.0, 17.5, 16.5, a);
        }
        // The ribbon: a band under its tabs, with a row of commands in it.
        "ribbon" => {
            pen.rect(3.0, 7.0, 17.0, 15.0, c);
            pen.line(&[(4.0, 4.5), (8.0, 4.5)]);
            pen.line_c(&[(10.0, 4.5), (13.0, 4.5)], c.linear_multiply(0.45));
            for x in [6.5, 10.0, 13.5] {
                pen.frect(x - 1.0, 10.0, x + 1.0, 12.0, a);
            }
        }
        "sort" | "reverseSort" => {
            pen.line(&[(6.0, 4.0), (6.0, 16.0)]);
            pen.line(&[(3.5, 6.5), (6.0, 4.0), (8.5, 6.5)]);
            pen.line_c(&[(14.0, 4.0), (14.0, 16.0)], a);
            pen.line_c(&[(11.5, 13.5), (14.0, 16.0), (16.5, 13.5)], a);
        }
        "previewLines" | "list" => {
            for y in [5.0, 10.0, 15.0] {
                pen.fcircle(4.0, y, 1.0, a);
                pen.line(&[(7.0, y), (17.0, y)]);
            }
        }
        "reset" => {
            pen.arc(10.0, 10.0, 6.0, 30.0, 330.0, c);
            pen.fill(&[(14.0, 4.5), (17.8, 7.5), (13.6, 8.6)], c);
        }
        // Quick access and general
        "undo" => {
            pen.arc(10.5, 11.0, 5.5, 200.0, 20.0 + 360.0, c);
            pen.fill(&[(2.6, 6.0), (7.8, 6.6), (4.6, 10.8)], a);
        }
        "redo" => {
            pen.arc(9.5, 11.0, 5.5, 160.0, 340.0, c);
            pen.fill(&[(17.4, 6.0), (12.2, 6.6), (15.4, 10.8)], a);
        }
        "more" => {
            for x in [5.0, 10.0, 15.0] {
                pen.fcircle(x, 10.0, 1.3, c);
            }
        }
        "dropdown" => pen.fill(&[(4.5, 7.5), (15.5, 7.5), (10.0, 13.5)], c),
        "expanded" => pen.line(&[(5.0, 7.5), (10.0, 12.5), (15.0, 7.5)]),
        "collapsed" => pen.line(&[(7.5, 5.0), (12.5, 10.0), (7.5, 15.0)]),
        "launcher" => {
            pen.line(&[(5.0, 15.0), (15.0, 15.0), (15.0, 5.0)]);
            pen.line(&[(8.0, 8.0), (13.5, 13.5)]);
        }
        "plus" | "add" => pen.plus(10.0, 10.0, 5.0, c),
        "minus" => pen.line(&[(5.0, 10.0), (15.0, 10.0)]),
        "options" | "settings" => {
            pen.circle(10.0, 10.0, 3.0, c);
            for i in 0..8 {
                let ang = (i as f32 * 45.0).to_radians();
                pen.line(&[(10.0 + 5.0 * ang.cos(), 10.0 + 5.0 * ang.sin()), (10.0 + 7.2 * ang.cos(), 10.0 + 7.2 * ang.sin())]);
            }
        }
        "info" | "about" => {
            pen.circle(10.0, 10.0, 7.0, a);
            pen.fcircle(10.0, 6.5, 1.0, c);
            pen.line(&[(10.0, 9.0), (10.0, 14.0)]);
        }
        "help" => {
            pen.circle(10.0, 10.0, 7.0, a);
            pen.text(10.0, 10.2, 11.0, "?", c, 0.0);
        }
        "print" => {
            pen.rect(6.0, 3.0, 14.0, 7.0, c);
            pen.rect(3.5, 7.0, 16.5, 13.5, c);
            pen.frect(6.0, 11.5, 14.0, 17.0, soft);
            pen.rect(6.0, 11.5, 14.0, 17.0, a);
        }
        "theme" => {
            pen.circle(10.0, 10.0, 6.5, c);
            pen.p.add(Shape::convex_polygon(
                (0..=12)
                    .map(|i| (90.0 + i as f32 * 15.0).to_radians())
                    .map(|ang: f32| pen.pt(10.0 + 6.5 * ang.cos(), 10.0 + 6.5 * ang.sin()))
                    .collect(),
                c,
                Stroke::NONE,
            ));
        }
        "phone" => {
            pen.rect(6.0, 3.0, 14.0, 17.0, c);
            pen.line_c(&[(8.5, 14.5), (11.5, 14.5)], a);
        }
        "web" | "link" => {
            pen.circle(10.0, 10.0, 6.5, c);
            pen.line(&[(3.5, 10.0), (16.5, 10.0)]);
            pen.arc(10.0, 10.0, 6.5, -90.0, 90.0, Color32::TRANSPARENT);
            pen.line(&[(10.0, 3.5), (7.0, 10.0), (10.0, 16.5), (13.0, 10.0), (10.0, 3.5)]);
        }
        "address" | "location" => {
            pen.closed(&[(10.0, 17.0), (5.0, 9.5), (6.0, 5.5), (10.0, 3.5), (14.0, 5.5), (15.0, 9.5)], c);
            pen.fcircle(10.0, 8.5, 1.8, a);
        }
        "clock" | "time" => {
            pen.circle(10.0, 10.0, 6.5, c);
            pen.line_c(&[(10.0, 6.0), (10.0, 10.0), (13.0, 12.0)], a);
        }
        // A meeting held online: a camera, its lens to the right.
        "onlineMeeting" | "joinMeeting" => {
            pen.frect(2.5, 6.0, 12.0, 14.0, soft);
            pen.rect(2.5, 6.0, 12.0, 14.0, c);
            pen.fill(&[(12.8, 9.2), (17.5, 6.3), (17.5, 13.7), (12.8, 10.8)], a);
        }
        // Another time for a meeting: a clock, and an arrow on to a later hour.
        "proposeTime" => {
            pen.circle(8.0, 9.0, 5.5, c);
            pen.line(&[(8.0, 5.8), (8.0, 9.0), (10.3, 10.4)]);
            pen.line_c(&[(10.5, 16.0), (17.0, 16.0)], a);
            pen.line_c(&[(14.6, 13.7), (17.2, 16.0), (14.6, 18.3)], a);
        }
        // What the people invited are asked for: one line ticked, one with a question.
        "responseOptions" => {
            pen.check(5.5, 6.5, 0.8, green);
            pen.line(&[(10.0, 6.5), (17.0, 6.5)]);
            pen.text(5.5, 13.5, 9.5, "?", a, 0.0);
            pen.line(&[(10.0, 13.5), (17.0, 13.5)]);
        }
        // The world with its meridians, and the hour a place on it has.
        "timeZones" | "globe" => {
            pen.circle(9.0, 10.0, 6.0, c);
            pen.line(&[(3.0, 10.0), (15.0, 10.0)]);
            pen.line(&[(9.0, 4.0), (6.9, 6.6), (6.3, 10.0), (6.9, 13.4), (9.0, 16.0)]);
            pen.line(&[(9.0, 4.0), (11.1, 6.6), (11.7, 10.0), (11.1, 13.4), (9.0, 16.0)]);
            pen.fcircle(14.6, 14.6, 3.6, soft);
            pen.circle(14.6, 14.6, 3.6, a);
            pen.line_c(&[(14.6, 12.6), (14.6, 14.6), (16.0, 15.4)], a);
        }
        "notes" | "description" => {
            pen.rect(4.0, 3.0, 16.0, 17.0, c);
            for y in [7.0, 10.0, 13.0] {
                pen.line(&[(6.5, y), (13.5, y)]);
            }
        }
        // Account setup
        "work" => {
            pen.rect(3.0, 7.0, 17.0, 16.0, c);
            pen.line(&[(7.5, 7.0), (7.5, 4.5), (12.5, 4.5), (12.5, 7.0)]);
            pen.line_c(&[(3.0, 11.0), (8.5, 11.0)], a);
            pen.line_c(&[(11.5, 11.0), (17.0, 11.0)], a);
            pen.rect(8.5, 10.0, 11.5, 12.5, a);
        }
        "server" => {
            for y in [4.0, 11.0] {
                pen.rect(3.5, y, 16.5, y + 5.0, c);
                pen.fcircle(6.5, y + 2.5, 1.0, a);
                pen.line(&[(10.0, y + 2.5), (14.0, y + 2.5)]);
            }
        }
        "device" => {
            pen.rect(3.0, 4.0, 17.0, 13.0, c);
            pen.line(&[(10.0, 13.0), (10.0, 16.0)]);
            pen.line(&[(6.5, 16.0), (13.5, 16.0)]);
            pen.frect(5.5, 6.5, 14.5, 10.5, soft);
        }
        "tick" => pen.check(10.0, 10.5, 1.5, c),
        // Formatting the text of a message being written
        "bold" => pen.text(10.0, 10.3, 15.0, "B", c, 0.0),
        "italic" => {
            pen.line(&[(11.8, 4.0), (8.2, 16.0)]);
            pen.line(&[(9.0, 4.0), (14.5, 4.0)]);
            pen.line(&[(5.5, 16.0), (11.0, 16.0)]);
        }
        "underline" => {
            pen.line(&[(6.0, 3.5), (6.0, 10.0)]);
            pen.arc(10.0, 10.0, 4.0, 0.0, 180.0, c);
            pen.line(&[(14.0, 3.5), (14.0, 10.0)]);
            pen.line_c(&[(4.5, 17.0), (15.5, 17.0)], a);
        }
        "fontSize" => {
            pen.text(7.5, 10.5, 15.0, "A", c, 0.0);
            pen.text(15.5, 12.5, 9.0, "A", c, 0.0);
        }
        "font" => {
            pen.text(7.0, 10.5, 14.0, "A", c, 0.0);
            pen.text(14.5, 12.0, 10.0, "a", a, 0.0);
        }
        "fontColor" => {
            pen.text(10.0, 8.5, 13.0, "A", c, 0.0);
            pen.frect(3.5, 15.0, 16.5, 18.0, a);
        }
        "bullets" => {
            for y in [5.0, 10.0, 15.0] {
                pen.fcircle(4.5, y, 1.4, a);
                pen.line(&[(8.0, y), (17.0, y)]);
            }
        }
        "numbering" => {
            for (y, n) in [(5.0, "1"), (10.0, "2"), (15.0, "3")] {
                pen.text(4.5, y, 6.0, n, a, 0.0);
                pen.line(&[(8.0, y), (17.0, y)]);
            }
        }
        "insertLink" => {
            // Two links of a chain.
            let s = pen.s();
            let link = |x0: f32, x1: f32, color: Color32| {
                pen.p.rect_stroke(Rect::from_min_max(pen.pt(x0, 7.0), pen.pt(x1, 13.0)), 3.0 * s, pen.stroke(color), egui::StrokeKind::Middle);
            };
            link(2.5, 11.5, c);
            link(8.5, 17.5, a);
        }
        "clearFormatting" => {
            pen.text(8.0, 9.5, 13.0, "A", c, 0.0);
            pen.line_c(&[(10.5, 17.0), (17.0, 10.5)], red);
            pen.line_c(&[(10.5, 10.5), (17.0, 17.0)], red);
        }
        "formatHtml" => pen.text(10.0, 10.0, 8.5, "</>", c, 0.0),
        "increaseIndent" | "decreaseIndent" => {
            pen.line(&[(3.0, 4.0), (17.0, 4.0)]);
            pen.line(&[(9.5, 8.0), (17.0, 8.0)]);
            pen.line(&[(9.5, 12.0), (17.0, 12.0)]);
            pen.line(&[(3.0, 16.0), (17.0, 16.0)]);
            if name == "increaseIndent" {
                pen.fill(&[(3.0, 7.0), (7.0, 10.0), (3.0, 13.0)], a);
            } else {
                pen.fill(&[(7.0, 7.0), (3.0, 10.0), (7.0, 13.0)], a);
            }
        }
        "formatPlain" => {
            pen.line(&[(4.0, 5.5), (16.0, 5.5)]);
            pen.line(&[(4.0, 10.0), (16.0, 10.0)]);
            pen.line(&[(4.0, 14.5), (11.5, 14.5)]);
        }
        // Anything else: a lettered tile, so a missing drawing is visible rather than empty.
        _ => {
            pen.rect(3.5, 3.5, 16.5, 16.5, c);
            let letter: String = name.chars().next().map(|ch| ch.to_uppercase().collect()).unwrap_or_default();
            pen.text(10.0, 10.3, 9.5, &letter, c, 0.0);
        }
    }
}

impl Pen<'_> {
    /// A small folder with its top-left corner at (`x`, `y`).
    fn folder_small(&self, x: f32, y: f32, c: Color32) {
        self.closed(&[(x, y), (x + 2.2, y), (x + 3.0, y + 1.2), (x + 5.5, y + 1.2), (x + 5.5, y + 5.5), (x, y + 5.5)], c);
    }
}

/// The names [`paint`] draws (not a lettered tile); tests check ribbon icons against it.
pub const NAMES: &[&str] = &[
    "mail",
    "email",
    "unread",
    "markUnread",
    "new",
    "newMail",
    "read",
    "markRead",
    "toggleRead",
    "markAllRead",
    "reply",
    "replyAll",
    "forward",
    "delete",
    "deleted",
    "discard",
    "deletePermanently",
    "remove",
    "empty",
    "emptyFolder",
    "archive",
    "junk",
    "notJunk",
    "move",
    "copy",
    "flag",
    "toggleFlag",
    "followUp",
    "clearFlag",
    "complete",
    "check",
    "accept",
    "categorize",
    "category",
    "categories",
    "pin",
    "rules",
    "createRule",
    "attach",
    "attachment",
    "send",
    "sent",
    "drafts",
    "editDraft",
    "edit",
    "rename",
    "inbox",
    "outbox",
    "export",
    "import",
    "open",
    "folder",
    "custom",
    "select",
    "folderNew",
    "newFolder",
    "favorite",
    "star",
    "starOn",
    "save",
    "saveDraft",
    "highImportance",
    "importanceHigh",
    "lowImportance",
    "importanceLow",
    "signature",
    "checkNames",
    "close",
    "decline",
    "cancel",
    "tentative",
    "calendar",
    "appointment",
    "newAppointment",
    "newEvent",
    "today",
    "day",
    "workWeek",
    "week",
    "month",
    "people",
    "contact",
    "account",
    "newContact",
    "meeting",
    "newMeeting",
    "fromMessage",
    "meetingRequest",
    "meetingCancelled",
    "meetingAccepted",
    "meetingTentative",
    "meetingDeclined",
    "tasks",
    "task",
    "newTask",
    "next",
    "forwardPeriod",
    "previous",
    "back",
    "reminder",
    "bell",
    "private",
    "lock",
    "recurrence",
    "repeat",
    "scheduling",
    "sync",
    "syncProgress",
    "request",
    "sendReceive",
    "showAs",
    "search",
    "find",
    "findRelated",
    "filter",
    "readingPane",
    "folderPane",
    "conversations",
    "ribbon",
    "sort",
    "reverseSort",
    "previewLines",
    "list",
    "reset",
    "undo",
    "redo",
    "more",
    "dropdown",
    "expanded",
    "collapsed",
    "launcher",
    "plus",
    "add",
    "minus",
    "options",
    "settings",
    "info",
    "about",
    "help",
    "print",
    "theme",
    "phone",
    "web",
    "link",
    "address",
    "location",
    "clock",
    "time",
    "onlineMeeting",
    "joinMeeting",
    "proposeTime",
    "responseOptions",
    "timeZones",
    "globe",
    "notes",
    "description",
    "work",
    "server",
    "device",
    "tick",
    "bold",
    "italic",
    "underline",
    "fontSize",
    "font",
    "fontColor",
    "bullets",
    "numbering",
    "insertLink",
    "clearFormatting",
    "formatHtml",
    "formatPlain",
    "increaseIndent",
    "decreaseIndent",
];
