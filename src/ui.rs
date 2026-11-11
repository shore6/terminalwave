//! Rendering.

use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph, Wrap};

use crate::app::{App, Item, Mode, ROW_H, TreeNode};
use crate::format;
use crate::vcd::SigKind;

const BAR_BG: Color = Color::Indexed(236);
const SEL_BG: Color = Color::Indexed(235);
const CURSOR_BG: Color = Color::Indexed(239);
const MARKER_BG: Color = Color::Indexed(24);
const WAVE: Color = Color::Green;
const BUS: Color = Color::Indexed(78);
const XCOL: Color = Color::Red;
const XFILL: Color = Color::Indexed(88);
const ZCOL: Color = Color::Yellow;
const TEXT: Color = Color::White;
const DIM: Color = Color::Indexed(244);
const VALUE: Color = Color::Cyan;
const SCOPE: Color = Color::Indexed(75);

fn put(buf: &mut Buffer, x: u16, y: u16, ch: char, fg: Color, bg: Option<Color>) {
    if let Some(c) = buf.cell_mut((x, y)) {
        c.set_char(ch).set_fg(fg);
        if let Some(bg) = bg {
            c.set_bg(bg);
        }
    }
}

fn set_bg(buf: &mut Buffer, x: u16, y: u16, bg: Color) {
    if let Some(c) = buf.cell_mut((x, y)) {
        c.set_bg(bg);
    }
}

/// Write `s` clipped to `w` columns. Returns columns used.
fn text(buf: &mut Buffer, x: u16, y: u16, w: u16, s: &str, style: Style) -> u16 {
    if w == 0 {
        return 0;
    }
    let (nx, _) = buf.set_stringn(x, y, s, w as usize, style);
    nx - x
}

fn truncate(s: &str, w: usize) -> String {
    let n = s.chars().count();
    if n <= w {
        s.to_string()
    } else if w == 0 {
        String::new()
    } else {
        let mut t: String = s.chars().take(w - 1).collect();
        t.push('…');
        t
    }
}

fn truncate_left(s: &str, w: usize) -> String {
    let n = s.chars().count();
    if n <= w {
        s.to_string()
    } else if w == 0 {
        String::new()
    } else {
        let mut t = String::from("…");
        t.extend(s.chars().skip(n - (w - 1)));
        t
    }
}

pub fn draw(f: &mut Frame, app: &mut App) {
    let area = f.area();
    app.update_layout(area.width, area.height);
    if area.width < 20 || area.height < 6 {
        f.render_widget(Paragraph::new("terminal too small"), area);
        return;
    }
    {
        let buf = f.buffer_mut();
        draw_header(buf, app);
        if app.layout.tree_w > 0 {
            draw_tree(buf, app);
        }
        draw_names(buf, app);
        draw_ruler(buf, app);
        draw_waves(buf, app);
        draw_status(buf, app);
    }
    if app.mode == Mode::Help {
        draw_help(f, app);
    }
}

