//! Application state and keyboard handling.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::format::{self, Radix};
use crate::vcd::{SigKind, Signal, Vcd};

/// Every displayed signal occupies this many terminal rows.
pub const ROW_H: usize = 2;

#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub var: usize,
    pub radix: Radix,
    /// Character position inside a vector (0 = MSB) when this row shows one bit.
    pub bit: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Normal,
    Command,
    Search,
    Tree,
    TreeFilter,
    Help,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TreeNode {
    Scope(usize, usize),
    Var(usize, usize),
}

#[derive(Default, Clone, Copy)]
pub struct Layout {
    pub width: u16,
    pub height: u16,
    pub tree_w: u16,
    pub names_x: u16,
    pub name_w: u16,
    pub wave_x: u16,
    pub wave_w: u16,
    pub rows_y: u16,
    pub rows_h: u16,
}

pub struct App {
    pub vcd: Vcd,
    pub path: PathBuf,
    pub items: Vec<Item>,
    pub sel: usize,
    pub top: usize,
    pub view_start: f64,
    /// Time units per terminal column.
    pub zoom: f64,
    /// Half period of the fastest clock-like signal; zoom steps snap to it.
    grid: Option<f64>,
    pub cursor: u64,
    pub markers: [Option<u64>; 26],
    pub last_marker: Option<usize>,
    pub mode: Mode,
    pub input: String,
    pub count: Option<usize>,
    pub pending: Option<char>,
    pub register: Vec<Item>,
    undo: Vec<(Vec<Item>, usize)>,
    redo: Vec<(Vec<Item>, usize)>,
    pub msg: Option<(String, bool)>,
    pub tree_visible: bool,
    pub expanded: Vec<bool>,
    pub tree_sel: usize,
    pub tree_top: usize,
    pub filter: String,
    pub tree_rows: Vec<TreeNode>,
    pub name_w: u16,
    pub tree_w: u16,
    pub last_search: Option<(String, bool)>,
    pub quit: bool,
    pub clear_screen: bool,
    pub help_scroll: u16,
    pub layout: Layout,
}

const MIN_ZOOM: f64 = 1.0 / 16.0;
/// Tolerance (in columns) so times exactly on a column boundary are not
/// pushed into the previous column by floating-point error.
const COL_EPS: f64 = 1e-9;

/// Most common interval between changes of the busiest 1-bit signal, i.e.
/// the half period of the fastest clock. `None` if nothing looks periodic.
fn clock_step(vcd: &Vcd) -> Option<f64> {
    let s = vcd
        .signals
        .iter()
        .filter(|s| s.kind == SigKind::Bits && s.width == 1 && s.times.len() >= 3)
        .max_by_key(|s| s.times.len())?;
    let mut counts: HashMap<u64, usize> = HashMap::new();
    for w in s.times.windows(2) {
        *counts.entry(w[1] - w[0]).or_default() += 1;
    }
    counts
        .into_iter()
        .filter(|&(d, _)| d > 0)
        .max_by_key(|&(d, n)| (n, std::cmp::Reverse(d)))
        .map(|(d, _)| d as f64)
}

impl App {
    pub fn new(vcd: Vcd, path: PathBuf) -> App {
        let n = vcd.scopes.len();
        let mut app = App {
            vcd,
            path,
            items: Vec::new(),
            sel: 0,
            top: 0,
            view_start: 0.0,
            zoom: 1.0,
            grid: None,
            cursor: 0,
            markers: [None; 26],
            last_marker: None,
            mode: Mode::Normal,
            input: String::new(),
            count: None,
            pending: None,
            register: Vec::new(),
            undo: Vec::new(),
            redo: Vec::new(),
            msg: None,
            tree_visible: false,
            expanded: vec![false; n],
            tree_sel: 0,
            tree_top: 0,
            filter: String::new(),
            tree_rows: Vec::new(),
            name_w: 26,
            tree_w: 32,
            last_search: None,
            quit: false,
            clear_screen: false,
            help_scroll: 0,
            layout: Layout::default(),
        };
        app.cursor = app.vcd.start_time;
        app.grid = clock_step(&app.vcd);
        // Expand the top-level scopes so the tree is useful right away.
        let roots = app.vcd.scopes[0].children.clone();
        for r in roots {
            app.expanded[r] = true;
        }
        app.refresh_tree();
        app
    }

    pub fn default_save_path(&self) -> PathBuf {
        self.path.with_extension("twv")
    }

    // ------------------------------------------------------------------
    // basic accessors
    // ------------------------------------------------------------------

    pub fn sig(&self, item: &Item) -> &Signal {
        self.vcd.signal_of(item.var)
    }

    pub fn is_scalar(&self, item: &Item) -> bool {
        let s = self.sig(item);
        s.kind == SigKind::Bits && (s.width == 1 || item.bit.is_some())
    }

    pub fn item_label(&self, item: &Item) -> String {
        let v = &self.vcd.vars[item.var];
        match item.bit {
            Some(k) => format!("  {}[{}]", v.name, v.bit_label(k)),
            None => v.display.clone(),
        }
    }

    pub fn value_text(&self, item: &Item, idx: usize) -> String {
        let s = self.sig(item);
        match s.kind {
            SigKind::Bits => match item.bit {
                Some(k) => (s.bit(idx, Some(k)) as char).to_string(),
                None => format::format_bits(s.bits(idx), item.radix),
            },
            SigKind::Real => format::format_real(s.real(idx)),
            SigKind::Str => s
                .string(idx)
                .chars()
                .map(|c| {
                    if c.is_ascii_graphic() || c == ' ' {
                        c
                    } else {
                        '.'
                    }
                })
                .collect(),
        }
    }

    pub fn value_at_cursor(&self, item: &Item) -> String {
        match self.sig(item).index_at(self.cursor) {
            Some(i) => self.value_text(item, i),
            None => "-".into(),
        }
    }

    pub fn fmt_time(&self, t: u64) -> String {
        format::fmt_time(t, self.vcd.timescale_fs)
    }

    // ------------------------------------------------------------------
    // layout & view
    // ------------------------------------------------------------------

    pub fn update_layout(&mut self, width: u16, height: u16) {
        let tree_w = if self.tree_visible {
            self.tree_w.min(width.saturating_sub(20))
        } else {
            0
        };
        let names_x = tree_w;
        let avail = width.saturating_sub(names_x);
        let name_w = self.name_w.min(avail.saturating_sub(10)).max(4.min(avail));
        let wave_x = names_x + name_w + 1;
        let wave_w = width.saturating_sub(wave_x).max(1);
        let rows_y = 3;
        let rows_h = height.saturating_sub(rows_y + 1);
        self.layout = Layout {
            width,
            height,
            tree_w,
            names_x,
            name_w,
            wave_x,
            wave_w,
            rows_y,
            rows_h,
        };
        self.fix_scroll();
        self.fix_tree_scroll();
    }

