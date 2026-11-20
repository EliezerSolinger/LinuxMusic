//! A small software painter with the semantics of the QPainter calls the legacy widgets make (antialiased lines with
//! square or round caps, circular arcs in Qt's angles, a radial gradient pen, pixmaps drawn over), so a control is
//! rasterized once per visible state into one image, like the legacy `Knob::m_cache`, and the GPU draws one image
//! per control (the cost in GPUI is per primitive: docs/prototipo-interface.md, 2.2).
//!
//! Geometry is given in the widget's logical pixels; the painter works at the screen's scale (`scale`), so lines
//! and arcs are sharp on a 2x screen and the pixmaps are resampled (bilinear), as Qt draws a 1x pixmap on a 2x
//! screen.
//!
//! Coverage is the 1-pixel ramp of the signed distance to the shape at each pixel's center, which is what Qt's
//! antialiased rasterizer gives for strokes of 1 to 3 pixels.

use std::sync::Arc;

use gpui::RenderImage;
use image::{Frame, RgbaImage};

/// A color, straight alpha, 0..1
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rgba {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Rgba {
    pub const fn rgb(r: u8, g: u8, b: u8) -> Rgba {
        Rgba { r: r as f32 / 255.0, g: g as f32 / 255.0, b: b as f32 / 255.0, a: 1.0 }
    }

    pub const fn alpha(r: u8, g: u8, b: u8, a: u8) -> Rgba {
        Rgba { r: r as f32 / 255.0, g: g as f32 / 255.0, b: b as f32 / 255.0, a: a as f32 / 255.0 }
    }

    /// `0xRRGGBB`, opaque
    pub const fn hex(v: u32) -> Rgba {
        Rgba::rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
    }

    fn lerp(self, o: Rgba, t: f32) -> Rgba {
        Rgba {
            r: self.r + (o.r - self.r) * t,
            g: self.g + (o.g - self.g) * t,
            b: self.b + (o.b - self.b) * t,
            a: self.a + (o.a - self.a) * t,
        }
    }

    /// The legacy `convertPixmapToGrayScale` weights
    pub fn gray(self) -> Rgba {
        let g = 0.2126 * self.r + 0.7152 * self.g + 0.0722 * self.b;
        Rgba { r: g, g, b: g, a: self.a }
    }
}

/// How a pen paints: one color, or Qt's `QRadialGradient(center, radius)` with stops (position 0..1, color)
#[derive(Clone, Debug, PartialEq)]
pub enum Paint {
    Solid(Rgba),
    Radial { cx: f32, cy: f32, radius: f32, stops: Vec<(f32, Rgba)> },
}

impl Paint {
    fn at(&self, x: f32, y: f32) -> Rgba {
        match self {
            Paint::Solid(c) => *c,
            Paint::Radial { cx, cy, radius, stops } => {
                let t = if *radius > 0.0 { ((x - cx).hypot(y - cy) / radius).clamp(0.0, 1.0) } else { 1.0 };
                let Some(first) = stops.first() else { return Rgba::alpha(0, 0, 0, 0) };
                if t <= first.0 {
                    return first.1;
                }
                for w in stops.windows(2) {
                    let ((a, ca), (b, cb)) = (w[0], w[1]);
                    if t <= b {
                        return ca.lerp(cb, if b > a { (t - a) / (b - a) } else { 1.0 });
                    }
                }
                stops.last().map_or(first.1, |s| s.1)
            }
        }
    }
}

/// Line ends (Qt's `Qt::PenCapStyle`; `QPen(color, width)` is `Square`)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cap {
    Flat,
    Square,
    Round,
}

/// A premultiplied RGBA canvas at `scale` device pixels per logical pixel
#[derive(Debug)]
pub struct Canvas {
    pub width: usize,
    pub height: usize,
    pub scale: f32,
    px: Vec<[f32; 4]>,
}

impl Canvas {
    /// A transparent canvas of `w`×`h` logical pixels
    pub fn new(w: f32, h: f32, scale: f32) -> Canvas {
        let width = (w * scale).ceil().max(1.0) as usize;
        let height = (h * scale).ceil().max(1.0) as usize;
        Canvas { width, height, scale, px: vec![[0.0; 4]; width * height] }
    }

    fn blend(&mut self, x: usize, y: usize, c: Rgba, coverage: f32) {
        let a = c.a * coverage;
        if a <= 0.0 {
            return;
        }
        let p = &mut self.px[y * self.width + x];
        let k = 1.0 - a;
        p[0] = c.r * a + p[0] * k;
        p[1] = c.g * a + p[1] * k;
        p[2] = c.b * a + p[2] * k;
        p[3] = a + p[3] * k;
    }

