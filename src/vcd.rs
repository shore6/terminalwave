//! Minimal, tolerant VCD (Value Change Dump, IEEE 1364) parser.

use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SigKind {
    Bits,
    Real,
    Str,
}

/// Value history of one VCD identifier code (shared by aliased vars).
#[derive(Debug)]
pub struct Signal {
    pub kind: SigKind,
    pub width: usize,
    pub times: Vec<u64>,
    bits: Vec<u8>,
    reals: Vec<f64>,
    strs: Vec<String>,
}

impl Signal {
    fn new(kind: SigKind, width: usize) -> Self {
        Signal {
            kind,
            width: width.max(1),
            times: Vec::new(),
            bits: Vec::new(),
            reals: Vec::new(),
            strs: Vec::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.times.len()
    }

    /// Bit characters (`0`,`1`,`x`,`z`) of change `i`, MSB first.
    pub fn bits(&self, i: usize) -> &[u8] {
        let w = self.width;
        &self.bits[i * w..(i + 1) * w]
    }

    pub fn real(&self, i: usize) -> f64 {
        self.reals[i]
    }

    pub fn string(&self, i: usize) -> &str {
        &self.strs[i]
    }

    /// Single bit character of change `i`. `bit` is the character position
    /// (0 = MSB). `None` means the whole (1-bit) signal.
    pub fn bit(&self, i: usize, bit: Option<usize>) -> u8 {
        match self.kind {
            SigKind::Bits => {
                let b = self.bits(i);
                b[bit.unwrap_or(0).min(b.len() - 1)]
            }
            SigKind::Real => {
                if self.reals[i].is_nan() {
                    b'x'
                } else {
                    b'1'
                }
            }
            SigKind::Str => b'1',
        }
    }

    /// Index of the last change at or before `t`.
    pub fn index_at(&self, t: u64) -> Option<usize> {
        self.times.partition_point(|&x| x <= t).checked_sub(1)
    }

    fn push_bits(&mut self, t: u64, raw: &[u8]) {
        if self.kind != SigKind::Bits {
            if self.kind == SigKind::Real {
                // e.g. "x" dumped for a real: treat as unknown.
                self.push_real(t, f64::NAN);
            }
            return;
        }
        let w = self.width;
        let start = self.bits.len();
        if raw.len() >= w {
            self.bits
                .extend(raw[raw.len() - w..].iter().map(|&c| norm(c)));
        } else {
            let pad = match raw.first().map(|&c| norm(c)) {
                Some(b'1') | Some(b'0') => b'0',
                Some(c) => c,
                None => b'x',
            };
            self.bits.extend(std::iter::repeat_n(pad, w - raw.len()));
            self.bits.extend(raw.iter().map(|&c| norm(c)));
        }
        let n = self.times.len();
        if n > 0 {
            if self.bits[start - w..start] == self.bits[start..] {
                self.bits.truncate(start);
                return;
            }
            if self.times[n - 1] == t {
                // Same timestamp: the last value wins.
                self.bits.copy_within(start.., start - w);
                self.bits.truncate(start);
                if n >= 2 && self.bits[start - 2 * w..start - w] == self.bits[start - w..start] {
                    self.times.pop();
                    self.bits.truncate(start - w);
                }
                return;
            }
        }
        self.times.push(t);
    }

    fn push_real(&mut self, t: u64, v: f64) {
        if self.kind != SigKind::Real {
            return;
        }
        let same = |a: f64, b: f64| a == b || (a.is_nan() && b.is_nan());
        if let Some(&last) = self.reals.last() {
            if same(last, v) {
                return;
            }
            if *self.times.last().unwrap() == t {
                *self.reals.last_mut().unwrap() = v;
                let n = self.reals.len();
                if n >= 2 && same(self.reals[n - 2], v) {
                    self.reals.pop();
                    self.times.pop();
                }
                return;
            }
        }
        self.times.push(t);
        self.reals.push(v);
    }

    fn push_str(&mut self, t: u64, v: &str) {
        if self.kind != SigKind::Str {
            return;
        }
        if let Some(last) = self.strs.last() {
            if last == v {
                return;
            }
            if *self.times.last().unwrap() == t {
                *self.strs.last_mut().unwrap() = v.to_string();
                return;
            }
        }
        self.times.push(t);
        self.strs.push(v.to_string());
    }
}

fn norm(c: u8) -> u8 {
    match c {
        b'0' | b'1' => c,
        b'z' | b'Z' => b'z',
        b'h' | b'H' => b'1',
        b'l' | b'L' => b'0',
        _ => b'x',
    }
}

#[derive(Debug)]
#[allow(dead_code)]
pub struct Var {
    /// Base name as written in the file.
    pub name: String,
    /// Name with range, e.g. `data[7:0]`.
    pub display: String,
    /// Full hierarchical path of the base name, e.g. `top.dut.data`.
    pub path: String,
    pub vtype: String,
    pub width: usize,
    pub msb: i64,
    pub lsb: i64,
    pub signal: usize,
    pub scope: usize,
}

impl Var {
    /// Bit index label for character position `k` (0 = MSB).
    pub fn bit_label(&self, k: usize) -> i64 {
        if self.msb >= self.lsb {
            self.msb - k as i64
        } else {
            self.msb + k as i64
        }
    }

