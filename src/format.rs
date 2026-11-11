//! Value and time formatting helpers.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Radix {
    Hex,
    Dec,
    SDec,
    Bin,
    Oct,
    Ascii,
}

impl Radix {
    pub const ALL: [Radix; 6] = [
        Radix::Hex,
        Radix::Dec,
        Radix::SDec,
        Radix::Bin,
        Radix::Oct,
        Radix::Ascii,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Radix::Hex => "hex",
            Radix::Dec => "dec",
            Radix::SDec => "sdec",
            Radix::Bin => "bin",
            Radix::Oct => "oct",
            Radix::Ascii => "ascii",
        }
    }

    pub fn short(self) -> &'static str {
        match self {
            Radix::Hex => "h",
            Radix::Dec => "d",
            Radix::SDec => "s",
            Radix::Bin => "b",
            Radix::Oct => "o",
            Radix::Ascii => "a",
        }
    }

    pub fn parse(s: &str) -> Option<Radix> {
        Some(match s.to_ascii_lowercase().as_str() {
            "hex" | "h" | "x" => Radix::Hex,
            "dec" | "d" | "u" | "unsigned" => Radix::Dec,
            "sdec" | "s" | "signed" | "i" => Radix::SDec,
            "bin" | "b" => Radix::Bin,
            "oct" | "o" => Radix::Oct,
            "ascii" | "a" | "str" => Radix::Ascii,
            _ => return None,
        })
    }

    pub fn next(self) -> Radix {
        let i = Radix::ALL.iter().position(|&r| r == self).unwrap();
        Radix::ALL[(i + 1) % Radix::ALL.len()]
    }
}

fn group(bits: &[u8], n: usize) -> String {
    const DIG: &[u8; 16] = b"0123456789ABCDEF";
    let len = bits.len();
    let first = if len.is_multiple_of(n) { n } else { len % n };
    let mut out = String::with_capacity(len / n + 1);
    let mut i = 0;
    let mut chunk = first;
    while i < len {
        let c = &bits[i..i + chunk];
        if c.contains(&b'x') {
            out.push('x');
        } else if c.contains(&b'z') {
            out.push('z');
        } else {
            let v = c
                .iter()
                .fold(0usize, |a, &b| (a << 1) | (b - b'0') as usize);
            out.push(DIG[v] as char);
        }
        i += chunk;
        chunk = n;
    }
    out
}

pub fn format_bits(bits: &[u8], radix: Radix) -> String {
    if bits.len() == 1 {
        return (bits[0] as char).to_string();
    }
    match radix {
        Radix::Bin => String::from_utf8_lossy(bits).into_owned(),
        Radix::Hex => group(bits, 4),
        Radix::Oct => group(bits, 3),
        Radix::Dec | Radix::SDec => {
            if bits.iter().all(|&c| c == b'z') {
                return "z".into();
            }
            if bits.iter().any(|&c| c != b'0' && c != b'1') {
                return "x".into();
            }
            if bits.len() > 128 {
                return group(bits, 4);
            }
            let v = bits
                .iter()
                .fold(0u128, |a, &b| (a << 1) | (b - b'0') as u128);
            if radix == Radix::SDec && bits[0] == b'1' {
                let sv = if bits.len() == 128 {
                    v as i128
                } else {
                    v as i128 - (1i128 << bits.len())
                };
                sv.to_string()
            } else {
                v.to_string()
            }
        }
        Radix::Ascii => {
            let len = bits.len();
            let first = if len.is_multiple_of(8) { 8 } else { len % 8 };
            let mut out = String::new();
            let mut i = 0;
            let mut chunk = first;
            while i < len {
                let c = &bits[i..i + chunk];
                if c.iter().any(|&b| b != b'0' && b != b'1') {
                    out.push('?');
                } else {
                    let v = c.iter().fold(0u8, |a, &b| (a << 1) | (b - b'0'));
                    if v == 0 {
                        // skip NUL padding
                    } else if (0x20..0x7f).contains(&v) {
                        out.push(v as char);
                    } else {
                        out.push('.');
                    }
                }
                i += chunk;
                chunk = 8;
            }
            out
        }
    }
}

pub fn format_real(v: f64) -> String {
    if v.is_nan() {
        "x".into()
    } else if v != 0.0 && (v.abs() >= 1e7 || v.abs() < 1e-4) {
        format!("{v:.4e}")
    } else {
        let s = format!("{v:.6}");
        let s = s.trim_end_matches('0').trim_end_matches('.');
        s.to_string()
    }
}

const UNITS: [(f64, &str); 6] = [
    (1e15, "s"),
    (1e12, "ms"),
    (1e9, "us"),
    (1e6, "ns"),
    (1e3, "ps"),
    (1.0, "fs"),
];

/// Format a duration in femtoseconds with an automatically chosen unit.
pub fn fmt_fs(fs: f64) -> String {
    if fs == 0.0 {
        return "0".into();
    }
    let a = fs.abs();
    let (u, name) = UNITS
        .iter()
        .copied()
        .find(|&(u, _)| a >= u * 0.9999999)
        .unwrap_or((1.0, "fs"));
    let v = fs / u;
    let s = format!("{v:.3}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    format!("{s}{name}")
}

pub fn fmt_time(t: u64, ts_fs: f64) -> String {
    fmt_fs(t as f64 * ts_fs)
}

pub fn fmt_delta(d: i128, ts_fs: f64) -> String {
    let s = fmt_fs(d as f64 * ts_fs);
    if d > 0 { format!("+{s}") } else { s }
}

/// Parse a time like `120`, `1.5us`, `30 ns` into time units.
pub fn parse_time(s: &str, ts_fs: f64) -> Option<u64> {
    let s = s.trim();
    let split = s
        .find(|c: char| !(c.is_ascii_digit() || c == '.'))
        .unwrap_or(s.len());
    let (num, unit) = s.split_at(split);
    let v: f64 = num.parse().ok()?;
    let unit = unit.trim();
    if unit.is_empty() {
        return Some(v.round() as u64);
    }
    let u = crate::vcd::unit_fs(unit)?;
    Some((v * u / ts_fs).round().max(0.0) as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bits() {
        assert_eq!(format_bits(b"00011111", Radix::Hex), "1F");
        assert_eq!(format_bits(b"1x0011111", Radix::Hex), "1xF");
        assert_eq!(format_bits(b"zzzz", Radix::Hex), "z");
        assert_eq!(format_bits(b"11111111", Radix::SDec), "-1");
        assert_eq!(format_bits(b"11111111", Radix::Dec), "255");
        assert_eq!(format_bits(b"0100000101000010", Radix::Ascii), "AB");
        assert_eq!(format_bits(b"111", Radix::Oct), "7");
    }

    #[test]
    fn times() {
        assert_eq!(fmt_fs(1500.0 * 1e6), "1.5us");
        assert_eq!(fmt_fs(10.0 * 1e6), "10ns");
        assert_eq!(parse_time("1.5us", 1e6), Some(1500));
        assert_eq!(parse_time("42", 1e6), Some(42));
        assert_eq!(parse_time("3 ns", 1e3), Some(3000));
    }
}