    /// Fills `paint` where `dist(x, y)` (signed distance in device pixels to the shape's edge, negative inside) is
    /// below half a pixel, in the device rectangle `x0..x1`, `y0..y1`
    fn cover(&mut self, bx: (f32, f32), by: (f32, f32), paint: &Paint, dist: impl Fn(f32, f32) -> f32) {
        let x0 = (bx.0.floor().max(0.0)) as usize;
        let x1 = (bx.1.ceil().max(0.0) as usize).min(self.width);
        let y0 = (by.0.floor().max(0.0)) as usize;
        let y1 = (by.1.ceil().max(0.0) as usize).min(self.height);
        let s = self.scale;
        for y in y0..y1 {
            for x in x0..x1 {
                let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
                let cov = (0.5 - dist(fx, fy)).clamp(0.0, 1.0);
                if cov > 0.0 {
                    // the paint is in logical coordinates
                    let c = paint.at(fx / s, fy / s);
                    self.blend(x, y, c, cov);
                }
            }
        }
    }

    /// `QPainter::drawLine` with an antialiased pen of `width` logical pixels
    pub fn line(&mut self, a: (f32, f32), b: (f32, f32), width: f32, cap: Cap, paint: &Paint) {
        let s = self.scale;
        let (ax, ay, bx, by) = (a.0 * s, a.1 * s, b.0 * s, b.1 * s);
        let hw = (width.max(1.0) * s) / 2.0;
        let (dx, dy) = (bx - ax, by - ay);
        let len = dx.hypot(dy);
        let (ux, uy) = if len > 0.0 { (dx / len, dy / len) } else { (1.0, 0.0) };
        let ext = match cap {
            Cap::Flat => 0.0,
            Cap::Square | Cap::Round => hw,
        };
        let pad = hw + ext + 1.0;
        let bxr = (ax.min(bx) - pad, ax.max(bx) + pad);
        let byr = (ay.min(by) - pad, ay.max(by) + pad);
        self.cover(bxr, byr, paint, |x, y| {
            let (px, py) = (x - ax, y - ay);
            let along = px * ux + py * uy;
            let across = (px * -uy + py * ux).abs();
            match cap {
                Cap::Round => {
                    let t = along.clamp(0.0, len);
                    (px - ux * t).hypot(py - uy * t) - hw
                }
                Cap::Flat | Cap::Square => {
                    let lo = -ext;
                    let hi = len + ext;
                    let d_along = (lo - along).max(along - hi);
                    (across - hw).max(d_along)
                }
            }
        });
    }

    /// `QPainter::drawArc(x, y, w, h, start, span)` on a circle (w == h), angles in degrees as Qt counts them
    /// (0 at three o'clock, counter-clockwise positive), with a pen of `width` and `cap`
    #[allow(clippy::too_many_arguments)]
    pub fn arc(&mut self, x: f32, y: f32, d: f32, start: f32, span: f32, width: f32, cap: Cap, color: Rgba) {
        if span == 0.0 {
            return;
        }
        let s = self.scale;
        let r = d / 2.0 * s;
        let (cx, cy) = ((x + d / 2.0) * s, (y + d / 2.0) * s);
        let hw = (width.max(1.0) * s) / 2.0;
        // the arc as a range of angles, start..start+span with span > 0 (Qt's y axis points up)
        let (a0, a1) = if span > 0.0 { (start, start + span) } else { (start + span, start) };
        let ext = if cap == Cap::Flat || r <= 0.0 { 0.0 } else { (hw / r).to_degrees() };
        let (a0, a1) = (a0 - ext, a1 + ext);
        let pad = r + hw + 1.0;
        let paint = Paint::Solid(color);
        self.cover((cx - pad, cx + pad), (cy - pad, cy + pad), &paint, |px, py| {
            let (dx, dy) = (px - cx, cy - py);
            let ring = (dx.hypot(dy) - r).abs() - hw;
            let ang = dy.atan2(dx).to_degrees();
            // the angle, brought into a0..a0+360
            let mut t = ang;
            while t < a0 {
                t += 360.0;
            }
            while t >= a0 + 360.0 {
                t -= 360.0;
            }
            if t <= a1 {
                ring
            } else {
                // outside the angular range: distance to the nearer end, along the circle
                let past = ((t - a1).min(a0 + 360.0 - t)).to_radians() * r;
                ring.max(past)
            }
        });
    }

    /// `QPainter::fillRect` / an axis-aligned rectangle in logical pixels
    pub fn fill_rect(&mut self, x: f32, y: f32, w: f32, h: f32, color: Rgba) {
        let s = self.scale;
        let (x0, y0, x1, y1) = (x * s, y * s, (x + w) * s, (y + h) * s);
        let paint = Paint::Solid(color);
        self.cover((x0 - 1.0, x1 + 1.0), (y0 - 1.0, y1 + 1.0), &paint, |px, py| {
            let dx = (x0 - px).max(px - x1);
            let dy = (y0 - py).max(py - y1);
            dx.max(dy)
        });
    }