    pub fn visible_items(&self) -> usize {
        (self.layout.rows_h as usize / ROW_H).max(1)
    }

    fn wave_w(&self) -> f64 {
        self.layout.wave_w.max(1) as f64
    }

    fn span(&self) -> u64 {
        self.vcd.end_time - self.vcd.start_time
    }

    fn max_zoom(&self) -> f64 {
        (self.span().max(1) as f64 / self.wave_w() * 2.0).max(self.fit_zoom())
    }

    /// Smallest zoom that shows the whole time range and is on the clock
    /// grid (see `snap_zoom`). Rounds up, so the right edge may stay empty.
    fn fit_zoom(&self) -> f64 {
        let span = self.span();
        if span == 0 {
            return 1.0;
        }
        let w = (self.wave_w() - 1.0).max(1.0);
        let need = (span as f64 / w).max(MIN_ZOOM);
        match self.grid {
            Some(g) if need <= g => {
                g / (g / need)
                    .floor()
                    .clamp(1.0, (g / MIN_ZOOM).floor().max(1.0))
            }
            Some(g) => g * (need / g).ceil(),
            None => need,
        }
    }

    pub fn fit(&mut self) {
        self.zoom = self.fit_zoom();
        self.view_start = self.vcd.start_time as f64;
        self.clamp_view();
    }

    fn clamp_view(&mut self) {
        let max = self.vcd.end_time as f64;
        if self.view_start > max {
            self.view_start = max;
        }
        if self.view_start < 0.0 {
            self.view_start = 0.0;
        }
    }

    /// Column of time `t` relative to the waveform area (may be off-screen).
    pub fn col_of(&self, t: u64) -> f64 {
        ((t as f64 - self.view_start) / self.zoom + COL_EPS).floor()
    }

    /// First integer time that belongs to column `x` (inverse of `col_of`).
    pub fn col_time(&self, x: usize) -> u64 {
        let t = self.view_start + (x as f64 - COL_EPS) * self.zoom;
        if t <= 0.0 { 0 } else { t.ceil() as u64 }
    }

    /// Snap a zoom value so that the clock half period spans a whole number
    /// of columns (or a column spans a whole number of half periods).
    /// Otherwise equal pulses round to columns of unequal width.
    fn snap_zoom(&self, z: f64) -> f64 {
        let max = self.max_zoom();
        let z = z.clamp(MIN_ZOOM, max);
        let Some(g) = self.grid else { return z };
        if z <= g {
            let kmin = (g / max).ceil().max(1.0);
            let kmax = (g / MIN_ZOOM).floor().max(kmin);
            g / (g / z).round().clamp(kmin, kmax)
        } else {
            g * (z / g).round().clamp(1.0, (max / g).floor().max(1.0))
        }
    }

    fn cursor_visible(&self) -> bool {
        let c = self.col_of(self.cursor);
        c >= 0.0 && c < self.wave_w()
    }

    fn ensure_cursor_visible(&mut self) {
        let w = self.wave_w();
        let so = (w / 8.0).min(5.0).floor();
        let c = self.col_of(self.cursor);
        if c < so {
            self.view_start = self.cursor as f64 - so * self.zoom;
        } else if c > w - 1.0 - so {
            self.view_start = self.cursor as f64 - (w - 1.0 - so) * self.zoom;
        }
        self.clamp_view();
    }

    pub fn set_cursor(&mut self, t: u64) {
        self.cursor = t.min(self.vcd.end_time);
        self.ensure_cursor_visible();
    }

    fn center_cursor(&mut self) {
        self.view_start = self.cursor as f64 - self.wave_w() / 2.0 * self.zoom;
        self.clamp_view();
    }

    fn move_cursor_cols(&mut self, n: i64) {
        let step = self.zoom.max(1.0);
        let t = (self.cursor as f64 + n as f64 * step).round().max(0.0);
        self.set_cursor(t as u64);
    }

    fn scroll_cols(&mut self, cols: f64) {
        let old = self.view_start;
        self.view_start += cols * self.zoom;
        self.clamp_view();
        let dt = self.view_start - old;
        let t = (self.cursor as f64 + dt).round().max(0.0) as u64;
        self.cursor = t.min(self.vcd.end_time);
        self.ensure_cursor_visible();
    }

    fn zoom_by(&mut self, factor: f64) {
        self.set_zoom(self.snap_zoom(self.zoom * factor));
    }

    fn set_zoom(&mut self, zoom: f64) {
        let w = self.wave_w();
        let (anchor_t, anchor_col) = if self.cursor_visible() {
            (self.cursor as f64, self.col_of(self.cursor))
        } else {
            (self.view_start + w / 2.0 * self.zoom, w / 2.0)
        };
        self.zoom = zoom;
        self.view_start = anchor_t - anchor_col * self.zoom;
        self.clamp_view();
    }

    pub fn fix_scroll(&mut self) {
        if self.items.is_empty() {
            self.sel = 0;
            self.top = 0;
            return;
        }
        if self.sel >= self.items.len() {
            self.sel = self.items.len() - 1;
        }
        let vis = self.visible_items();
        if self.sel < self.top {
            self.top = self.sel;
        } else if self.sel >= self.top + vis {
            self.top = self.sel + 1 - vis;
        }
        let max_top = self.items.len().saturating_sub(vis);
        if self.top > max_top {
            self.top = max_top;
        }
    }

    fn select(&mut self, i: i64) {
        if self.items.is_empty() {
            return;
        }
        self.sel = i.clamp(0, self.items.len() as i64 - 1) as usize;
        self.fix_scroll();
    }

    // ------------------------------------------------------------------
    // edges & search
    // ------------------------------------------------------------------

    fn next_change(&self, item: &Item, from: u64) -> Option<u64> {
        let s = self.sig(item);
        let mut i = s.times.partition_point(|&t| t <= from);
        match item.bit {
            None => s.times.get(i).copied(),
            Some(b) => {
                while i < s.len() {
                    if i == 0 || s.bit(i, Some(b)) != s.bit(i - 1, Some(b)) {
                        return Some(s.times[i]);
                    }
                    i += 1;
                }
                None
            }
        }
    }

    fn prev_change(&self, item: &Item, from: u64) -> Option<u64> {
        let s = self.sig(item);
        let p = s.times.partition_point(|&t| t < from);
        match item.bit {
            None => p.checked_sub(1).map(|i| s.times[i]),
            Some(b) => {
                let mut i = p;
                while i > 0 {
                    i -= 1;
                    if i == 0 || s.bit(i, Some(b)) != s.bit(i - 1, Some(b)) {
                        return Some(s.times[i]);
                    }
                }
                None
            }
        }
    }

