//! HID report descriptor parsing.
//!
//! Why this exists rather than leaning on the OS: on macOS, IOKit will decode
//! elements for us through a value callback, but that queue is change-driven
//! and in testing it produced zero callbacks for devices that were delivering
//! reports normally. Depending on it would mean a mouse that reports fine but
//! decodes to nothing. Parsing the descriptor gives a decode path that is
//! independent of that callback and can be tested against real descriptors
//! without the hardware present.
//!
//! Only what a pointing device needs is modelled: variable input fields with a
//! usage, their bit offset, width and signedness. Array items, which mice do
//! not use for axes or buttons, are skipped rather than half-supported.

/// One decodable field inside an input report.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Field {
    pub report_id: u8,
    pub usage_page: u16,
    pub usage: u16,
    /// Bit offset within the report, not counting the leading report-ID byte.
    pub bit_offset: u32,
    pub bit_size: u32,
    pub signed: bool,
    pub relative: bool,
}

#[derive(Clone, Debug, Default)]
pub struct ReportMap {
    pub fields: Vec<Field>,
    /// True when the descriptor uses report IDs, so every report is prefixed
    /// with one.
    pub uses_report_ids: bool,
    /// Total input report size in bits, per report id.
    pub report_bits: Vec<(u8, u32)>,
}

#[derive(Clone, Copy, Default)]
struct GlobalState {
    usage_page: u16,
    logical_min: i64,
    logical_max: i64,
    report_size: u32,
    report_count: u32,
    report_id: u8,
}

pub fn parse(desc: &[u8]) -> ReportMap {
    let mut map = ReportMap::default();
    let mut g = GlobalState::default();
    let mut stack: Vec<GlobalState> = Vec::new();
    let mut usages: Vec<u32> = Vec::new();
    let mut usage_min: Option<u32> = None;
    let mut usage_max: Option<u32> = None;
    // Input reports are laid out per report id, so each id has its own cursor.
    let mut bit_pos: Vec<(u8, u32)> = Vec::new();

    let mut i = 0usize;
    while i < desc.len() {
        let prefix = desc[i];
        i += 1;

        if prefix == 0b1111_1110 {
            // Long item: one size byte, one tag byte, then the payload. No HID
            // device in this domain uses them; skip it correctly rather than
            // desynchronising the parse.
            if i + 1 >= desc.len() {
                break;
            }
            let size = desc[i] as usize;
            i += 2 + size;
            continue;
        }

        let b_size = match prefix & 0b11 {
            0 => 0usize,
            1 => 1,
            2 => 2,
            3 => 4,
            _ => unreachable!(),
        };
        if i + b_size > desc.len() {
            break;
        }
        let raw = &desc[i..i + b_size];
        i += b_size;

        let uval: u32 = match b_size {
            0 => 0,
            1 => raw[0] as u32,
            2 => u16::from_le_bytes([raw[0], raw[1]]) as u32,
            _ => u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]),
        };
        // Logical minimum and maximum are signed; everything else is not.
        let ival: i64 = match b_size {
            0 => 0,
            1 => raw[0] as i8 as i64,
            2 => i16::from_le_bytes([raw[0], raw[1]]) as i64,
            _ => i32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]) as i64,
        };

        let tag = (prefix >> 4) & 0xF;
        let typ = (prefix >> 2) & 0b11;

        match typ {
            // Main
            0 => {
                match tag {
                    // Input
                    0x8 => {
                        let constant = uval & 0b1 != 0;
                        let variable = uval & 0b10 != 0;
                        let relative = uval & 0b100 != 0;

                        let cursor = match bit_pos.iter_mut().find(|(id, _)| *id == g.report_id) {
                            Some((_, p)) => p,
                            None => {
                                bit_pos.push((g.report_id, 0));
                                &mut bit_pos.last_mut().unwrap().1
                            }
                        };

                        if variable && !constant {
                            for n in 0..g.report_count {
                                let usage_full = if let Some(u) = usages.get(n as usize) {
                                    *u
                                } else if let (Some(lo), Some(hi)) = (usage_min, usage_max) {
                                    let u = lo + n;
                                    if u > hi {
                                        hi
                                    } else {
                                        u
                                    }
                                } else if let Some(&last) = usages.last() {
                                    // A single usage repeated across the count.
                                    last
                                } else {
                                    0
                                };
                                // A usage may carry its page in the high half.
                                let (page, usage) = if usage_full > 0xFFFF {
                                    ((usage_full >> 16) as u16, (usage_full & 0xFFFF) as u16)
                                } else {
                                    (g.usage_page, usage_full as u16)
                                };
                                map.fields.push(Field {
                                    report_id: g.report_id,
                                    usage_page: page,
                                    usage,
                                    bit_offset: *cursor + n * g.report_size,
                                    bit_size: g.report_size,
                                    // Signed only if the descriptor says values
                                    // can go below zero.
                                    signed: g.logical_min < 0,
                                    relative,
                                });
                            }
                        }
                        // Constant and array items still occupy their bits.
                        *cursor += g.report_size.saturating_mul(g.report_count);
                    }
                    // Output, Feature, Collection, EndCollection: no input bits.
                    _ => {}
                }
                usages.clear();
                usage_min = None;
                usage_max = None;
            }
            // Global
            1 => match tag {
                0x0 => g.usage_page = uval as u16,
                0x1 => g.logical_min = ival,
                0x2 => g.logical_max = ival,
                0x7 => g.report_size = uval,
                0x8 => {
                    g.report_id = uval as u8;
                    map.uses_report_ids = true;
                }
                0x9 => g.report_count = uval,
                0xA => stack.push(g),
                0xB => {
                    if let Some(prev) = stack.pop() {
                        g = prev;
                    }
                }
                _ => {}
            },
            // Local
            2 => match tag {
                0x0 => usages.push(uval),
                0x1 => usage_min = Some(uval),
                0x2 => usage_max = Some(uval),
                _ => {}
            },
            _ => {}
        }
    }

    map.report_bits = bit_pos;
    map
}