    pub fn full_display(&self, scopes: &[Scope]) -> String {
        let sp = &scopes[self.scope].path;
        if sp.is_empty() {
            self.display.clone()
        } else {
            format!("{}.{}", sp, self.display)
        }
    }
}

#[derive(Debug)]
#[allow(dead_code)]
pub struct Scope {
    pub name: String,
    pub kind: String,
    pub path: String,
    pub parent: Option<usize>,
    pub children: Vec<usize>,
    pub vars: Vec<usize>,
}

#[derive(Debug)]
pub struct Vcd {
    /// Femtoseconds per time unit.
    pub timescale_fs: f64,
    /// Scope 0 is an unnamed root.
    pub scopes: Vec<Scope>,
    pub vars: Vec<Var>,
    pub signals: Vec<Signal>,
    pub start_time: u64,
    pub end_time: u64,
}

impl Vcd {
    pub fn signal_of(&self, var: usize) -> &Signal {
        &self.signals[self.vars[var].signal]
    }
}

struct Tokens<'a> {
    s: &'a [u8],
    pos: usize,
}

impl<'a> Tokens<'a> {
    fn next(&mut self) -> Option<&'a [u8]> {
        let s = self.s;
        let mut i = self.pos;
        while i < s.len() && s[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= s.len() {
            self.pos = i;
            return None;
        }
        let st = i;
        while i < s.len() && !s[i].is_ascii_whitespace() {
            i += 1;
        }
        self.pos = i;
        Some(&s[st..i])
    }

    fn until_end(&mut self) -> Vec<&'a [u8]> {
        let mut v = Vec::new();
        while let Some(t) = self.next() {
            if t == b"$end" {
                break;
            }
            v.push(t);
        }
        v
    }
}

