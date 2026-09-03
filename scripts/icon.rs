//! Renders the application icon.
//!
//! Standalone: compile with `rustc -O scripts/icon.rs -o <tmp>/icon` and run it
//! with an output directory. It has no dependencies and is not part of the
//! crate, so building the app never needs it. `scripts/make-icon.sh` drives it
//! and hands the result to `iconutil`.
//!
//! THE ICON IS SOURCE CODE, not an asset someone drew once and lost the file
//! for. The geometry below is the definition; everything else here is a small
//! renderer for it. Changing the mark means editing `ICON` and re-running.
//!
//! Rendering is a scalar painter's algorithm over a supersampled grid: for each
//! output pixel, `SS * SS` sample points are each pushed through the whole op
//! list, then averaged. That gives real antialiasing without a rasteriser, and
//! it is fast enough because every op carries a bounding box and most samples
//! fall outside most ops.

use std::io::Write;

// ---------------------------------------------------------------- geometry

// The full primitive set is the palette the icon is designed against, so it
// stays available whether or not the current mark happens to use every shape.
#[allow(dead_code)]
#[derive(Clone, Copy)]
enum Op {
    /// Fill the whole canvas.
    Fill { c: f32 },
    /// Superellipse tile: |2x/w - 1|^n + |2y/h - 1|^n <= 1. n = 5 is close to
    /// the shape Apple uses for application icons.
    Squircle { x: f32, y: f32, w: f32, h: f32, n: f32, c: f32 },
    Rect { x: f32, y: f32, w: f32, h: f32, c: f32 },
    RRect { x: f32, y: f32, w: f32, h: f32, r: f32, c: f32 },
    Circle { cx: f32, cy: f32, r: f32, c: f32 },
    Ellipse { cx: f32, cy: f32, rx: f32, ry: f32, c: f32 },
    /// Rectangle with semicircular ends on its short axis.
    Capsule { x: f32, y: f32, w: f32, h: f32, c: f32 },
    Line { x1: f32, y1: f32, x2: f32, y2: f32, w: f32, c: f32 },
    Ring { cx: f32, cy: f32, r: f32, w: f32, c: f32 },
    /// Degrees, 0 at +x, increasing clockwise (y is down).
    Arc { cx: f32, cy: f32, r: f32, a0: f32, a1: f32, w: f32, c: f32 },
    Poly { pts: &'static [(f32, f32)], c: f32 },
}

/// Alpha channel value for a colour. Everything is opaque except the canvas
/// itself, which starts clear so the tile's corners stay transparent.
const CLEAR: f32 = -1.0;

#[allow(dead_code)]
const BLACK: f32 = 0.0;
#[allow(dead_code)]
const WHITE: f32 = 1.0;
#[allow(dead_code)]
const GREY: f32 = 0.5;

impl Op {
    /// Axis-aligned bounds, generously padded. Only used to skip samples.
    fn bounds(&self) -> (f32, f32, f32, f32) {
        match *self {
            Op::Fill { .. } => (-1.0, -1.0, 2.0, 2.0),
            Op::Squircle { x, y, w, h, .. } | Op::Rect { x, y, w, h, .. } => (x, y, w, h),
            Op::RRect { x, y, w, h, .. } | Op::Capsule { x, y, w, h, .. } => (x, y, w, h),
            Op::Circle { cx, cy, r, .. } => (cx - r, cy - r, 2.0 * r, 2.0 * r),
            Op::Ellipse { cx, cy, rx, ry, .. } => (cx - rx, cy - ry, 2.0 * rx, 2.0 * ry),
            Op::Line { x1, y1, x2, y2, w, .. } => {
                let h = w * 0.5;
                let (lo_x, hi_x) = (x1.min(x2) - h, x1.max(x2) + h);
                let (lo_y, hi_y) = (y1.min(y2) - h, y1.max(y2) + h);
                (lo_x, lo_y, hi_x - lo_x, hi_y - lo_y)
            }
            Op::Ring { cx, cy, r, w, .. } | Op::Arc { cx, cy, r, w, .. } => {
                let o = r + w * 0.5;
                (cx - o, cy - o, 2.0 * o, 2.0 * o)
            }
            Op::Poly { pts, .. } => {
                let mut b = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
                for &(x, y) in pts {
                    b.0 = b.0.min(x);
                    b.1 = b.1.min(y);
                    b.2 = b.2.max(x);
                    b.3 = b.3.max(y);
                }
                (b.0, b.1, b.2 - b.0, b.3 - b.1)
            }
        }
    }