/// Reads `bit_size` bits at `bit_offset` from `data`, little-endian bit order
/// within each byte, sign-extending when asked.
pub fn extract(data: &[u8], bit_offset: u32, bit_size: u32, signed: bool) -> i64 {
    if bit_size == 0 || bit_size > 32 {
        return 0;
    }
    let mut value: u64 = 0;
    for b in 0..bit_size {
        let abs = bit_offset + b;
        let byte = (abs / 8) as usize;
        if byte >= data.len() {
            return 0;
        }
        let bit = (data[byte] >> (abs % 8)) & 1;
        value |= (bit as u64) << b;
    }
    if signed && bit_size < 64 {
        let sign_bit = 1u64 << (bit_size - 1);
        if value & sign_bit != 0 {
            return (value as i64) - (1i64 << bit_size);
        }
    }
    value as i64
}

/// What a mouse report decodes to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Decoded {
    pub dx: i32,
    pub dy: i32,
    pub wheel: i32,
    pub hwheel: i32,
    /// Bit n set means button n+1 is currently down.
    pub buttons: u32,
    pub matched_fields: u32,
}

impl ReportMap {
    /// Fields belonging to one report id, ready for repeated decoding.
    pub fn fields_for(&self, report_id: u8) -> Vec<Field> {
        self.fields
            .iter()
            .copied()
            .filter(|f| f.report_id == report_id)
            .collect()
    }

    /// True if this report id carries anything a pointing device would send.
    #[allow(dead_code)]
    pub fn is_pointer_report(&self, report_id: u8) -> bool {
        self.fields.iter().any(|f| {
            f.report_id == report_id
                && ((f.usage_page == 0x01 && (f.usage == 0x30 || f.usage == 0x31))
                    || f.usage_page == 0x09)
        })
    }
}