fn draw_header(buf: &mut Buffer, app: &App) {
    let l = app.layout;
    for x in 0..l.width {
        put(buf, x, 0, ' ', TEXT, Some(BAR_BG));
    }
    let ts = app.vcd.timescale_fs;
    let file = app
        .path
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut spans = vec![
        Span::styled(
            " terminalwave ",
            Style::new()
                .fg(Color::Black)
                .bg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!(" {file} "),
            Style::new()
                .fg(TEXT)
                .bg(BAR_BG)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("│ cursor ", Style::new().fg(DIM).bg(BAR_BG)),
        Span::styled(
            app.fmt_time(app.cursor),
            Style::new()
                .fg(Color::Yellow)
                .bg(BAR_BG)
                .add_modifier(Modifier::BOLD),
        ),
    ];
    if let Some(m) = app.last_marker
        && let Some(t) = app.markers[m]
    {
        let d = app.cursor as i128 - t as i128;
        spans.push(Span::styled(
            format!("  Δ'{}' ", (b'a' + m as u8) as char),
            Style::new().fg(DIM).bg(BAR_BG),
        ));
        spans.push(Span::styled(
            format::fmt_delta(d, ts),
            Style::new()
                .fg(Color::LightBlue)
                .bg(BAR_BG)
                .add_modifier(Modifier::BOLD),
        ));
    }
    spans.push(Span::styled(
        format!(
            "  │ {}/col  │ {} – {}  │ ts {} ",
            format::fmt_fs(app.zoom * ts),
            app.fmt_time(app.vcd.start_time),
            app.fmt_time(app.vcd.end_time),
            format::fmt_fs(ts),
        ),
        Style::new().fg(DIM).bg(BAR_BG),
    ));
    buf.set_line(0, 0, &Line::from(spans), l.width);
    let help = " ?:help ";
    let hx = l.width.saturating_sub(help.len() as u16);
    text(
        buf,
        hx,
        0,
        help.len() as u16,
        help,
        Style::new().fg(DIM).bg(BAR_BG),
    );
}

fn draw_tree(buf: &mut Buffer, app: &App) {
    let l = app.layout;
    let w = l.tree_w.saturating_sub(1);
    let focused = matches!(app.mode, Mode::Tree | Mode::TreeFilter);
    let bottom = l.height.saturating_sub(1);
    for y in 1..bottom {
        put(
            buf,
            w,
            y,
            '│',
            if focused { Color::Green } else { DIM },
            None,
        );
    }
    let title = if app.filter.is_empty() && app.mode != Mode::TreeFilter {
        " SIGNALS".to_string()
    } else {
        format!(
            " /{}",
            if app.mode == Mode::TreeFilter {
                &app.input
            } else {
                &app.filter
            }
        )
    };
    let title_style = if focused {
        Style::new()
            .fg(Color::Black)
            .bg(Color::Green)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::new().fg(TEXT).bg(BAR_BG)
    };
    for x in 0..w {
        put(buf, x, 1, ' ', TEXT, Some(title_style.bg.unwrap()));
    }
    text(
        buf,
        0,
        1,
        w,
        &truncate_left(&title, w as usize),
        title_style,
    );
    let count = format!("{} ", app.tree_rows.len());
    if (count.len() as u16) + 4 < w {
        let cs = title_style.remove_modifier(Modifier::BOLD);
        text(
            buf,
            w - count.len() as u16,
            1,
            count.len() as u16,
            &count,
            cs,
        );
    }

    let shown: std::collections::HashSet<usize> = app.items.iter().map(|i| i.var).collect();
    let rows = app.tree_visible_rows();
    for (k, node) in app
        .tree_rows
        .iter()
        .skip(app.tree_top)
        .take(rows)
        .enumerate()
    {
        let idx = app.tree_top + k;
        let y = 2 + k as u16;
        let sel = idx == app.tree_sel;
        let bg = if sel {
            Some(if focused { Color::Indexed(238) } else { SEL_BG })
        } else {
            None
        };
        if let Some(bg) = bg {
            for x in 0..w {
                set_bg(buf, x, y, bg);
            }
        }
        let (label, style) = match *node {
            TreeNode::Scope(s, d) => {
                let sc = &app.vcd.scopes[s];
                let arrow = if app.expanded[s] { '▾' } else { '▸' };
                (
                    format!("{}{} {}", "  ".repeat(d), arrow, sc.name),
                    Style::new().fg(SCOPE).add_modifier(Modifier::BOLD),
                )
            }
            TreeNode::Var(v, d) => {
                let var = &app.vcd.vars[v];
                let mark = if shown.contains(&v) { '●' } else { ' ' };
                let name = if app.filter.is_empty() {
                    format!("{}{} {}", "  ".repeat(d), mark, var.display)
                } else {
                    format!("{} {}", mark, var.full_display(&app.vcd.scopes))
                };
                let fg = if shown.contains(&v) {
                    Color::Green
                } else {
                    TEXT
                };
                (name, Style::new().fg(fg))
            }
        };
        let style = match bg {
            Some(b) => style.bg(b),
            None => style,
        };
        let label = if app.filter.is_empty() {
            truncate(&label, w as usize)
        } else {
            truncate_left(&label, w as usize)
        };
        text(buf, 0, y, w, &label, style);
    }
}

fn draw_names(buf: &mut Buffer, app: &App) {
    let l = app.layout;
    let x0 = l.names_x;
    let w = l.name_w;
    // header
    for x in x0..x0 + w {
        put(buf, x, 1, ' ', TEXT, Some(BAR_BG));
        put(buf, x, 2, '─', DIM, None);
    }
    text(
        buf,
        x0,
        1,
        w,
        &format!(" Signals ({})", app.items.len()),
        Style::new()
            .fg(TEXT)
            .bg(BAR_BG)
            .add_modifier(Modifier::BOLD),
    );
    let sep_x = x0 + w;
    for y in 1..l.height.saturating_sub(1) {
        put(buf, sep_x, y, if y == 2 { '┼' } else { '│' }, DIM, None);
    }
    if app.items.is_empty() {
        let msg = [
            " (no signals)",
            "",
            " a / Tab : 信号ツリーを開く",
            " :add <pat> : 名前で追加",
        ];
        for (i, m) in msg.iter().enumerate() {
            text(buf, x0, l.rows_y + i as u16, w, m, Style::new().fg(DIM));
        }
        return;
    }
    let focused = app.mode != Mode::Tree && app.mode != Mode::TreeFilter;
    for (k, item) in app
        .items
        .iter()
        .enumerate()
        .skip(app.top)
        .take(app.visible_items())
    {
        let y = l.rows_y + ((k - app.top) * ROW_H) as u16;
        if y + 1 > l.rows_y + l.rows_h {
            break;
        }
        let sel = k == app.sel;
        let row_style = if sel {
            if focused {
                Style::new().fg(Color::Black).bg(Color::Green)
            } else {
                Style::new().fg(TEXT).bg(Color::Indexed(238))
            }
        } else {
            Style::new().fg(TEXT)
        };
        if sel {
            for x in x0..x0 + w {
                put(buf, x, y, ' ', TEXT, row_style.bg);
                put(buf, x, y + 1, ' ', TEXT, Some(SEL_BG));
            }
        }
        let label = app.item_label(item);
        let sig = app.sig(item);
        let tag = if sig.kind == SigKind::Bits && sig.width > 1 && item.bit.is_none() {
            item.radix.short()
        } else if sig.kind == SigKind::Real {
            "r"
        } else {
            ""
        };
        let lw = w.saturating_sub(tag.len() as u16 + 1);
        text(
            buf,
            x0,
            y,
            lw,
            &format!(" {}", truncate(&label, lw as usize - 1)),
            row_style.add_modifier(Modifier::BOLD),
        );
        if !tag.is_empty() {
            text(
                buf,
                x0 + w - tag.len() as u16 - 1,
                y,
                tag.len() as u16,
                tag,
                row_style.fg(if sel && focused { Color::Black } else { DIM }),
            );
        }
        let val = app.value_at_cursor(item);
        let vs = Style::new()
            .fg(VALUE)
            .bg(if sel { SEL_BG } else { Color::Reset });
        text(
            buf,
            x0,
            y + 1,
            w,
            &format!("   {}", truncate(&val, w as usize - 3)),
            vs,
        );
    }
}

fn nice_step(x: f64) -> f64 {
    if x <= 1.0 {
        return 1.0;
    }
    let p = 10f64.powf(x.log10().floor());
    let m = x / p;
    let n = if m <= 1.0 {
        1.0
    } else if m <= 2.0 {
        2.0
    } else if m <= 5.0 {
        5.0
    } else {
        10.0
    };
    n * p
}

fn draw_ruler(buf: &mut Buffer, app: &App) {
    let l = app.layout;
    let (x0, w) = (l.wave_x, l.wave_w);
    for x in x0..x0 + w {
        put(buf, x, 1, ' ', TEXT, Some(BAR_BG));
        put(buf, x, 2, '─', DIM, None);
    }
    let z = app.zoom;
    let vs = app.view_start;
    let ts = app.vcd.timescale_fs;
    // label width estimate decides tick spacing
    let label_w = format::fmt_time(app.vcd.end_time, ts).len().max(6) as f64 + 4.0;
    let step = nice_step(z * label_w);
    let mut t = (vs / step).ceil() * step;
    let end = vs + w as f64 * z;
    let mut last_end: i64 = -1;
    while t < end {
        let col = ((t - vs) / z).floor() as i64;
        if col >= 0 && col < w as i64 {
            put(buf, x0 + col as u16, 2, '┬', DIM, None);
            if col > last_end {
                let lab = format::fmt_time(t as u64, ts);
                let used = text(
                    buf,
                    x0 + col as u16,
                    1,
                    w - col as u16,
                    &lab,
                    Style::new().fg(DIM).bg(BAR_BG),
                );
                last_end = col + used as i64;
            }
        }
        t += step;
    }
    // markers
    for (i, m) in app.markers.iter().enumerate() {
        if let Some(t) = *m {
            let c = app.col_of(t);
            if c >= 0.0 && c < w as f64 {
                put(
                    buf,
                    x0 + c as u16,
                    2,
                    (b'a' + i as u8) as char,
                    Color::White,
                    Some(MARKER_BG),
                );
            }
        }
    }
    let c = app.col_of(app.cursor);
    if c >= 0.0 && c < w as f64 {
        put(buf, x0 + c as u16, 2, '▼', Color::Yellow, Some(CURSOR_BG));
    }
    // end-of-dump indicator
    let ce = app.col_of(app.vcd.end_time) + 1.0;
    if ce >= 0.0 && ce < w as f64 {
        for x in (x0 + ce as u16)..(x0 + w) {
            put(buf, x, 2, '┄', Color::Indexed(238), None);
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum VClass {
    Normal,
    X,
    Z,
}

fn vclass(app: &App, item: &Item, idx: usize) -> VClass {
    let s = app.sig(item);
    match s.kind {
        SigKind::Bits => {
            let b = s.bits(idx);
            if b.iter().all(|&c| c == b'z') {
                VClass::Z
            } else if b.iter().any(|&c| c != b'0' && c != b'1') {
                VClass::X
            } else {
                VClass::Normal
            }
        }
        SigKind::Real => {
            if s.real(idx).is_nan() {
                VClass::X
            } else {
                VClass::Normal
            }
        }
        SigKind::Str => VClass::Normal,
    }
}

fn level(v: u8) -> (char, char, Color) {
    match v {
        b'1' => ('─', ' ', WAVE),
        b'0' => (' ', '─', WAVE),
        b'z' => ('▁', ' ', ZCOL),
        _ => ('▄', '▀', XFILL),
    }
}

fn color_of(v: u8) -> Color {
    match v {
        b'0' | b'1' => WAVE,
        b'z' => ZCOL,
        _ => XCOL,
    }
}

fn scalar_glyph(before: Option<u8>, after: Option<u8>, n: usize) -> (char, char, Color) {
    match (n, before, after) {
        (_, _, None) => (' ', ' ', WAVE),
        (0, _, Some(v)) => level(v),
        (1, None, Some(v)) => level(v),
        (1, Some(b'0'), Some(b'1')) => ('┌', '┘', WAVE),
        (1, Some(b'1'), Some(b'0')) => ('┐', '└', WAVE),
        (1, Some(_), Some(a)) => ('│', '│', color_of(a)),
        (_, _, Some(a)) => ('│', '│', color_of(a)),
    }
}

fn draw_waves(buf: &mut Buffer, app: &App) {
    let l = app.layout;
    let vis = app.visible_items();
    for (k, item) in app.items.iter().enumerate().skip(app.top).take(vis) {
        let y = l.rows_y + ((k - app.top) * ROW_H) as u16;
        let bg = if k == app.sel { Some(SEL_BG) } else { None };
        if let Some(bg) = bg {
            for x in l.wave_x..l.wave_x + l.wave_w {
                set_bg(buf, x, y, bg);
                set_bg(buf, x, y + 1, bg);
            }
        }
        draw_item_wave(buf, app, item, l.wave_x, y, l.wave_w as usize);
    }
    // vertical marker / cursor lines
    let y_end = l.rows_y + (vis.min(app.items.len().saturating_sub(app.top)) * ROW_H) as u16;
    let y_end = y_end.min(l.rows_y + l.rows_h).max(l.rows_y);
    let column = |buf: &mut Buffer, t: u64, bg: Color| {
        let c = app.col_of(t);
        if c >= 0.0 && c < l.wave_w as f64 {
            let x = l.wave_x + c as u16;
            for y in l.rows_y..l.rows_y + l.rows_h {
                if y < y_end {
                    set_bg(buf, x, y, bg);
                } else {
                    put(buf, x, y, '┊', bg, None);
                }
            }
        }
    };
    for m in app.markers.iter().flatten() {
        column(buf, *m, MARKER_BG);
    }
    column(buf, app.cursor, CURSOR_BG);
}

fn draw_item_wave(buf: &mut Buffer, app: &App, item: &Item, x0: u16, y: u16, w: usize) {
    let s = app.sig(item);
    let times = &s.times;
    let col_t = |x: usize| app.col_time(x);
    let mut p0 = times.partition_point(|&t| t < col_t(0));
    let scalar = app.is_scalar(item);
    let end_col = app.col_of(app.vcd.end_time);

    // vector segment tracking: (start_col, end_col_exclusive, value index)
    let mut segs: Vec<(usize, usize, usize)> = Vec::new();
    let mut seg_start = 0usize;
    let mut seg_val = p0.checked_sub(1);
    let mut seg_class = seg_val.map(|i| vclass(app, item, i));

    for x in 0..w {
        if x as f64 > end_col {
            break;
        }
        let te = col_t(x + 1);
        let p1 = p0 + times[p0..].partition_point(|&t| t < te);
        let before = p0.checked_sub(1);
        let after = p1.checked_sub(1);
        let mut changes = p1 - p0;
        let xx = x0 + x as u16;
        if scalar {
            let bv = |i: Option<usize>| i.map(|i| s.bit(i, item.bit));
            if item.bit.is_some() && changes > 0 {
                if changes <= 32 {
                    let mut c = 0;
                    let mut prev = bv(before);
                    for i in p0..p1 {
                        let v = Some(s.bit(i, item.bit));
                        if v != prev {
                            c += 1;
                        }
                        prev = v;
                    }
                    changes = c;
                } else {
                    changes = 2;
                }
            }
            let (top, bot, col) = scalar_glyph(bv(before), bv(after), changes);
            put(buf, xx, y, top, col, None);
            put(buf, xx, y + 1, bot, col, None);
        } else {
            if changes == 0 || (changes == 1 && before.is_none()) {
                if changes == 1 {
                    seg_start = x;
                    seg_val = after;
                    seg_class = after.map(|i| vclass(app, item, i));
                }
                if let Some(cl) = seg_class {
                    let c = match cl {
                        VClass::Normal => BUS,
                        VClass::X => XCOL,
                        VClass::Z => ZCOL,
                    };
                    if cl == VClass::Z {
                        put(buf, xx, y, '▁', c, None);
                        put(buf, xx, y + 1, ' ', c, None);
                    } else {
                        put(buf, xx, y, '─', c, None);
                        put(buf, xx, y + 1, '─', c, None);
                    }
                }
            } else {
                if let Some(v) = seg_val {
                    segs.push((seg_start, x, v));
                }
                let ch = if changes == 1 { '╳' } else { '▒' };
                put(buf, xx, y, ch, BUS, None);
                put(buf, xx, y + 1, ch, BUS, None);
                seg_start = x + 1;
                seg_val = after;
                seg_class = after.map(|i| vclass(app, item, i));
            }
        }
        p0 = p1;
    }
    if !scalar {
        let last = ((end_col + 1.0).max(0.0) as usize).min(w);
        if let Some(v) = seg_val
            && seg_start < last
        {
            segs.push((seg_start, last, v));
        }
        for (a, b, v) in segs {
            let len = b.saturating_sub(a);
            if len < 2 {
                continue;
            }
            let val = app.value_text(item, v);
            let shown = truncate(&val, len);
            let fg = match vclass(app, item, v) {
                VClass::Normal => TEXT,
                VClass::X => XCOL,
                VClass::Z => ZCOL,
            };
            let y_text = if vclass(app, item, v) == VClass::Z {
                y + 1
            } else {
                y
            };
            for (k, ch) in shown.chars().enumerate() {
                if let Some(c) = buf.cell_mut((x0 + (a + k) as u16, y_text)) {
                    c.set_char(ch)
                        .set_style(Style::new().fg(fg).add_modifier(Modifier::BOLD));
                }
            }
        }
    }
}

fn draw_status(buf: &mut Buffer, app: &App) {
    let l = app.layout;
    let y = l.height - 1;
    for x in 0..l.width {
        put(buf, x, y, ' ', TEXT, Some(Color::Reset));
    }
    let (tag, tag_bg) = match app.mode {
        Mode::Normal => (" NORMAL ", Color::Green),
        Mode::Tree => (" TREE ", Color::Blue),
        Mode::TreeFilter => (" FILTER ", Color::Magenta),
        Mode::Command => (" COMMAND ", Color::Yellow),
        Mode::Search => (" SEARCH ", Color::Yellow),
        Mode::Help => (" HELP ", Color::Cyan),
    };
    let used = text(
        buf,
        0,
        y,
        l.width,
        tag,
        Style::new()
            .fg(Color::Black)
            .bg(tag_bg)
            .add_modifier(Modifier::BOLD),
    );
    let x = used + 1;
    let rest = l.width.saturating_sub(x + 12);
    match app.mode {
        Mode::Command | Mode::Search | Mode::TreeFilter => {
            let prefix = match app.mode {
                Mode::Command => ":",
                _ => "/",
            };
            let s = format!("{prefix}{}", app.input);
            let s = truncate_left(&s, rest as usize);
            let n = text(buf, x, y, rest, &s, Style::new().fg(TEXT));
            put(buf, x + n, y, ' ', TEXT, Some(Color::White));
        }
        _ => {
            if let Some((m, err)) = &app.msg {
                let st = if *err {
                    Style::new().fg(Color::White).bg(Color::Red)
                } else {
                    Style::new().fg(TEXT)
                };
                text(buf, x, y, rest, &truncate(m, rest as usize), st);
            } else if app.mode == Mode::Tree {
                text(
                    buf,
                    x,
                    y,
                    rest,
                    "j/k:移動 l:展開/追加 h:畳む Enter:追加 a/A:スコープ追加 /:検索 Tab/Esc:戻る",
                    Style::new().fg(DIM),
                );
            } else if let Some(item) = app.items.get(app.sel) {
                let v = &app.vcd.vars[item.var];
                let info = format!(
                    "{}  ({} {}bit)",
                    v.full_display(&app.vcd.scopes),
                    v.vtype,
                    v.width
                );
                text(
                    buf,
                    x,
                    y,
                    rest,
                    &truncate_left(&info, rest as usize),
                    Style::new().fg(DIM),
                );
            }
        }
    }
    // showcmd area
    let mut sc = String::new();
    if let Some(c) = app.count {
        sc += &c.to_string();
    }
    if let Some(p) = app.pending {
        sc.push(p);
    }
    let pos = if app.items.is_empty() {
        String::new()
    } else {
        format!("{}/{}", app.sel + 1, app.items.len())
    };
    let right = format!("{sc:>5} {pos:>7}");
    let rx = l.width.saturating_sub(right.chars().count() as u16);
    text(buf, rx, y, l.width - rx, &right, Style::new().fg(DIM));
}

pub const HELP: &[(&str, &str)] = &[
    ("", "── カーソル / 表示範囲 ──"),
    ("h l  ← →", "カーソルを 1 列 左/右へ (数字で回数指定: 10l)"),
    ("H L", "表示を半画面 左/右へスクロール"),
    ("Ctrl-b Ctrl-f", "表示を 1 画面 左/右へスクロール"),
    ("w / b", "選択信号の 次/前 の変化点へジャンプ"),
    ("W / B", "表示中の全信号の 次/前 の変化点へジャンプ"),
    ("0 ^ Home", "先頭時刻へ"),
    ("$ End", "最終時刻へ"),
    ("zz", "カーソルを画面中央に"),
    (":<time>", "指定時刻へ移動 (例 :1500, :1.5us)"),
    ("", "── ズーム ──"),
    ("k + =", "拡大 (カーソル位置中心)"),
    ("j - _", "縮小"),
    ("F  zf", "全体表示 (fit)"),
    (":zoom <t>", "1 列あたりの時間を指定 (例 :zoom 10ns)"),
    ("", "── 信号リスト ──"),
    ("J K  ↓ ↑", "選択信号を 下/上 へ"),
    ("gg / G", "先頭 / 末尾の信号へ (5G で 5 番目)"),
    ("Ctrl-d Ctrl-u", "半ページ スクロール"),
    ("Ctrl-e Ctrl-y", "1 行 スクロール"),
    ("Alt-j Alt-k", "選択信号を 下/上 へ移動 (並べ替え)"),
    ("dd x", "信号を削除 (レジスタへ)"),
    ("yy", "信号をヤンク (コピー)"),
    ("p P", "レジスタを 下/上 に貼り付け"),
    ("u  Ctrl-r", "アンドゥ / リドゥ"),
    ("r", "基数を切替 hex→dec→sdec→bin→oct→ascii"),
    (":radix <r>", "基数を指定 (hex dec sdec bin oct ascii)"),
    ("zo zc za", "バスを各ビットに展開 / 畳む / トグル"),
    ("< >", "信号名欄の幅を 狭く/広く"),
    ("", "── 信号の追加 (ツリー) ──"),
    ("a i o Tab", "信号ツリーを開いてフォーカス"),
    ("t", "信号ツリーの表示/非表示"),
    ("  j k", "移動 (gg G Ctrl-d Ctrl-u も可)"),
    ("  l →", "スコープを展開 / 信号を追加"),
    ("  h ←", "スコープを畳む / 親へ"),
    ("  Enter Space", "信号を追加 (スコープは開閉)"),
    ("  a", "スコープ直下の信号を全部追加"),
    (
        "  A",
        "スコープ以下を再帰的に全部追加 (フィルタ中は一覧全部)",
    ),
    ("  /", "インクリメンタル検索 (空白区切りで AND)"),
    ("  Esc Tab", "波形に戻る (フィルタ中は解除)"),
    (":add <pat>", "パスに pat を含む信号をすべて追加 (* で全部)"),
    ("", "── マーカー / 検索 ──"),
    ("m{a-z}", "カーソル位置にマーカーを設定"),
    ("'{a-z}", "マーカーへジャンプ (ヘッダに Δ時間を表示)"),
    (":marks  :delm!", "マーカー一覧 / 全削除"),
    ("/<value>", "選択信号が値 value になる時刻を前方検索"),
    ("n N", "次 / 前 を検索"),
    ("", "── ファイル ──"),
    (":w [file]", "信号リスト・マーカーを保存 (既定 <vcd>.twv)"),
    (":e  R", "VCD を再読み込み (表示信号は維持)"),
    (":q", "終了 (q でも可)"),
    ("Ctrl-l", "画面再描画"),
    ("?  :help", "このヘルプ (j/k でスクロール, 他キーで閉じる)"),
];

fn draw_help(f: &mut Frame, app: &App) {
    let area = f.area();
    let w = area.width.saturating_sub(4).min(80);
    let h = area.height.saturating_sub(2);
    let r = Rect {
        x: (area.width - w) / 2,
        y: (area.height - h) / 2,
        width: w,
        height: h,
    };
    let lines: Vec<Line> = HELP
        .iter()
        .map(|(k, d)| {
            if k.is_empty() {
                Line::from(Span::styled(
                    *d,
                    Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD),
                ))
            } else {
                Line::from(vec![
                    Span::styled(
                        format!(" {k:<16}"),
                        Style::new().fg(Color::Green).add_modifier(Modifier::BOLD),
                    ),
                    Span::raw(*d),
                ])
            }
        })
        .collect();
    let max_scroll = (lines.len() as u16).saturating_sub(h.saturating_sub(2));
    let p = Paragraph::new(lines)
        .wrap(Wrap { trim: false })
        .scroll((app.help_scroll.min(max_scroll), 0))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .title(" terminalwave help ")
                .title_bottom(" j/k: scroll   other key: close "),
        );
    f.render_widget(Clear, r);
    f.render_widget(p, r);
}