    fn colour(&self) -> f32 {
        match *self {
            Op::Fill { c }
            | Op::Squircle { c, .. }
            | Op::Rect { c, .. }
            | Op::RRect { c, .. }
            | Op::Circle { c, .. }
            | Op::Ellipse { c, .. }
            | Op::Capsule { c, .. }
            | Op::Line { c, .. }
            | Op::Ring { c, .. }
            | Op::Arc { c, .. }
            | Op::Poly { c, .. } => c,
        }
    }

    fn covers(&self, px: f32, py: f32) -> bool {
        match *self {
            Op::Fill { .. } => true,
            Op::Squircle { x, y, w, h, n, .. } => {
                let u = ((px - x) / w * 2.0 - 1.0).abs();
                let v = ((py - y) / h * 2.0 - 1.0).abs();
                u.powf(n) + v.powf(n) <= 1.0
            }
            Op::Rect { x, y, w, h, .. } => px >= x && px <= x + w && py >= y && py <= y + h,
            Op::RRect { x, y, w, h, r, .. } => rrect(px, py, x, y, w, h, r),
            Op::Capsule { x, y, w, h, .. } => {
                rrect(px, py, x, y, w, h, w.min(h) * 0.5)
            }
            Op::Circle { cx, cy, r, .. } => {
                let (dx, dy) = (px - cx, py - cy);
                dx * dx + dy * dy <= r * r
            }
            Op::Ellipse { cx, cy, rx, ry, .. } => {
                let (dx, dy) = ((px - cx) / rx, (py - cy) / ry);
                dx * dx + dy * dy <= 1.0
            }
            Op::Line { x1, y1, x2, y2, w, .. } => seg_dist(px, py, x1, y1, x2, y2) <= w * 0.5,
            Op::Ring { cx, cy, r, w, .. } => {
                let d = ((px - cx).powi(2) + (py - cy).powi(2)).sqrt();
                (d - r).abs() <= w * 0.5
            }
            Op::Arc { cx, cy, r, a0, a1, w, .. } => {
                let (dx, dy) = (px - cx, py - cy);
                let d = (dx * dx + dy * dy).sqrt();
                if (d - r).abs() > w * 0.5 {
                    return false;
                }
                // y is down, so a positive angle already runs clockwise on screen.
                let mut a = dy.atan2(dx).to_degrees();
                if a < 0.0 {
                    a += 360.0;
                }
                let (mut lo, mut hi) = (a0.rem_euclid(360.0), a1.rem_euclid(360.0));
                if hi <= lo {
                    hi += 360.0;
                }
                if a < lo {
                    lo -= 360.0;
                    hi -= 360.0;
                }
                a >= lo && a <= hi
            }
            Op::Poly { pts, .. } => {
                // Crossing number. Comparing on the half-open interval keeps a
                // sample sitting exactly on a shared vertex from counting twice.
                let mut inside = false;
                let n = pts.len();
                for i in 0..n {
                    let (xi, yi) = pts[i];
                    let (xj, yj) = pts[(i + n - 1) % n];
                    if (yi > py) != (yj > py) {
                        let t = (py - yi) / (yj - yi);
                        if px < xi + t * (xj - xi) {
                            inside = !inside;
                        }
                    }
                }
                inside
            }
        }
    }
}

fn rrect(px: f32, py: f32, x: f32, y: f32, w: f32, h: f32, r: f32) -> bool {
    let r = r.min(w * 0.5).min(h * 0.5);
    if px < x || px > x + w || py < y || py > y + h {
        return false;
    }
    // Outside the corner squares it is a plain rectangle. The bounds are
    // ordered explicitly because a capsule has r exactly equal to half its
    // short side, and float rounding can then put the low bound a hair above
    // the high one, which clamp treats as a programming error and panics on.
    let (lo_x, hi_x) = (x + r, (x + w - r).max(x + r));
    let (lo_y, hi_y) = (y + r, (y + h - r).max(y + r));
    let cx = px.clamp(lo_x, hi_x);
    let cy = py.clamp(lo_y, hi_y);
    let (dx, dy) = (px - cx, py - cy);
    dx * dx + dy * dy <= r * r
}

fn seg_dist(px: f32, py: f32, x1: f32, y1: f32, x2: f32, y2: f32) -> f32 {
    let (vx, vy) = (x2 - x1, y2 - y1);
    let len2 = vx * vx + vy * vy;
    let t = if len2 <= 0.0 {
        0.0
    } else {
        (((px - x1) * vx + (py - y1) * vy) / len2).clamp(0.0, 1.0)
    };
    let (dx, dy) = (px - (x1 + t * vx), py - (y1 + t * vy));
    (dx * dx + dy * dy).sqrt()
}

// ---------------------------------------------------------------- the icon

include!("icon_art.rs");

// ---------------------------------------------------------------- rendering

/// Samples per axis per output pixel. 8 gives 64 samples, so 65 levels of edge
/// coverage, which is past the point where banding is visible on a curve.
const SS: usize = 8;

/// The master is rendered once at this size and box-filtered down to the rest.
/// Every size an iconset needs divides it exactly, so no resampling is ever
/// asked to interpolate.
const MASTER: usize = 1024;

/// Renders to a straight (non-premultiplied) RGBA8 buffer.
fn render(ops: &[Op], size: usize) -> Vec<u8> {
    let mut out = vec![0u8; size * size * 4];
    let inv = 1.0 / size as f32;
    let sub = 1.0 / (SS as f32 * size as f32);
    let boxes: Vec<(f32, f32, f32, f32)> = ops.iter().map(|o| o.bounds()).collect();

    for py in 0..size {
        for px in 0..size {
            let (mut acc_c, mut acc_a) = (0.0f32, 0.0f32);
            for sy in 0..SS {
                let y = py as f32 * inv + (sy as f32 + 0.5) * sub;
                for sx in 0..SS {
                    let x = px as f32 * inv + (sx as f32 + 0.5) * sub;
                    // Painter's algorithm on one sample: opaque ops simply
                    // replace, which is all this icon needs.
                    let mut c = 0.0f32;
                    let mut a = 0.0f32;
                    for (op, b) in ops.iter().zip(&boxes) {
                        if x < b.0 || x > b.0 + b.2 || y < b.1 || y > b.1 + b.3 {
                            continue;
                        }
                        if !op.covers(x, y) {
                            continue;
                        }
                        let col = op.colour();
                        if col == CLEAR {
                            c = 0.0;
                            a = 0.0;
                        } else {
                            c = col;
                            a = 1.0;
                        }
                    }
                    acc_c += c * a;
                    acc_a += a;
                }
            }
            let n = (SS * SS) as f32;
            let a = acc_a / n;
            // Un-premultiply so the stored colour is the colour, not the colour
            // faded towards black at every edge.
            let c = if acc_a > 0.0 { acc_c / acc_a } else { 0.0 };
            let i = (py * size + px) * 4;
            let v = (c.clamp(0.0, 1.0) * 255.0).round() as u8;
            out[i] = v;
            out[i + 1] = v;
            out[i + 2] = v;
            out[i + 3] = (a.clamp(0.0, 1.0) * 255.0).round() as u8;
        }
    }
    out
}

/// Exact box filter. `from` must be a whole multiple of `to`.
fn downsample(src: &[u8], from: usize, to: usize) -> Vec<u8> {
    assert_eq!(from % to, 0, "{from} is not a whole multiple of {to}");
    let k = from / to;
    let n = (k * k) as f32;
    let mut out = vec![0u8; to * to * 4];
    for y in 0..to {
        for x in 0..to {
            let (mut sc, mut sa) = (0.0f32, 0.0f32);
            for j in 0..k {
                for i in 0..k {
                    let s = ((y * k + j) * from + (x * k + i)) * 4;
                    let a = src[s + 3] as f32 / 255.0;
                    // Average in premultiplied space, or a transparent pixel's
                    // arbitrary colour bleeds into its neighbours.
                    sc += (src[s] as f32 / 255.0) * a;
                    sa += a;
                }
            }
            let a = sa / n;
            let c = if sa > 0.0 { sc / sa } else { 0.0 };
            let d = (y * to + x) * 4;
            let v = (c * 255.0).round() as u8;
            out[d] = v;
            out[d + 1] = v;
            out[d + 2] = v;
            out[d + 3] = (a * 255.0).round() as u8;
        }
    }
    out
}

// ---------------------------------------------------------------- png

fn crc32(data: &[u8]) -> u32 {
    let mut table = [0u32; 256];
    for (i, e) in table.iter_mut().enumerate() {
        let mut c = i as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
        }
        *e = c;
    }
    let mut c = 0xFFFF_FFFFu32;
    for &b in data {
        c = table[((c ^ b as u32) & 0xFF) as usize] ^ (c >> 8);
    }
    c ^ 0xFFFF_FFFF
}

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for &x in data {
        a = (a + x as u32) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

/// LSB-first bit sink, which is deflate's bit order for everything except the
/// Huffman codes themselves.
struct Bits {
    out: Vec<u8>,
    acc: u32,
    n: u32,
}

impl Bits {
    fn new() -> Self {
        Bits { out: Vec::new(), acc: 0, n: 0 }
    }
    fn put(&mut self, value: u32, bits: u32) {
        self.acc |= value << self.n;
        self.n += bits;
        while self.n >= 8 {
            self.out.push((self.acc & 0xFF) as u8);
            self.acc >>= 8;
            self.n -= 8;
        }
    }
    /// Huffman codes are written most significant bit first.
    fn put_rev(&mut self, code: u32, bits: u32) {
        let mut r = 0u32;
        for i in 0..bits {
            r |= ((code >> i) & 1) << (bits - 1 - i);
        }
        self.put(r, bits);
    }
    fn finish(mut self) -> Vec<u8> {
        if self.n > 0 {
            self.out.push((self.acc & 0xFF) as u8);
        }
        self.out
    }
}

const LEN_BASE: [u16; 29] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131,
    163, 195, 227, 258,
];
const LEN_EXTRA: [u8; 29] = [
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
];
const DIST_BASE: [u16; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537,
    2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
];
const DIST_EXTRA: [u8; 30] = [
    0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13,
    13,
];