/// Decodes one report body. `body` must exclude the report-ID byte.
pub fn decode(fields: &[Field], body: &[u8]) -> Decoded {
    let mut d = Decoded::default();
    for f in fields {
        let v = extract(body, f.bit_offset, f.bit_size, f.signed);
        match (f.usage_page, f.usage) {
            (0x01, 0x30) => {
                d.dx = v as i32;
                d.matched_fields += 1;
            }
            (0x01, 0x31) => {
                d.dy = v as i32;
                d.matched_fields += 1;
            }
            (0x01, 0x38) => {
                d.wheel = v as i32;
                d.matched_fields += 1;
            }
            // Consumer page AC Pan is the usual horizontal wheel.
            (0x0C, 0x0238) => {
                d.hwheel = v as i32;
                d.matched_fields += 1;
            }
            (0x09, b) if b >= 1 && b <= 32 => {
                if v != 0 {
                    d.buttons |= 1 << (b - 1);
                }
                d.matched_fields += 1;
            }
            _ => {}
        }
    }
    d
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The real report descriptor read from the built-in trackpad on this
    /// machine, byte for byte. Testing against a descriptor that actually came
    /// off hardware is worth more than any number of invented ones.
    const TRACKPAD: &[u8] = &[
        0x05, 0x01, 0x09, 0x02, 0xa1, 0x01, 0x09, 0x01, 0xa1, 0x00, 0x05, 0x09, 0x19, 0x01,
        0x29, 0x03, 0x15, 0x00, 0x25, 0x01, 0x85, 0x02, 0x95, 0x03, 0x75, 0x01, 0x81, 0x02,
        0x95, 0x01, 0x75, 0x05, 0x81, 0x01, 0x05, 0x01, 0x09, 0x30, 0x09, 0x31, 0x15, 0x81,
        0x25, 0x7f, 0x75, 0x08, 0x95, 0x02, 0x81, 0x06, 0x95, 0x04, 0x75, 0x08, 0x81, 0x01,
        0x76, 0x00, 0x40, 0x95, 0x02, 0xb1, 0x01, 0xc0, 0xc0, 0x05, 0x0d, 0x09, 0x05, 0xa1,
        0x01, 0x06, 0x00, 0xff, 0x09, 0x0c, 0x15, 0x00, 0x26, 0xff, 0x00, 0x75, 0x08, 0x95,
        0x10, 0x85, 0x3f, 0x81, 0x22, 0xc0, 0x06, 0x00, 0xff, 0x09, 0x0c, 0xa1, 0x01, 0x06,
        0x00, 0xff, 0x09, 0x0c, 0x15, 0x00, 0x26, 0xff, 0x00, 0x85, 0x44, 0x75, 0x08, 0x96,
        0xd7, 0x06, 0x81, 0x00, 0xc0,
    ];

    /// A classic three-button-plus-wheel mouse with no report id.
    const CLASSIC: &[u8] = &[
        0x05, 0x01, 0x09, 0x02, 0xa1, 0x01, 0x09, 0x01, 0xa1, 0x00, 0x05, 0x09, 0x19, 0x01,
        0x29, 0x03, 0x15, 0x00, 0x25, 0x01, 0x95, 0x03, 0x75, 0x01, 0x81, 0x02, 0x95, 0x01,
        0x75, 0x05, 0x81, 0x03, 0x05, 0x01, 0x09, 0x30, 0x09, 0x31, 0x09, 0x38, 0x15, 0x81,
        0x25, 0x7f, 0x75, 0x08, 0x95, 0x03, 0x81, 0x06, 0xc0, 0xc0,
    ];

    /// The shape a gaming mouse uses: five buttons, 16-bit deltas, a wheel and
    /// a horizontal wheel on the consumer page.
    const HIGH_RES: &[u8] = &[
        0x05, 0x01, 0x09, 0x02, 0xa1, 0x01, 0x85, 0x01, 0x09, 0x01, 0xa1, 0x00, 0x05, 0x09,
        0x19, 0x01, 0x29, 0x05, 0x15, 0x00, 0x25, 0x01, 0x95, 0x05, 0x75, 0x01, 0x81, 0x02,
        0x95, 0x01, 0x75, 0x03, 0x81, 0x01, 0x05, 0x01, 0x09, 0x30, 0x09, 0x31, 0x16, 0x00,
        0x80, 0x26, 0xff, 0x7f, 0x75, 0x10, 0x95, 0x02, 0x81, 0x06, 0x09, 0x38, 0x15, 0x81,
        0x25, 0x7f, 0x75, 0x08, 0x95, 0x01, 0x81, 0x06, 0x05, 0x0c, 0x0a, 0x38, 0x02, 0x15,
        0x81, 0x25, 0x7f, 0x75, 0x08, 0x95, 0x01, 0x81, 0x06, 0xc0, 0xc0,
    ];

    fn field(map: &ReportMap, page: u16, usage: u16) -> Option<Field> {
        map.fields
            .iter()
            .copied()
            .find(|f| f.usage_page == page && f.usage == usage)
    }

    #[test]
    fn parses_the_real_trackpad_descriptor() {
        let map = parse(TRACKPAD);
        assert!(map.uses_report_ids);

        let x = field(&map, 0x01, 0x30).expect("no X field");
        let y = field(&map, 0x01, 0x31).expect("no Y field");
        assert_eq!(x.report_id, 2);
        assert_eq!((x.bit_offset, x.bit_size, x.signed, x.relative), (8, 8, true, true));
        assert_eq!((y.bit_offset, y.bit_size, y.signed, y.relative), (16, 8, true, true));

        // Three buttons in the low three bits, then five bits of padding.
        for (n, expected_bit) in [(1u16, 0u32), (2, 1), (3, 2)] {
            let b = field(&map, 0x09, n).unwrap_or_else(|| panic!("no button {n}"));
            assert_eq!(b.bit_offset, expected_bit);
            assert_eq!(b.bit_size, 1);
            assert!(!b.relative);
        }
        // This descriptor genuinely has no wheel, which is why the device
        // reports no scroll.
        assert!(field(&map, 0x01, 0x38).is_none());
        assert!(map.is_pointer_report(2));
    }

    #[test]
    fn parses_a_classic_mouse_without_report_ids() {
        let map = parse(CLASSIC);
        assert!(!map.uses_report_ids);
        let x = field(&map, 0x01, 0x30).unwrap();
        let y = field(&map, 0x01, 0x31).unwrap();
        let w = field(&map, 0x01, 0x38).unwrap();
        assert_eq!(x.bit_offset, 8);
        assert_eq!(y.bit_offset, 16);
        assert_eq!(w.bit_offset, 24);
        for f in [x, y, w] {
            assert_eq!(f.bit_size, 8);
            assert!(f.signed && f.relative);
        }
    }

    #[test]
    fn parses_sixteen_bit_deltas_and_a_horizontal_wheel() {
        let map = parse(HIGH_RES);
        let x = field(&map, 0x01, 0x30).unwrap();
        let y = field(&map, 0x01, 0x31).unwrap();
        assert_eq!((x.bit_offset, x.bit_size), (8, 16));
        assert_eq!((y.bit_offset, y.bit_size), (24, 16));
        assert!(x.signed && x.relative);
        let w = field(&map, 0x01, 0x38).unwrap();
        assert_eq!((w.bit_offset, w.bit_size), (40, 8));
        let h = field(&map, 0x0C, 0x0238).expect("no AC Pan field");
        assert_eq!((h.bit_offset, h.bit_size), (48, 8));
        // Five buttons, not three.
        assert!(field(&map, 0x09, 5).is_some());
        assert!(field(&map, 0x09, 6).is_none());
    }

    #[test]
    fn bit_extraction_handles_sign_and_straddling_bytes() {
        // 0xFF as a signed 8-bit value is -1, as unsigned it is 255.
        assert_eq!(extract(&[0xFF], 0, 8, true), -1);
        assert_eq!(extract(&[0xFF], 0, 8, false), 255);
        assert_eq!(extract(&[0x80], 0, 8, true), -128);
        // 12 bits spanning a byte boundary.
        assert_eq!(extract(&[0x34, 0x12], 0, 12, false), 0x234);
        assert_eq!(extract(&[0b0000_0110], 1, 2, false), 0b11);
        // 16-bit little endian.
        assert_eq!(extract(&[0x00, 0x80], 0, 16, true), -32768);
        assert_eq!(extract(&[0xFF, 0x7F], 0, 16, true), 32767);
        // Reading past the end yields zero rather than panicking.
        assert_eq!(extract(&[0x01], 0, 32, false), 0);
    }

    #[test]
    fn decodes_a_trackpad_report_body() {
        let map = parse(TRACKPAD);
        let fields = map.fields_for(2);
        // Buttons byte: left down. X = -3, Y = +7.
        let body = [0b0000_0001u8, (-3i8) as u8, 7u8, 0, 0, 0, 0];
        let d = decode(&fields, &body);
        assert_eq!(d.dx, -3);
        assert_eq!(d.dy, 7);
        assert_eq!(d.buttons, 0b1);
        assert_eq!(d.wheel, 0);
    }

    #[test]
    fn decodes_a_high_resolution_report_body() {
        let map = parse(HIGH_RES);
        let fields = map.fields_for(1);
        // Buttons 1 and 5 down, dx = -1000, dy = 2000, wheel -1, pan +1.
        let mut body = vec![0u8; 7];
        body[0] = 0b0001_0001;
        body[1..3].copy_from_slice(&(-1000i16).to_le_bytes());
        body[3..5].copy_from_slice(&2000i16.to_le_bytes());
        body[5] = (-1i8) as u8;
        body[6] = 1;
        let d = decode(&fields, &body);
        assert_eq!(d.dx, -1000);
        assert_eq!(d.dy, 2000);
        assert_eq!(d.wheel, -1);
        assert_eq!(d.hwheel, 1);
        assert_eq!(d.buttons, 0b1_0001);
    }

    #[test]
    fn a_truncated_or_junk_descriptor_does_not_panic() {
        for cut in 0..TRACKPAD.len() {
            let _ = parse(&TRACKPAD[..cut]);
        }
        let _ = parse(&[0xFE, 0x02, 0x00, 0xAA, 0xBB]);
        let _ = parse(&[0xFF; 64]);
    }
}