    /// `QPainter::drawPixmap(x, y, pixmap)`: the pixmap at its logical size (1 pixel = 1 logical pixel), resampled
    /// to the canvas's scale. `gray`: the disabled look (`convertPixmapToGrayScale`)
    pub fn pixmap(&mut self, img: &RgbaImage, x: f32, y: f32, gray: bool) {
        self.pixmap_part(img, (0, 0, img.width(), img.height()), x, y, gray);
    }

    /// Draws the source rectangle `src` (x, y, w, h in image pixels) of `img` at `x`, `y`
    pub fn pixmap_part(&mut self, img: &RgbaImage, src: (u32, u32, u32, u32), x: f32, y: f32, gray: bool) {
        let s = self.scale;
        let (sx, sy, sw, sh) = src;
        let x0 = (x * s).round() as i64;
        let y0 = (y * s).round() as i64;
        let dw = (sw as f32 * s).round() as i64;
        let dh = (sh as f32 * s).round() as i64;
        let sample = |u: f32, v: f32| -> [f32; 4] {
            // bilinear over the source rectangle, premultiplied, clamped to its edges
            let u = (u - 0.5).clamp(0.0, sw as f32 - 1.0);
            let v = (v - 0.5).clamp(0.0, sh as f32 - 1.0);
            let (iu, iv) = (u.floor() as u32, v.floor() as u32);
            let (fu, fv) = (u - iu as f32, v - iv as f32);
            let mut acc = [0.0f32; 4];
            for (du, dv, w) in
                [(0, 0, (1.0 - fu) * (1.0 - fv)), (1, 0, fu * (1.0 - fv)), (0, 1, (1.0 - fu) * fv), (1, 1, fu * fv)]
            {
                if w == 0.0 {
                    continue;
                }
                let px = img.get_pixel(sx + (iu + du).min(sw - 1), sy + (iv + dv).min(sh - 1)).0;
                let a = px[3] as f32 / 255.0;
                acc[0] += px[0] as f32 / 255.0 * a * w;
                acc[1] += px[1] as f32 / 255.0 * a * w;
                acc[2] += px[2] as f32 / 255.0 * a * w;
                acc[3] += a * w;
            }
            acc
        };
        for dy in 0..dh {
            let ty = y0 + dy;
            if ty < 0 || ty >= self.height as i64 {
                continue;
            }
            for dx in 0..dw {
                let tx = x0 + dx;
                if tx < 0 || tx >= self.width as i64 {
                    continue;
                }
                let p = if s == 1.0 {
                    let px = img.get_pixel(sx + dx as u32, sy + dy as u32).0;
                    let a = px[3] as f32 / 255.0;
                    [px[0] as f32 / 255.0 * a, px[1] as f32 / 255.0 * a, px[2] as f32 / 255.0 * a, a]
                } else {
                    sample((dx as f32 + 0.5) / s, (dy as f32 + 0.5) / s)
                };
                if p[3] <= 0.0 {
                    continue;
                }
                let mut src = p;
                if gray {
                    let a = p[3];
                    let c = Rgba { r: p[0] / a, g: p[1] / a, b: p[2] / a, a }.gray();
                    src = [c.r * a, c.g * a, c.b * a, a];
                }
                let d = &mut self.px[ty as usize * self.width + tx as usize];
                let k = 1.0 - src[3];
                for i in 0..4 {
                    d[i] = src[i] + d[i] * k;
                }
            }
        }
    }

    /// The canvas as a GPUI image (BGRA, straight alpha)
    pub fn into_image(self) -> Arc<RenderImage> {
        let mut buf = RgbaImage::new(self.width as u32, self.height as u32);
        for (i, p) in self.px.iter().enumerate() {
            let a = p[3].clamp(0.0, 1.0);
            let un = |v: f32| if a > 0.0 { ((v / a).clamp(0.0, 1.0) * 255.0 + 0.5) as u8 } else { 0 };
            let x = (i % self.width) as u32;
            let y = (i / self.width) as u32;
            buf.put_pixel(x, y, image::Rgba([un(p[2]), un(p[1]), un(p[0]), (a * 255.0 + 0.5) as u8]));
        }
        Arc::new(RenderImage::new(vec![Frame::new(buf)]))
    }

    /// Draws another canvas of the same scale over this one at logical `x`, `y` (the snapshots)
    pub fn composite(&mut self, other: &Canvas, x: f32, y: f32) {
        let (ox, oy) = ((x * self.scale).round() as usize, (y * self.scale).round() as usize);
        for yy in 0..other.height {
            for xx in 0..other.width {
                let (tx, ty) = (ox + xx, oy + yy);
                if tx >= self.width || ty >= self.height {
                    continue;
                }
                let s = other.px[yy * other.width + xx];
                let d = &mut self.px[ty * self.width + tx];
                let k = 1.0 - s[3];
                for i in 0..4 {
                    d[i] = s[i] + d[i] * k;
                }
            }
        }
    }