    fn jump_edge(&mut self, forward: bool, any: bool, n: usize) {
        if self.items.is_empty() {
            self.error("no signal");
            return;
        }
        for _ in 0..n {
            let targets: Vec<Item> = if any {
                self.items.clone()
            } else {
                vec![self.items[self.sel].clone()]
            };
            let found = targets
                .iter()
                .filter_map(|it| {
                    if forward {
                        self.next_change(it, self.cursor)
                    } else {
                        self.prev_change(it, self.cursor)
                    }
                })
                .reduce(|a, b| if forward { a.min(b) } else { a.max(b) });
            match found {
                Some(t) => self.cursor = t,
                None => break,
            }
        }
        let t = self.cursor;
        self.set_cursor(t);
    }

    fn norm_value(s: &str) -> String {
        let s = s.trim().to_ascii_lowercase();
        let s = s
            .strip_prefix("0x")
            .or_else(|| s.strip_prefix("'h"))
            .or_else(|| s.strip_prefix("0b"))
            .unwrap_or(&s)
            .to_string();
        if s.starts_with('-') {
            return s;
        }
        let t = s.trim_start_matches('0');
        if t.is_empty() && !s.is_empty() {
            "0".into()
        } else {
            t.to_string()
        }
    }

    fn search(&mut self, pat: &str, forward: bool) {
        if self.items.is_empty() {
            self.error("no signal");
            return;
        }
        let item = self.items[self.sel].clone();
        let want = Self::norm_value(pat);
        let s = self.sig(&item);
        let n = s.len();
        let start = s.times.partition_point(|&t| t <= self.cursor);
        let found = if forward {
            (start..n).find(|&i| Self::norm_value(&self.value_text(&item, i)) == want)
        } else {
            let end = s.times.partition_point(|&t| t < self.cursor);
            // value already in effect at the cursor doesn't count as a hit
            (0..end)
                .rev()
                .find(|&i| Self::norm_value(&self.value_text(&item, i)) == want)
        };
        match found {
            Some(i) => {
                let t = self.sig(&item).times[i];
                self.set_cursor(t);
                self.info(format!("/{pat}  @ {}", self.fmt_time(t)));
            }
            None => self.error(format!("pattern not found: {pat}")),
        }
    }

    // ------------------------------------------------------------------
    // list editing
    // ------------------------------------------------------------------