fn put_literal(b: &mut Bits, byte: u8) {
    let v = byte as u32;
    if v < 144 {
        b.put_rev(0x30 + v, 8);
    } else {
        b.put_rev(0x190 + (v - 144), 9);
    }
}

fn put_length_code(b: &mut Bits, code: u32) {
    // 256-279 are seven bits, 280-287 are eight.
    if code < 280 {
        b.put_rev(code - 256, 7);
    } else {
        b.put_rev(0xC0 + (code - 280), 8);
    }
}

/// Deflate with a fixed Huffman table and a single-candidate hash match. Not a
/// strong compressor, but flat monochrome art is nearly all long runs, and the
/// alternative is a stored stream that would put ten megabytes of icon into the
/// repository.
fn deflate(data: &[u8]) -> Vec<u8> {
    const WINDOW: usize = 32768;
    const HASH_BITS: usize = 15;
    let mut head = vec![usize::MAX; 1 << HASH_BITS];
    let mut prev = vec![usize::MAX; data.len().max(1)];
    let hash = |d: &[u8], i: usize| -> usize {
        ((d[i] as usize) << 10 ^ (d[i + 1] as usize) << 5 ^ d[i + 2] as usize) & ((1 << HASH_BITS) - 1)
    };

    let mut b = Bits::new();
    b.put(1, 1); // final block
    b.put(1, 2); // fixed Huffman

    let mut i = 0usize;
    while i < data.len() {
        let mut best_len = 0usize;
        let mut best_dist = 0usize;
        if i + 3 <= data.len() {
            let h = hash(data, i);
            let mut cand = head[h];
            // A handful of candidates is plenty here: matches in flat art are
            // found immediately and are very long.
            let mut tries = 16;
            while cand != usize::MAX && tries > 0 && i - cand <= WINDOW {
                let max = (data.len() - i).min(258);
                let mut l = 0usize;
                while l < max && data[cand + l] == data[i + l] {
                    l += 1;
                }
                if l > best_len {
                    best_len = l;
                    best_dist = i - cand;
                    if l >= 258 {
                        break;
                    }
                }
                cand = prev[cand];
                tries -= 1;
            }
            prev[i] = head[h];
            head[h] = i;
        }

        if best_len >= 3 {
            let li = LEN_BASE.iter().rposition(|&v| v as usize <= best_len).unwrap();
            put_length_code(&mut b, 257 + li as u32);
            let extra = LEN_EXTRA[li] as u32;
            if extra > 0 {
                b.put(best_len as u32 - LEN_BASE[li] as u32, extra);
            }
            let di = DIST_BASE.iter().rposition(|&v| v as usize <= best_dist).unwrap();
            b.put_rev(di as u32, 5);
            let extra = DIST_EXTRA[di] as u32;
            if extra > 0 {
                b.put(best_dist as u32 - DIST_BASE[di] as u32, extra);
            }
            // Register the positions the match skipped over, or later matches
            // lose every anchor inside it.
            for k in (i + 1)..(i + best_len).min(data.len().saturating_sub(2)) {
                let h = hash(data, k);
                prev[k] = head[h];
                head[h] = k;
            }
            i += best_len;
        } else {
            put_literal(&mut b, data[i]);
            i += 1;
        }
    }
    put_length_code(&mut b, 256); // end of block
    b.finish()
}

