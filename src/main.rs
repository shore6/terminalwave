mod app;
mod format;
mod ui;
mod vcd;

use std::io;
use std::path::PathBuf;
use std::process::ExitCode;

use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{self, Event, KeyEventKind};

use app::{App, Mode};

const USAGE: &str = "\
terminalwave - terminal VCD waveform viewer

USAGE:
    terminalwave [OPTIONS] <FILE.vcd>

OPTIONS:
    -s, --save <FILE>   load a layout file (signal list / markers) saved with :w
                        (default: <FILE>.twv next to the VCD, if it exists)
    -l, --list          print all signals in the VCD and exit
    -h, --help          print this help
    -V, --version       print version

Press ? inside the viewer for key bindings.";

fn main() -> ExitCode {
    let mut file: Option<PathBuf> = None;
    let mut save: Option<PathBuf> = None;
    let mut list = false;
    let mut args = std::env::args_os().skip(1);
    while let Some(a) = args.next() {
        match a.to_str() {
            Some("-h") | Some("--help") => {
                println!("{USAGE}");
                return ExitCode::SUCCESS;
            }
            Some("-V") | Some("--version") => {
                println!("terminalwave {}", env!("CARGO_PKG_VERSION"));
                return ExitCode::SUCCESS;
            }
            Some("-l") | Some("--list") => list = true,
            Some("-s") | Some("--save") => match args.next() {
                Some(p) => save = Some(PathBuf::from(p)),
                None => {
                    eprintln!("error: {} requires a file argument", a.to_string_lossy());
                    return ExitCode::from(2);
                }
            },
            Some(s) if s.starts_with('-') && s.len() > 1 => {
                eprintln!("error: unknown option {s}\n\n{USAGE}");
                return ExitCode::from(2);
            }
            _ => file = Some(PathBuf::from(a)),
        }
    }
    let Some(file) = file else {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    };

    let data = match std::fs::read(&file) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("error: {}: {e}", file.display());
            return ExitCode::FAILURE;
        }
    };
    let vcd = match vcd::parse(&data) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("error: {}: {e}", file.display());
            return ExitCode::FAILURE;
        }
    };
    drop(data);

    if list {
        for v in &vcd.vars {
            println!(
                "{:<10} {:>4}  {}",
                v.vtype,
                v.width,
                v.full_display(&vcd.scopes)
            );
        }
        return ExitCode::SUCCESS;
    }

    let mut app = App::new(vcd, file);
    let mut terminal = ratatui::init();
    let res = run(&mut terminal, &mut app, save);
    ratatui::restore();
    match res {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(terminal: &mut DefaultTerminal, app: &mut App, save: Option<PathBuf>) -> io::Result<()> {
    let save = save.or_else(|| Some(app.default_save_path()).filter(|p| p.exists()));
    app.tree_visible = save.is_none();
    let size = terminal.size()?;
    app.update_layout(size.width, size.height);
    app.fit();
    match save {
        Some(p) => {
            if let Err(e) = app.load_layout(&p) {
                app.error(e);
            }
        }
        None => {
            // Nothing to show yet: start in the signal tree.
            app.mode = Mode::Tree;
            app.info("信号を選んで Enter で追加、Tab で波形へ。? でヘルプ");
        }
    }
    loop {
        terminal.draw(|f| ui::draw(f, app))?;
        if app.quit {
            return Ok(());
        }
        match event::read()? {
            Event::Key(k) if k.kind != KeyEventKind::Release => app.on_key(k),
            Event::Resize(w, h) => app.update_layout(w, h),
            _ => {}
        }
        if app.clear_screen {
            app.clear_screen = false;
            terminal.clear()?;
        }
    }
}