    fn snapshot(&mut self) {
        self.undo.push((self.items.clone(), self.sel));
        if self.undo.len() > 200 {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    pub fn add_var(&mut self, var: usize) {
        self.snapshot();
        let item = Item {
            var,
            radix: Radix::Hex,
            bit: None,
        };
        if self.items.is_empty() {
            self.items.push(item);
            self.sel = 0;
        } else {
            self.sel += 1;
            self.items.insert(self.sel, item);
        }
        self.fix_scroll();
    }

    pub fn add_vars(&mut self, vars: &[usize]) {
        if vars.is_empty() {
            return;
        }
        self.snapshot();
        let at = if self.items.is_empty() {
            0
        } else {
            self.sel + 1
        };
        for (k, &v) in vars.iter().enumerate() {
            self.items.insert(
                at + k,
                Item {
                    var: v,
                    radix: Radix::Hex,
                    bit: None,
                },
            );
        }
        self.sel = at + vars.len() - 1;
        self.fix_scroll();
    }

    fn delete(&mut self, n: usize) {
        if self.items.is_empty() {
            return;
        }
        self.snapshot();
        let end = (self.sel + n).min(self.items.len());
        self.register = self.items.drain(self.sel..end).collect();
        self.fix_scroll();
        self.info(format!("{} signal(s) deleted", self.register.len()));
    }

    fn yank(&mut self, n: usize) {
        if self.items.is_empty() {
            return;
        }
        let end = (self.sel + n).min(self.items.len());
        self.register = self.items[self.sel..end].to_vec();
        self.info(format!("{} signal(s) yanked", self.register.len()));
    }

    fn paste(&mut self, after: bool, n: usize) {
        if self.register.is_empty() {
            self.error("register is empty");
            return;
        }
        self.snapshot();
        let at = if self.items.is_empty() {
            0
        } else if after {
            self.sel + 1
        } else {
            self.sel
        };
        let mut ins = Vec::new();
        for _ in 0..n {
            ins.extend(self.register.iter().cloned());
        }
        let len = ins.len();
        self.items.splice(at..at, ins);
        self.sel = at + len - 1;
        self.fix_scroll();
    }

    fn move_item(&mut self, down: bool) {
        if self.items.len() < 2 {
            return;
        }
        let j = if down {
            if self.sel + 1 >= self.items.len() {
                return;
            }
            self.sel + 1
        } else {
            if self.sel == 0 {
                return;
            }
            self.sel - 1
        };
        self.snapshot();
        self.items.swap(self.sel, j);
        self.sel = j;
        self.fix_scroll();
    }

    fn bits_run(&self, parent: usize) -> usize {
        let var = self.items[parent].var;
        self.items[parent + 1..]
            .iter()
            .take_while(|it| it.var == var && it.bit.is_some())
            .count()
    }

    fn expand(&mut self) {
        let Some(item) = self.items.get(self.sel).cloned() else {
            return;
        };
        let s = self.sig(&item);
        if item.bit.is_some() || s.kind != SigKind::Bits || s.width < 2 {
            self.error("not a vector");
            return;
        }
        if self.bits_run(self.sel) > 0 {
            return;
        }
        let w = s.width;
        self.snapshot();
        let at = self.sel + 1;
        let bits: Vec<Item> = (0..w)
            .map(|k| Item {
                var: item.var,
                radix: item.radix,
                bit: Some(k),
            })
            .collect();
        self.items.splice(at..at, bits);
        self.fix_scroll();
    }

    fn collapse(&mut self) {
        let Some(item) = self.items.get(self.sel).cloned() else {
            return;
        };
        let mut parent = self.sel;
        if item.bit.is_some() {
            while parent > 0
                && self.items[parent].bit.is_some()
                && self.items[parent].var == item.var
            {
                parent -= 1;
            }
            if !(self.items[parent].var == item.var && self.items[parent].bit.is_none()) {
                // orphaned bit rows: just remove the run containing the cursor
                let mut st = self.sel;
                while st > 0
                    && self.items[st - 1].var == item.var
                    && self.items[st - 1].bit.is_some()
                {
                    st -= 1;
                }
                let mut en = self.sel;
                while en + 1 < self.items.len()
                    && self.items[en + 1].var == item.var
                    && self.items[en + 1].bit.is_some()
                {
                    en += 1;
                }
                self.snapshot();
                self.items.drain(st..=en);
                self.sel = st.saturating_sub(1);
                self.fix_scroll();
                return;
            }
        }
        let run = self.bits_run(parent);
        if run == 0 {
            return;
        }
        self.snapshot();
        self.items.drain(parent + 1..parent + 1 + run);
        self.sel = parent;
        self.fix_scroll();
    }

    fn toggle_fold(&mut self) {
        let Some(item) = self.items.get(self.sel) else {
            return;
        };
        if item.bit.is_some() || self.bits_run(self.sel) > 0 {
            self.collapse();
        } else {
            self.expand();
        }
    }

    fn set_radix(&mut self, r: Option<Radix>) {
        if self.items.is_empty() {
            return;
        }
        self.snapshot();
        let var = self.items[self.sel].var;
        let r = r.unwrap_or_else(|| self.items[self.sel].radix.next());
        self.items[self.sel].radix = r;
        // keep expanded bit rows consistent
        let mut i = self.sel + 1;
        while i < self.items.len() && self.items[i].var == var && self.items[i].bit.is_some() {
            self.items[i].radix = r;
            i += 1;
        }
        self.info(format!("radix: {}", r.name()));
    }

    fn undo(&mut self, redo: bool) {
        let (from, to) = if redo {
            (&mut self.redo, &mut self.undo)
        } else {
            (&mut self.undo, &mut self.redo)
        };
        match from.pop() {
            Some((items, sel)) => {
                to.push((std::mem::replace(&mut self.items, items), self.sel));
                self.sel = sel;
                self.fix_scroll();
            }
            None => {
                let m = if redo {
                    "Already at newest change"
                } else {
                    "Already at oldest change"
                };
                self.error(m);
            }
        }
    }

    // ------------------------------------------------------------------
    // messages
    // ------------------------------------------------------------------

    pub fn info(&mut self, s: impl Into<String>) {
        self.msg = Some((s.into(), false));
    }

    pub fn error(&mut self, s: impl Into<String>) {
        self.msg = Some((s.into(), true));
    }

    // ------------------------------------------------------------------
    // tree
    // ------------------------------------------------------------------

    pub fn refresh_tree(&mut self) {
        let mut rows = Vec::new();
        if self.filter.is_empty() {
            self.push_scope_rows(0, 0, &mut rows);
        } else {
            let terms: Vec<String> = self
                .filter
                .split_whitespace()
                .map(|t| t.to_ascii_lowercase())
                .collect();
            for (i, v) in self.vcd.vars.iter().enumerate() {
                let p = v.full_display(&self.vcd.scopes).to_ascii_lowercase();
                if terms.iter().all(|t| p.contains(t.as_str())) {
                    rows.push(TreeNode::Var(i, 0));
                }
            }
        }
        self.tree_rows = rows;
        if self.tree_sel >= self.tree_rows.len() {
            self.tree_sel = self.tree_rows.len().saturating_sub(1);
        }
        self.fix_tree_scroll();
    }

    fn push_scope_rows(&self, scope: usize, depth: usize, rows: &mut Vec<TreeNode>) {
        let sc = &self.vcd.scopes[scope];
        for &c in &sc.children {
            rows.push(TreeNode::Scope(c, depth));
            if self.expanded[c] {
                self.push_scope_rows(c, depth + 1, rows);
            }
        }
        for &v in &sc.vars {
            rows.push(TreeNode::Var(v, depth));
        }
    }

    pub fn tree_visible_rows(&self) -> usize {
        // header row is used for the title
        (self.layout.height.saturating_sub(3) as usize).max(1)
    }

    fn fix_tree_scroll(&mut self) {
        let vis = self.tree_visible_rows();
        if self.tree_sel < self.tree_top {
            self.tree_top = self.tree_sel;
        } else if self.tree_sel >= self.tree_top + vis {
            self.tree_top = self.tree_sel + 1 - vis;
        }
    }

    fn tree_move(&mut self, d: i64) {
        if self.tree_rows.is_empty() {
            return;
        }
        self.tree_sel =
            (self.tree_sel as i64 + d).clamp(0, self.tree_rows.len() as i64 - 1) as usize;
        self.fix_tree_scroll();
    }

    fn scope_vars_recursive(&self, scope: usize, out: &mut Vec<usize>) {
        let sc = &self.vcd.scopes[scope];
        out.extend(sc.vars.iter().copied());
        for &c in &sc.children {
            self.scope_vars_recursive(c, out);
        }
    }

    fn open_tree(&mut self) {
        self.mode = Mode::Tree;
        self.set_tree_visible(true);
    }

    fn close_tree_focus(&mut self) {
        self.mode = Mode::Normal;
    }

    /// Show/hide the tree pane while keeping the same time range on screen.
    fn set_tree_visible(&mut self, v: bool) {
        if self.tree_visible == v {
            return;
        }
        let old_w = self.wave_w();
        self.tree_visible = v;
        let (w, h) = (self.layout.width, self.layout.height);
        self.update_layout(w, h);
        let new_w = self.wave_w();
        self.zoom = self.snap_zoom(self.zoom * old_w / new_w);
        self.clamp_view();
    }

    // ------------------------------------------------------------------
    // key handling
    // ------------------------------------------------------------------

    pub fn on_key(&mut self, k: KeyEvent) {
        match self.mode {
            Mode::Normal => {
                self.msg = None;
                self.normal_key(k)
            }
            Mode::Command | Mode::Search | Mode::TreeFilter => self.input_key(k),
            Mode::Tree => {
                self.msg = None;
                self.tree_key(k)
            }
            Mode::Help => self.help_key(k),
        }
    }

    fn take_count(&mut self) -> usize {
        self.count.take().unwrap_or(1).max(1)
    }

    fn normal_key(&mut self, k: KeyEvent) {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        let alt = k.modifiers.contains(KeyModifiers::ALT);

        if let Some(p) = self.pending.take() {
            let c = match k.code {
                KeyCode::Char(c) => c,
                _ => {
                    self.count = None;
                    return;
                }
            };
            let n = self.take_count();
            match (p, c) {
                ('g', 'g') => {
                    self.select(n as i64 - 1);
                }
                ('z', 'z') | ('z', '.') => self.center_cursor(),
                ('z', 'o') | ('z', 'O') => self.expand(),
                ('z', 'c') | ('z', 'C') => self.collapse(),
                ('z', 'a') | ('z', 'A') => self.toggle_fold(),
                ('z', 'f') | ('z', 'F') => self.fit(),
                ('d', 'd') => self.delete(n),
                ('y', 'y') => self.yank(n),
                ('m', c) if c.is_ascii_lowercase() => {
                    let i = (c as u8 - b'a') as usize;
                    self.markers[i] = Some(self.cursor);
                    self.last_marker = Some(i);
                    self.info(format!("mark '{c}' set at {}", self.fmt_time(self.cursor)));
                }
                ('\'', c) | ('`', c) if c.is_ascii_lowercase() => {
                    let i = (c as u8 - b'a') as usize;
                    match self.markers[i] {
                        Some(t) => {
                            self.last_marker = Some(i);
                            self.set_cursor(t);
                        }
                        None => self.error(format!("E20: Mark not set: {c}")),
                    }
                }
                _ => {}
            }
            return;
        }

        if let KeyCode::Char(d) = k.code
            && d.is_ascii_digit()
            && !ctrl
            && !alt
            && (d != '0' || self.count.is_some())
        {
            let v = self.count.unwrap_or(0).saturating_mul(10) + (d as usize - '0' as usize);
            self.count = Some(v.min(100_000));
            return;
        }

        if ctrl {
            let n = self.take_count();
            match k.code {
                KeyCode::Char('d') => {
                    let h = (self.visible_items() / 2).max(1) * n;
                    self.top = (self.top + h).min(self.items.len().saturating_sub(1));
                    self.select(self.sel as i64 + h as i64);
                }
                KeyCode::Char('u') => {
                    let h = (self.visible_items() / 2).max(1) * n;
                    self.top = self.top.saturating_sub(h);
                    self.select(self.sel as i64 - h as i64);
                }
                KeyCode::Char('e') => {
                    let max_top = self.items.len().saturating_sub(self.visible_items());
                    self.top = (self.top + n).min(max_top);
                    if self.sel < self.top {
                        self.sel = self.top;
                    }
                }
                KeyCode::Char('y') => {
                    self.top = self.top.saturating_sub(n);
                    let vis = self.visible_items();
                    if self.sel >= self.top + vis {
                        self.sel = self.top + vis - 1;
                    }
                }
                KeyCode::Char('f') => self.scroll_cols(self.wave_w() * n as f64),
                KeyCode::Char('b') => self.scroll_cols(-self.wave_w() * n as f64),
                KeyCode::Char('r') => {
                    for _ in 0..n {
                        self.undo(true)
                    }
                }
                KeyCode::Char('l') => self.clear_screen = true,
                KeyCode::Char('c') => self.info("Type  :q  and press <Enter> to exit"),
                _ => {}
            }
            return;
        }

        if alt {
            match k.code {
                KeyCode::Char('j') | KeyCode::Down => self.move_item(true),
                KeyCode::Char('k') | KeyCode::Up => self.move_item(false),
                _ => {}
            }
            self.count = None;
            return;
        }

        match k.code {
            KeyCode::Char('g')
            | KeyCode::Char('z')
            | KeyCode::Char('d')
            | KeyCode::Char('y')
            | KeyCode::Char('m')
            | KeyCode::Char('\'')
            | KeyCode::Char('`') => {
                if let KeyCode::Char(c) = k.code {
                    self.pending = Some(c);
                }
                return;
            }
            _ => {}
        }

        let given = self.count.is_some();
        let n = self.take_count();
        match k.code {
            KeyCode::Char('h') | KeyCode::Left => self.move_cursor_cols(-(n as i64)),
            KeyCode::Char('l') | KeyCode::Right => self.move_cursor_cols(n as i64),
            KeyCode::Char('H') => self.scroll_cols(-(self.wave_w() / 2.0).floor() * n as f64),
            KeyCode::Char('L') => self.scroll_cols((self.wave_w() / 2.0).floor() * n as f64),
            KeyCode::Char('k') | KeyCode::Char('+') | KeyCode::Char('=') => {
                self.zoom_by(0.5f64.powi(n as i32))
            }
            KeyCode::Char('j') | KeyCode::Char('-') | KeyCode::Char('_') => {
                self.zoom_by(2f64.powi(n as i32))
            }
            KeyCode::Char('F') => self.fit(),
            KeyCode::Char('0') | KeyCode::Char('^') | KeyCode::Home => {
                let t = self.vcd.start_time;
                self.set_cursor(t)
            }
            KeyCode::Char('$') | KeyCode::End => {
                let t = self.vcd.end_time;
                self.set_cursor(t)
            }
            KeyCode::Char('w') | KeyCode::Char('e') => self.jump_edge(true, false, n),
            KeyCode::Char('b') => self.jump_edge(false, false, n),
            KeyCode::Char('W') | KeyCode::Char('E') => self.jump_edge(true, true, n),
            KeyCode::Char('B') => self.jump_edge(false, true, n),
            KeyCode::Char('J') | KeyCode::Down => self.select(self.sel as i64 + n as i64),
            KeyCode::Char('K') | KeyCode::Up => self.select(self.sel as i64 - n as i64),
            KeyCode::PageDown => {
                let v = self.visible_items() as i64;
                self.select(self.sel as i64 + v * n as i64)
            }
            KeyCode::PageUp => {
                let v = self.visible_items() as i64;
                self.select(self.sel as i64 - v * n as i64)
            }
            KeyCode::Char('G') => {
                if given {
                    self.select(n as i64 - 1)
                } else {
                    self.select(i64::MAX / 2)
                }
            }
            KeyCode::Char('x') | KeyCode::Delete => self.delete(n),
            KeyCode::Char('p') => self.paste(true, n),
            KeyCode::Char('P') => self.paste(false, n),
            KeyCode::Char('u') => {
                for _ in 0..n {
                    self.undo(false)
                }
            }
            KeyCode::Char('r') => self.set_radix(None),
            KeyCode::Char('R') => self.reload(),
            KeyCode::Char('n') => self.repeat_search(false),
            KeyCode::Char('N') => self.repeat_search(true),
            KeyCode::Char(':') => {
                self.mode = Mode::Command;
                self.input.clear();
            }
            KeyCode::Char('/') => {
                self.mode = Mode::Search;
                self.input.clear();
            }
            KeyCode::Char('?') | KeyCode::F(1) => {
                self.mode = Mode::Help;
                self.help_scroll = 0;
            }
            KeyCode::Tab | KeyCode::Char('a') | KeyCode::Char('i') | KeyCode::Char('o') => {
                self.open_tree()
            }
            KeyCode::Char('t') => {
                let v = !self.tree_visible;
                self.set_tree_visible(v)
            }
            KeyCode::Char('<') => {
                self.name_w = self.name_w.saturating_sub(2 * n as u16).max(8);
                self.relayout();
            }
            KeyCode::Char('>') => {
                self.name_w = (self.name_w + 2 * n as u16).min(120);
                self.relayout();
            }
            KeyCode::Char('q') => self.quit = true,
            KeyCode::Esc => {
                self.count = None;
            }
            _ => {}
        }
    }

    fn relayout(&mut self) {
        let (w, h) = (self.layout.width, self.layout.height);
        self.update_layout(w, h);
    }

    fn repeat_search(&mut self, reverse: bool) {
        match self.last_search.clone() {
            Some((p, fwd)) => self.search(&p, fwd != reverse),
            None => self.error("E35: No previous regular expression"),
        }
    }

    fn input_key(&mut self, k: KeyEvent) {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        match k.code {
            KeyCode::Esc => {
                if self.mode == Mode::TreeFilter {
                    self.filter.clear();
                    self.refresh_tree();
                    self.mode = Mode::Tree;
                } else {
                    self.mode = Mode::Normal;
                }
                self.input.clear();
            }
            KeyCode::Enter => {
                let s = std::mem::take(&mut self.input);
                match self.mode {
                    Mode::Command => {
                        self.mode = Mode::Normal;
                        self.exec_command(&s);
                    }
                    Mode::Search => {
                        self.mode = Mode::Normal;
                        if s.is_empty() {
                            self.repeat_search(false);
                        } else {
                            self.last_search = Some((s.clone(), true));
                            self.search(&s, true);
                        }
                    }
                    Mode::TreeFilter => {
                        self.mode = Mode::Tree;
                    }
                    _ => {}
                }
            }
            KeyCode::Backspace => {
                if self.input.pop().is_none() {
                    if self.mode == Mode::TreeFilter {
                        self.mode = Mode::Tree;
                    } else {
                        self.mode = Mode::Normal;
                    }
                }
                self.sync_filter();
            }
            KeyCode::Char('u') if ctrl => {
                self.input.clear();
                self.sync_filter();
            }
            KeyCode::Char('w') if ctrl => {
                let t = self.input.trim_end().len();
                let cut = self.input[..t].rfind(' ').map(|p| p + 1).unwrap_or(0);
                self.input.truncate(cut);
                self.sync_filter();
            }
            KeyCode::Char('c') if ctrl => {
                self.mode = if self.mode == Mode::TreeFilter {
                    Mode::Tree
                } else {
                    Mode::Normal
                };
                self.input.clear();
            }
            KeyCode::Down | KeyCode::Char('n') if ctrl || k.code == KeyCode::Down => {
                if self.mode == Mode::TreeFilter {
                    self.tree_move(1);
                }
            }
            KeyCode::Up | KeyCode::Char('p') if ctrl || k.code == KeyCode::Up => {
                if self.mode == Mode::TreeFilter {
                    self.tree_move(-1);
                }
            }
            KeyCode::Char(c) if !ctrl => {
                self.input.push(c);
                self.sync_filter();
            }
            _ => {}
        }
    }

    fn sync_filter(&mut self) {
        if self.mode == Mode::TreeFilter {
            self.filter = self.input.clone();
            self.tree_sel = 0;
            self.tree_top = 0;
            self.refresh_tree();
        }
    }

    fn tree_key(&mut self, k: KeyEvent) {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        if let Some(p) = self.pending.take() {
            if p == 'g' && k.code == KeyCode::Char('g') {
                self.tree_sel = 0;
                self.fix_tree_scroll();
            }
            return;
        }
        let half = (self.tree_visible_rows() / 2).max(1) as i64;
        match k.code {
            KeyCode::Char('d') if ctrl => self.tree_move(half),
            KeyCode::Char('u') if ctrl => self.tree_move(-half),
            KeyCode::Char('n') if ctrl => self.tree_move(1),
            KeyCode::Char('p') if ctrl => self.tree_move(-1),
            KeyCode::Char('c') if ctrl => self.close_tree_focus(),
            KeyCode::Char('j') | KeyCode::Down => self.tree_move(1),
            KeyCode::Char('k') | KeyCode::Up => self.tree_move(-1),
            KeyCode::PageDown => self.tree_move(half * 2),
            KeyCode::PageUp => self.tree_move(-half * 2),
            KeyCode::Char('g') => self.pending = Some('g'),
            KeyCode::Char('G') => {
                self.tree_sel = self.tree_rows.len().saturating_sub(1);
                self.fix_tree_scroll();
            }
            KeyCode::Char('l') | KeyCode::Right => match self.tree_rows.get(self.tree_sel).copied()
            {
                Some(TreeNode::Scope(s, _)) => {
                    if self.expanded[s] {
                        self.tree_move(1);
                    } else {
                        self.expanded[s] = true;
                        self.refresh_tree();
                    }
                }
                Some(TreeNode::Var(v, _)) => self.add_var(v),
                None => {}
            },
            KeyCode::Char('h') | KeyCode::Left => {
                match self.tree_rows.get(self.tree_sel).copied() {
                    Some(TreeNode::Scope(s, _)) if self.expanded[s] => {
                        self.expanded[s] = false;
                        self.refresh_tree();
                    }
                    Some(node) => {
                        // jump to parent scope
                        let parent = match node {
                            TreeNode::Scope(s, _) => self.vcd.scopes[s].parent,
                            TreeNode::Var(v, _) => Some(self.vcd.vars[v].scope),
                        };
                        if let Some(p) = parent
                            && let Some(i) = self
                                .tree_rows
                                .iter()
                                .position(|r| *r == TreeNode::Scope(p, r_depth(r)))
                        {
                            self.tree_sel = i;
                            self.fix_tree_scroll();
                        }
                    }
                    None => {}
                }
            }
            KeyCode::Enter | KeyCode::Char(' ') | KeyCode::Char('a') | KeyCode::Char('o') => {
                match self.tree_rows.get(self.tree_sel).copied() {
                    Some(TreeNode::Scope(s, _)) => {
                        if k.code == KeyCode::Char('a') {
                            let vars = self.vcd.scopes[s].vars.clone();
                            self.add_vars(&vars);
                            self.info(format!("{} signal(s) added", vars.len()));
                        } else {
                            self.expanded[s] = !self.expanded[s];
                            self.refresh_tree();
                        }
                    }
                    Some(TreeNode::Var(v, _)) => {
                        self.add_var(v);
                        self.info(format!("added {}", self.vcd.vars[v].display));
                        self.tree_move(1);
                    }
                    None => {}
                }
            }
            KeyCode::Char('A') => match self.tree_rows.get(self.tree_sel).copied() {
                Some(TreeNode::Scope(s, _)) => {
                    let mut vars = Vec::new();
                    self.scope_vars_recursive(s, &mut vars);
                    self.add_vars(&vars);
                    self.info(format!("{} signal(s) added", vars.len()));
                }
                Some(TreeNode::Var(_, _)) => {
                    // add every row currently listed (useful with a filter)
                    let vars: Vec<usize> = self
                        .tree_rows
                        .iter()
                        .filter_map(|r| match r {
                            TreeNode::Var(v, _) => Some(*v),
                            _ => None,
                        })
                        .collect();
                    self.add_vars(&vars);
                    self.info(format!("{} signal(s) added", vars.len()));
                }
                None => {}
            },
            KeyCode::Char('/') => {
                self.mode = Mode::TreeFilter;
                self.input = self.filter.clone();
            }
            KeyCode::Esc => {
                if !self.filter.is_empty() {
                    self.filter.clear();
                    self.refresh_tree();
                } else {
                    self.close_tree_focus();
                }
            }
            KeyCode::Tab => self.close_tree_focus(),
            KeyCode::Char('q') | KeyCode::Char('t') => {
                self.mode = Mode::Normal;
                self.set_tree_visible(false);
            }
            KeyCode::Char(':') => {
                self.mode = Mode::Command;
                self.input.clear();
            }
            KeyCode::Char('?') | KeyCode::F(1) => {
                self.mode = Mode::Help;
                self.help_scroll = 0;
            }
            KeyCode::Char('<') => {
                self.tree_w = self.tree_w.saturating_sub(2).max(12);
                self.relayout();
            }
            KeyCode::Char('>') => {
                self.tree_w = (self.tree_w + 2).min(120);
                self.relayout();
            }
            _ => {}
        }
    }

    fn help_key(&mut self, k: KeyEvent) {
        match k.code {
            KeyCode::Char('j') | KeyCode::Down => {
                self.help_scroll = self.help_scroll.saturating_add(1)
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.help_scroll = self.help_scroll.saturating_sub(1)
            }
            KeyCode::Char('d') | KeyCode::PageDown => {
                self.help_scroll = self.help_scroll.saturating_add(10)
            }
            KeyCode::Char('u') | KeyCode::PageUp => {
                self.help_scroll = self.help_scroll.saturating_sub(10)
            }
            KeyCode::Char('g') => self.help_scroll = 0,
            _ => self.mode = Mode::Normal,
        }
    }

    // ------------------------------------------------------------------
    // ex commands
    // ------------------------------------------------------------------

    fn exec_command(&mut self, cmd: &str) {
        let cmd = cmd.trim();
        if cmd.is_empty() {
            return;
        }
        let (name, arg) = match cmd.split_once(char::is_whitespace) {
            Some((a, b)) => (a, b.trim()),
            None => (cmd, ""),
        };
        match name {
            "q" | "q!" | "quit" | "qa" | "qa!" | "qall" | "x" | "exit" => self.quit = true,
            "w" | "write" => {
                self.save_cmd(arg);
            }
            "wq" | "wq!" => {
                if self.save_cmd(arg) {
                    self.quit = true;
                }
            }
            "e" | "e!" | "edit" | "reload" => self.reload(),
            "add" | "a" => self.add_pattern(arg),
            "radix" | "r" | "fmt" => match Radix::parse(arg) {
                Some(r) => self.set_radix(Some(r)),
                None => self.error("usage: :radix hex|dec|sdec|bin|oct|ascii"),
            },
            "fit" | "f" => self.fit(),
            "zoom" | "z" => match format::parse_time(arg, self.vcd.timescale_fs) {
                Some(t) if t > 0 => {
                    // An explicit value is honoured as is (no snapping).
                    self.set_zoom((t as f64).clamp(MIN_ZOOM, self.max_zoom()));
                }
                _ => self.error("usage: :zoom <time-per-column>  (e.g. :zoom 10ns)"),
            },
            "delm" | "delm!" | "delmarks" | "delmarks!" => {
                if arg.is_empty() || name.ends_with('!') {
                    self.markers = [None; 26];
                    self.last_marker = None;
                } else {
                    for c in arg.chars().filter(|c| c.is_ascii_lowercase()) {
                        self.markers[(c as u8 - b'a') as usize] = None;
                    }
                }
            }
            "marks" => {
                let s: Vec<String> = self
                    .markers
                    .iter()
                    .enumerate()
                    .filter_map(|(i, m)| {
                        m.map(|t| format!("{}:{}", (b'a' + i as u8) as char, self.fmt_time(t)))
                    })
                    .collect();
                if s.is_empty() {
                    self.info("no marks set");
                } else {
                    self.info(s.join("  "));
                }
            }
            "goto" | "go" | "cursor" | "t" => self.goto_time(arg),
            "help" | "h" => {
                self.mode = Mode::Help;
                self.help_scroll = 0;
            }
            "clear" => {
                self.snapshot();
                self.items.clear();
                self.fix_scroll();
            }
            _ if name.starts_with(|c: char| c.is_ascii_digit() || c == '.') => self.goto_time(cmd),
            _ => self.error(format!("E492: Not an editor command: {cmd}")),
        }
    }

    fn goto_time(&mut self, arg: &str) {
        match format::parse_time(arg, self.vcd.timescale_fs) {
            Some(t) => {
                self.set_cursor(t);
                if !self.cursor_visible() {
                    self.center_cursor();
                }
            }
            None => self.error(format!("invalid time: {arg}")),
        }
    }

    fn add_pattern(&mut self, pat: &str) {
        if pat.is_empty() {
            self.error("usage: :add <pattern>   (substring of the full path; '*' = all)");
            return;
        }
        let terms: Vec<String> = pat
            .split_whitespace()
            .map(|t| t.to_ascii_lowercase())
            .collect();
        let vars: Vec<usize> = self
            .vcd
            .vars
            .iter()
            .enumerate()
            .filter(|(_, v)| {
                let p = v.full_display(&self.vcd.scopes).to_ascii_lowercase();
                terms.iter().all(|t| t == "*" || p.contains(t.as_str()))
            })
            .map(|(i, _)| i)
            .collect();
        if vars.is_empty() {
            self.error(format!("no signal matches: {pat}"));
        } else {
            self.add_vars(&vars);
            self.info(format!("{} signal(s) added", vars.len()));
        }
    }

    // ------------------------------------------------------------------
    // save / load / reload
    // ------------------------------------------------------------------

    fn save_cmd(&mut self, arg: &str) -> bool {
        let path = if arg.is_empty() {
            self.default_save_path()
        } else {
            PathBuf::from(arg)
        };
        match std::fs::write(&path, self.save_string()) {
            Ok(()) => {
                self.info(format!(
                    "\"{}\" written ({} signals)",
                    path.display(),
                    self.items.len()
                ));
                true
            }
            Err(e) => {
                self.error(format!(
                    "E212: Can't open file for writing: {} ({e})",
                    path.display()
                ));
                false
            }
        }
    }

    pub fn save_string(&self) -> String {
        let mut s = String::from("# terminalwave layout\n");
        s += &format!("view {} {} {}\n", self.view_start, self.zoom, self.cursor);
        for (i, m) in self.markers.iter().enumerate() {
            if let Some(t) = m {
                s += &format!("marker {} {}\n", (b'a' + i as u8) as char, t);
            }
        }
        for it in &self.items {
            let v = &self.vcd.vars[it.var];
            match it.bit {
                Some(k) => {
                    s += &format!(
                        "bit {} {} {}\n",
                        it.radix.name(),
                        k,
                        v.full_display(&self.vcd.scopes)
                    )
                }
                None => {
                    s += &format!(
                        "signal {} {}\n",
                        it.radix.name(),
                        v.full_display(&self.vcd.scopes)
                    )
                }
            }
        }
        s
    }

    fn var_index(&self) -> HashMap<String, usize> {
        let mut m = HashMap::new();
        for (i, v) in self.vcd.vars.iter().enumerate() {
            m.entry(v.full_display(&self.vcd.scopes)).or_insert(i);
        }
        m
    }

    pub fn load_layout(&mut self, path: &Path) -> Result<(), String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let idx = self.var_index();
        let mut items = Vec::new();
        let mut missing = 0;
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let mut parts = line.splitn(2, ' ');
            let kw = parts.next().unwrap_or("");
            let rest = parts.next().unwrap_or("");
            match kw {
                "view" => {
                    let f: Vec<&str> = rest.split_whitespace().collect();
                    if f.len() == 3
                        && let (Ok(a), Ok(b), Ok(c)) = (f[0].parse(), f[1].parse(), f[2].parse())
                    {
                        self.view_start = a;
                        self.zoom = self.snap_zoom(b);
                        self.cursor = c;
                    }
                }
                "marker" => {
                    let f: Vec<&str> = rest.split_whitespace().collect();
                    if f.len() == 2
                        && let (Some(c), Ok(t)) = (f[0].chars().next(), f[1].parse::<u64>())
                        && c.is_ascii_lowercase()
                    {
                        self.markers[(c as u8 - b'a') as usize] = Some(t);
                    }
                }
                "signal" => {
                    if let Some((r, p)) = rest.split_once(' ') {
                        match idx.get(p.trim()) {
                            Some(&v) => items.push(Item {
                                var: v,
                                radix: Radix::parse(r).unwrap_or(Radix::Hex),
                                bit: None,
                            }),
                            None => missing += 1,
                        }
                    }
                }
                "bit" => {
                    let f: Vec<&str> = rest.splitn(3, ' ').collect();
                    if f.len() == 3 {
                        match (idx.get(f[2].trim()), f[1].parse::<usize>()) {
                            (Some(&v), Ok(k)) if k < self.vcd.vars[v].width => items.push(Item {
                                var: v,
                                radix: Radix::parse(f[0]).unwrap_or(Radix::Hex),
                                bit: Some(k),
                            }),
                            _ => missing += 1,
                        }
                    }
                }
                _ => {}
            }
        }
        self.items = items;
        self.sel = 0;
        self.top = 0;
        self.clamp_view();
        self.fix_scroll();
        if missing > 0 {
            self.error(format!(
                "{missing} signal(s) in {} not found",
                path.display()
            ));
        } else {
            self.info(format!("loaded \"{}\"", path.display()));
        }
        Ok(())
    }