    /// Writes the canvas as an RGBA PNG (the snapshots)
    pub fn save_png(&self, path: &str) {
        let mut buf = RgbaImage::new(self.width as u32, self.height as u32);
        for (i, p) in self.px.iter().enumerate() {
            let a = p[3].clamp(0.0, 1.0);
            let un = |v: f32| if a > 0.0 { ((v / a).clamp(0.0, 1.0) * 255.0 + 0.5) as u8 } else { 0 };
            buf.put_pixel((i % self.width) as u32, (i / self.width) as u32, image::Rgba([un(p[0]), un(p[1]), un(p[2]), (a * 255.0 + 0.5) as u8]));
        }
        let _ = buf.save_with_format(path, image::ImageFormat::Png);
    }

    /// The premultiplied pixel at device `x`, `y` (the tests read it)
    pub fn pixel(&self, x: usize, y: usize) -> [f32; 4] {
        self.px[y * self.width + x]
    }
}

/// Decodes an embedded PNG (the legacy artwork, copied into `assets/`)
pub fn decode_png(bytes: &[u8]) -> RgbaImage {
    match image::load_from_memory_with_format(bytes, image::ImageFormat::Png) {
        Ok(img) => img.into_rgba8(),
        // the assets are ours and checked by a test; a broken one draws nothing rather than taking the UI down
        Err(_) => RgbaImage::new(1, 1),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_covers_its_width_and_not_beyond() {
        let mut c = Canvas::new(20.0, 20.0, 1.0);
        c.line((2.0, 10.0), (18.0, 10.0), 2.0, Cap::Flat, &Paint::Solid(Rgba::rgb(255, 0, 0)));
        assert!(c.pixel(10, 9)[3] > 0.99 && c.pixel(10, 10)[3] > 0.99);
        assert!(c.pixel(10, 12)[3] < 0.01 && c.pixel(10, 7)[3] < 0.01);
        assert!(c.pixel(0, 10)[3] < 0.01);
    }

    #[test]
    fn an_arc_follows_qt_angles() {
        // a quarter from 0 (three o'clock) counter-clockwise to twelve o'clock
        let mut c = Canvas::new(40.0, 40.0, 1.0);
        c.arc(0.0, 0.0, 40.0, 0.0, 90.0, 2.0, Cap::Flat, Rgba::rgb(255, 255, 255));
        let right = c.pixel(39, 19)[3] + c.pixel(39, 20)[3];
        let top = c.pixel(19, 0)[3] + c.pixel(20, 0)[3];
        let left = c.pixel(0, 19)[3] + c.pixel(0, 20)[3];
        let bottom = c.pixel(19, 39)[3] + c.pixel(20, 39)[3];
        assert!(right > 0.5 && top > 0.5, "{right} {top}");
        assert!(left < 0.01 && bottom < 0.01, "{left} {bottom}");
        // diagonal up-right is on it
        let k = 20.0 + 19.0 * std::f32::consts::FRAC_1_SQRT_2;
        assert!(c.pixel(k as usize, (40.0 - k) as usize)[3] > 0.2);
    }

    #[test]
    fn the_radial_gradient_goes_from_the_inner_color_to_the_outer() {
        let p = Paint::Radial {
            cx: 0.0,
            cy: 0.0,
            radius: 10.0,
            stops: vec![(0.4, Rgba::rgb(255, 0, 0)), (1.0, Rgba::rgb(0, 0, 255))],
        };
        assert_eq!(p.at(2.0, 0.0), Rgba::rgb(255, 0, 0));
        assert_eq!(p.at(10.0, 0.0), Rgba::rgb(0, 0, 255));
        let mid = p.at(7.0, 0.0);
        assert!((mid.r - 0.5).abs() < 0.01 && (mid.b - 0.5).abs() < 0.01);
    }

    #[test]
    fn a_pixmap_is_drawn_at_its_size_on_any_scale() {
        let mut img = RgbaImage::new(4, 2);
        for p in img.pixels_mut() {
            *p = image::Rgba([0, 255, 0, 255]);
        }
        let mut c = Canvas::new(10.0, 10.0, 2.0);
        c.pixmap(&img, 1.0, 1.0, false);
        assert!(c.pixel(2, 2)[1] > 0.99 && c.pixel(9, 5)[1] > 0.99);
        assert!(c.pixel(10, 2)[3] < 0.01 && c.pixel(2, 6)[3] < 0.01);
    }
}