fn png(rgba: &[u8], size: usize) -> Vec<u8> {
    // One filter byte per row. Filter 0 (none) throughout: the art is flat, so
    // the matcher finds whole identical rows, which beats per-row prediction.
    let mut raw = Vec::with_capacity(size * (size * 4 + 1));
    for y in 0..size {
        raw.push(0u8);
        raw.extend_from_slice(&rgba[y * size * 4..(y + 1) * size * 4]);
    }
    let mut z = vec![0x78, 0x01]; // zlib header, no preset dictionary
    z.extend_from_slice(&deflate(&raw));
    z.extend_from_slice(&adler32(&raw).to_be_bytes());

    let chunk = |kind: &[u8], data: &[u8]| -> Vec<u8> {
        let mut c = Vec::with_capacity(data.len() + 12);
        c.extend_from_slice(&(data.len() as u32).to_be_bytes());
        c.extend_from_slice(kind);
        c.extend_from_slice(data);
        let mut crc_in = kind.to_vec();
        crc_in.extend_from_slice(data);
        c.extend_from_slice(&crc32(&crc_in).to_be_bytes());
        c
    };

    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&(size as u32).to_be_bytes());
    ihdr.extend_from_slice(&(size as u32).to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]); // 8-bit, RGBA, deflate, no interlace

    let mut out = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    out.extend_from_slice(&chunk(b"IHDR", &ihdr));
    out.extend_from_slice(&chunk(b"IDAT", &z));
    out.extend_from_slice(&chunk(b"IEND", &[]));
    out
}

