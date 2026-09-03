//! `--screenshot <path>` renders a few frames, saves the window, and exits.
//!
//! Capturing from inside the process avoids the Screen Recording permission
//! that `screencapture` and every external tool needs, so this works on a
//! machine with no grants at all. The PNG is written with stored (uncompressed)
//! deflate blocks so the program needs no image or compression dependency.

pub struct Job {
    pub path: String,
    /// Frames to render before asking for the capture, so fonts and layout have
    /// settled.
    pub warmup: u32,
    pub requested: bool,
}

impl Job {
    pub fn new(path: String) -> Self {
        Job {
            path,
            warmup: 6,
            requested: false,
        }
    }

    /// Drives the capture. Returns true once the file has been written.
    pub fn step(&mut self, ctx: &egui::Context) -> bool {
        if self.warmup > 0 {
            self.warmup -= 1;
            ctx.request_repaint();
            return false;
        }
        if !self.requested {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
            self.requested = true;
            ctx.request_repaint();
            return false;
        }
        let shot = ctx.input(|i| {
            i.raw.events.iter().find_map(|e| match e {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        match shot {
            Some(image) => {
                let mut rgba = Vec::with_capacity(image.pixels.len() * 4);
                for p in &image.pixels {
                    rgba.extend_from_slice(&p.to_array());
                }
                match write_png(
                    &self.path,
                    image.width() as u32,
                    image.height() as u32,
                    &rgba,
                ) {
                    Ok(()) => eprintln!(
                        "wrote {} ({}x{})",
                        self.path,
                        image.width(),
                        image.height()
                    ),
                    Err(e) => eprintln!("screenshot failed: {e}"),
                }
                true
            }
            None => {
                ctx.request_repaint();
                false
            }
        }
    }
}

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
    for &byte in data {
        a = (a + byte as u32) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], body: &[u8]) {
    out.extend_from_slice(&(body.len() as u32).to_be_bytes());
    let mut with_kind = Vec::with_capacity(4 + body.len());
    with_kind.extend_from_slice(kind);
    with_kind.extend_from_slice(body);
    out.extend_from_slice(&with_kind);
    out.extend_from_slice(&crc32(&with_kind).to_be_bytes());
}

pub fn write_png(path: &str, w: u32, h: u32, rgba: &[u8]) -> std::io::Result<()> {
    let mut raw = Vec::with_capacity((w as usize * 4 + 1) * h as usize);
    for y in 0..h as usize {
        raw.push(0u8); // filter: none
        let start = y * w as usize * 4;
        raw.extend_from_slice(&rgba[start..start + w as usize * 4]);
    }

    // zlib wrapper around stored deflate blocks: no compression, no dependency.
    let mut z = vec![0x78, 0x01];
    for (i, block) in raw.chunks(65535).enumerate() {
        let last = (i + 1) * 65535 >= raw.len();
        z.push(if last { 1 } else { 0 });
        z.extend_from_slice(&(block.len() as u16).to_le_bytes());
        z.extend_from_slice(&(!(block.len() as u16)).to_le_bytes());
        z.extend_from_slice(block);
    }
    z.extend_from_slice(&adler32(&raw).to_be_bytes());

    let mut png = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&w.to_be_bytes());
    ihdr.extend_from_slice(&h.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]); // 8-bit RGBA
    chunk(&mut png, b"IHDR", &ihdr);
    chunk(&mut png, b"IDAT", &z);
    chunk(&mut png, b"IEND", &[]);
    std::fs::write(path, png)
}