    fn reload(&mut self) {
        let data = match std::fs::read(&self.path) {
            Ok(d) => d,
            Err(e) => {
                self.error(format!("reload failed: {e}"));
                return;
            }
        };
        let vcd = match crate::vcd::parse(&data) {
            Ok(v) => v,
            Err(e) => {
                self.error(format!("reload failed: {e}"));
                return;
            }
        };
        let names: Vec<(String, Item)> = self
            .items
            .iter()
            .map(|it| {
                (
                    self.vcd.vars[it.var].full_display(&self.vcd.scopes),
                    it.clone(),
                )
            })
            .collect();
        let old_expanded: HashMap<String, bool> = self
            .vcd
            .scopes
            .iter()
            .zip(self.expanded.iter())
            .map(|(s, &e)| (s.path.clone(), e))
            .collect();
        self.vcd = vcd;
        self.grid = clock_step(&self.vcd);
        self.expanded = self
            .vcd
            .scopes
            .iter()
            .map(|s| old_expanded.get(&s.path).copied().unwrap_or(false))
            .collect();
        let idx = self.var_index();
        let mut items = Vec::new();
        for (n, it) in names {
            if let Some(&v) = idx.get(&n)
                && it.bit.is_none_or(|k| k < self.vcd.vars[v].width)
            {
                items.push(Item { var: v, ..it });
            }
        }
        self.items = items;
        self.undo.clear();
        self.redo.clear();
        self.cursor = self.cursor.min(self.vcd.end_time);
        self.clamp_view();
        self.fix_scroll();
        self.refresh_tree();
        self.info(format!("reloaded \"{}\"", self.path.display()));
    }
}