fn s(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

pub fn unit_fs(unit: &str) -> Option<f64> {
    Some(match unit {
        "s" => 1e15,
        "ms" => 1e12,
        "us" | "µs" => 1e9,
        "ns" => 1e6,
        "ps" => 1e3,
        "fs" => 1.0,
        _ => return None,
    })
}

fn parse_timescale(toks: &[&[u8]]) -> Result<f64, String> {
    let joined: String = toks.iter().map(|t| s(t)).collect::<Vec<_>>().join("");
    let j = joined.trim();
    let split = j
        .find(|c: char| !(c.is_ascii_digit() || c == '.'))
        .unwrap_or(j.len());
    let (num, unit) = j.split_at(split);
    let num: f64 = if num.is_empty() {
        1.0
    } else {
        num.parse().map_err(|_| format!("bad $timescale: {j}"))?
    };
    let u = unit_fs(unit.trim()).ok_or_else(|| format!("bad $timescale unit: {j}"))?;
    Ok(num * u)
}

fn parse_range(r: &str) -> Option<(i64, i64)> {
    let r = r.trim().strip_prefix('[')?.strip_suffix(']')?;
    if let Some((a, b)) = r.split_once(':') {
        Some((a.trim().parse().ok()?, b.trim().parse().ok()?))
    } else {
        let v = r.trim().parse().ok()?;
        Some((v, v))
    }
}

fn parse_u64(b: &[u8]) -> Option<u64> {
    if b.is_empty() {
        return None;
    }
    let mut v: u64 = 0;
    for &c in b {
        if !c.is_ascii_digit() {
            // Fallback for odd writers emitting e.g. "#1.0" or "#1e3".
            return std::str::from_utf8(b)
                .ok()?
                .parse::<f64>()
                .ok()
                .map(|f| f.max(0.0) as u64);
        }
        v = v.checked_mul(10)?.checked_add((c - b'0') as u64)?;
    }
    Some(v)
}

pub fn parse(input: &[u8]) -> Result<Vcd, String> {
    let mut toks = Tokens { s: input, pos: 0 };
    let mut vcd = Vcd {
        timescale_fs: 1e6,
        scopes: vec![Scope {
            name: String::new(),
            kind: String::new(),
            path: String::new(),
            parent: None,
            children: Vec::new(),
            vars: Vec::new(),
        }],
        vars: Vec::new(),
        signals: Vec::new(),
        start_time: 0,
        end_time: 0,
    };
    let mut stack: Vec<usize> = vec![0];
    let mut ids: HashMap<&[u8], usize> = HashMap::new();
    let mut pending: Option<&[u8]> = None;

    // ---- header ----
    while let Some(tok) = toks.next() {
        match tok {
            b"$timescale" => {
                let b = toks.until_end();
                vcd.timescale_fs = parse_timescale(&b)?;
            }
            b"$scope" => {
                let b = toks.until_end();
                let kind = b.first().map(|t| s(t)).unwrap_or_default();
                let name = b.get(1).map(|t| s(t)).unwrap_or_else(|| "?".into());
                let parent = *stack.last().unwrap();
                let ppath = &vcd.scopes[parent].path;
                let path = if ppath.is_empty() {
                    name.clone()
                } else {
                    format!("{ppath}.{name}")
                };
                let id = vcd.scopes.len();
                vcd.scopes.push(Scope {
                    name,
                    kind,
                    path,
                    parent: Some(parent),
                    children: Vec::new(),
                    vars: Vec::new(),
                });
                vcd.scopes[parent].children.push(id);
                stack.push(id);
            }
            b"$upscope" => {
                toks.until_end();
                if stack.len() > 1 {
                    stack.pop();
                }
            }
            b"$var" => {
                let b = toks.until_end();
                if b.len() < 4 {
                    continue;
                }
                let vtype = s(b[0]);
                let width: usize = s(b[1]).parse().unwrap_or(1).max(1);
                let code = b[2];
                let mut name = s(b[3]);
                let rest: String = b[4..].iter().map(|t| s(t)).collect::<Vec<_>>().join("");
                let mut range = if rest.starts_with('[') {
                    parse_range(&rest)
                } else {
                    None
                };
                if range.is_none()
                    && width > 1
                    && name.ends_with(']')
                    && let Some(p) = name.rfind('[')
                    && name[p..].contains(':')
                    && let Some(r) = parse_range(&name[p..])
                {
                    range = Some(r);
                    name.truncate(p);
                }
                let kind = match vtype.as_str() {
                    "real" | "realtime" | "shortreal" => SigKind::Real,
                    "string" => SigKind::Str,
                    _ => SigKind::Bits,
                };
                let bits_w = if kind == SigKind::Bits { width } else { 1 };
                let (msb, lsb) = range.unwrap_or((bits_w as i64 - 1, 0));
                let display = match range {
                    Some((a, b)) if a == b => format!("{name}[{a}]"),
                    Some((a, b)) => format!("{name}[{a}:{b}]"),
                    None => name.clone(),
                };
                let signal = *ids.entry(code).or_insert_with(|| {
                    vcd.signals.push(Signal::new(kind, bits_w));
                    vcd.signals.len() - 1
                });
                let scope = *stack.last().unwrap();
                let spath = &vcd.scopes[scope].path;
                let path = if spath.is_empty() {
                    name.clone()
                } else {
                    format!("{spath}.{name}")
                };
                let vid = vcd.vars.len();
                vcd.vars.push(Var {
                    name,
                    display,
                    path,
                    vtype,
                    width: vcd.signals[signal].width,
                    msb,
                    lsb,
                    signal,
                    scope,
                });
                vcd.scopes[scope].vars.push(vid);
            }
            b"$enddefinitions" => {
                toks.until_end();
                break;
            }
            t if t.starts_with(b"$") => {
                toks.until_end();
            }
            _ => {
                pending = Some(tok);
                break;
            }
        }
    }

    // ---- value changes ----
    let mut cur: u64 = 0;
    let mut seen_time = false;
    let mut next = pending.or_else(|| toks.next());
    while let Some(tok) = next {
        match tok[0] {
            b'#' => {
                cur = parse_u64(&tok[1..]).ok_or_else(|| format!("bad timestamp: {}", s(tok)))?;
                if !seen_time {
                    vcd.start_time = cur;
                    seen_time = true;
                }
                if cur > vcd.end_time {
                    vcd.end_time = cur;
                }
            }
            b'b' | b'B' => {
                let id = toks.next().ok_or("unexpected end of file")?;
                if let Some(&sg) = ids.get(id) {
                    vcd.signals[sg].push_bits(cur, &tok[1..]);
                }
            }
            b'r' | b'R' => {
                let id = toks.next().ok_or("unexpected end of file")?;
                if let Some(&sg) = ids.get(id) {
                    let v = std::str::from_utf8(&tok[1..])
                        .ok()
                        .and_then(|x| x.parse::<f64>().ok())
                        .unwrap_or(f64::NAN);
                    vcd.signals[sg].push_real(cur, v);
                }
            }
            b's' | b'S' => {
                let id = toks.next().ok_or("unexpected end of file")?;
                if let Some(&sg) = ids.get(id) {
                    vcd.signals[sg].push_str(cur, &s(&tok[1..]));
                }
            }
            b'$' => {
                if tok == b"$comment" {
                    toks.until_end();
                }
                // $dumpvars / $dumpall / $dumpon / $dumpoff / $end: just markers.
            }
            _ => {
                let (val, id) = if tok.len() > 1 {
                    (&tok[..1], &tok[1..])
                } else {
                    (tok, toks.next().unwrap_or(b""))
                };
                if let Some(&sg) = ids.get(id) {
                    vcd.signals[sg].push_bits(cur, val);
                }
            }
        }
        next = toks.next();
    }
    if vcd.end_time < vcd.start_time {
        vcd.end_time = vcd.start_time;
    }
    Ok(vcd)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "$date today $end
$timescale 10 ps $end
$scope module top $end
$var wire 1 ! clk $end
$var wire 8 \" data [7:0] $end
$var reg 4 # cnt[3:0] $end
$var real 64 $ r $end
$scope module sub $end
$var wire 1 ! clk_alias $end
$upscope $end
$upscope $end
$enddefinitions $end
#0
$dumpvars
0!
bx \"
b0 #
r0.5 $
$end
#5
1!
b1010 \"
#10
0!
b11 #
#10
b11 #
#15
1!
r1.25 $
";

    #[test]
    fn parses_sample() {
        let v = parse(SAMPLE.as_bytes()).unwrap();
        assert_eq!(v.timescale_fs, 10_000.0);
        assert_eq!(v.vars.len(), 5);
        assert_eq!(v.signals.len(), 4);
        assert_eq!(v.vars[1].display, "data[7:0]");
        assert_eq!(v.vars[2].display, "cnt[3:0]");
        assert_eq!(v.vars[2].name, "cnt");
        assert_eq!(v.vars[4].path, "top.sub.clk_alias");
        assert_eq!(v.vars[4].signal, v.vars[0].signal);
        let clk = v.signal_of(0);
        assert_eq!(clk.times, vec![0, 5, 10, 15]);
        let data = v.signal_of(1);
        assert_eq!(data.bits(0), b"xxxxxxxx");
        assert_eq!(data.bits(1), b"00001010");
        let cnt = v.signal_of(2);
        assert_eq!(cnt.times, vec![0, 10]);
        assert_eq!(cnt.bits(1), b"0011");
        assert_eq!(v.signal_of(3).real(1), 1.25);
        assert_eq!(v.end_time, 15);
        assert_eq!(clk.index_at(7), Some(1));
        assert_eq!(v.vars[1].bit_label(0), 7);
    }
}