// ---------------------------------------------------------------- main

/// The names iconutil expects. Both entries of a pair hold the same pixels;
/// the difference is only which scale factor macOS believes it is looking at.
const ICONSET: [(&str, usize); 10] = [
    ("icon_16x16.png", 16),
    ("icon_16x16@2x.png", 32),
    ("icon_32x32.png", 32),
    ("icon_32x32@2x.png", 64),
    ("icon_128x128.png", 128),
    ("icon_128x128@2x.png", 256),
    ("icon_256x256.png", 256),
    ("icon_256x256@2x.png", 512),
    ("icon_512x512.png", 512),
    ("icon_512x512@2x.png", 1024),
];

fn main() {
    let mut args = std::env::args().skip(1);
    let dir = args.next().unwrap_or_else(|| {
        eprintln!("usage: icon <output.iconset directory> [variant]");
        eprintln!("variants: {}", VARIANTS.iter().map(|v| v.0).collect::<Vec<_>>().join(" "));
        std::process::exit(2);
    });
    // A named variant renders one of the alternates instead of the shipped
    // mark. Only used while choosing a design; the build never passes one.
    let ops: &[Op] = match args.next() {
        None => ICON,
        Some(name) => match VARIANTS.iter().find(|v| v.0 == name) {
            Some(v) => v.1,
            None => {
                eprintln!("no variant {name:?}");
                eprintln!("variants: {}", VARIANTS.iter().map(|v| v.0).collect::<Vec<_>>().join(" "));
                std::process::exit(2);
            }
        },
    };
    std::fs::create_dir_all(&dir).expect("create output directory");

    let master = render(ops, MASTER);
    let mut cache: Vec<(usize, Vec<u8>)> = vec![(MASTER, master)];

    for (name, size) in ICONSET {
        if !cache.iter().any(|(s, _)| *s == size) {
            let src = cache
                .iter()
                .find(|(s, _)| *s == MASTER)
                .map(|(_, b)| b.clone())
                .unwrap();
            cache.push((size, downsample(&src, MASTER, size)));
        }
        let buf = &cache.iter().find(|(s, _)| *s == size).unwrap().1;
        let path = format!("{dir}/{name}");
        let bytes = png(buf, size);
        let mut f = std::fs::File::create(&path).expect("create png");
        f.write_all(&bytes).expect("write png");
        println!("{path}  {size}x{size}  {} bytes", bytes.len());
    }
}