fn r_depth(r: &TreeNode) -> usize {
    match r {
        TreeNode::Scope(_, d) | TreeNode::Var(_, d) => *d,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clock_app(half: u64, edges: u64, wave_w: u16) -> App {
        let mut src = String::from(
            "$timescale 1ns $end\n$scope module t $end\n$var wire 1 ! clk $end\n\
             $upscope $end\n$enddefinitions $end\n",
        );
        for i in 0..=edges {
            src += &format!("#{}\n{}!\n", i * half, i % 2);
        }
        let mut app = App::new(
            crate::vcd::parse(src.as_bytes()).unwrap(),
            PathBuf::from("t.vcd"),
        );
        app.layout.wave_w = wave_w;
        app.fit();
        app
    }

    /// Widths (in columns) of the fully visible clock pulses.
    fn pulse_widths(app: &App, half: u64, edges: u64) -> Vec<i64> {
        let w = app.layout.wave_w as f64;
        let cols: Vec<f64> = (0..=edges)
            .map(|i| app.col_of(i * half))
            .filter(|&c| c >= 0.0 && c < w)
            .collect();
        cols.windows(2).map(|p| (p[1] - p[0]) as i64).collect()
    }

    #[test]
    fn clock_pulses_have_equal_width_after_zoom() {
        // Issue #1: odd widths/periods used to give pulses of 2 and 3 columns.
        for (half, edges, wave_w) in [
            (5, 60, 173),
            (10, 37, 131),
            (7, 200, 97),
            (500, 90, 211),
            (5, 12, 80),
        ] {
            let mut app = clock_app(half, edges, wave_w);
            assert_eq!(app.grid, Some(half as f64));
            // :fit / initial view: everything visible and already uniform.
            assert!(app.col_of(app.vcd.end_time) < wave_w as f64);
            if app.zoom <= half as f64 {
                let widths = pulse_widths(&app, half, edges);
                assert!(
                    widths.windows(2).all(|p| p[0] == p[1]),
                    "fit: half={half} w={wave_w} zoom={} widths={widths:?}",
                    app.zoom
                );
            }
            for step in 0..8 {
                app.zoom_by(0.5);
                app.scroll_cols(3.0);
                if app.zoom > half as f64 {
                    continue; // less than one column per pulse
                }
                let widths = pulse_widths(&app, half, edges);
                assert!(
                    widths.windows(2).all(|p| p[0] == p[1]),
                    "half={half} w={wave_w} step={step} zoom={} widths={widths:?}",
                    app.zoom
                );
                // The renderer must agree with col_of on where edges fall.
                for i in 0..=edges {
                    let c = app.col_of(i * half);
                    if c >= 0.0 && c < wave_w as f64 {
                        let x = c as usize;
                        assert!(app.col_time(x) <= i * half && i * half < app.col_time(x + 1));
                    }
                }
            }
            // Zooming back out always makes progress up to the limit.
            let mut prev = app.zoom;
            for _ in 0..20 {
                app.zoom_by(2.0);
                if app.zoom == prev {
                    break;
                }
                assert!(app.zoom > prev);
                prev = app.zoom;
            }
            assert!(app.zoom * app.wave_w() >= app.span() as f64);
        }
    }
}
