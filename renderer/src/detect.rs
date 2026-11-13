//! What the 'auto' options resolve to: the terminal's cell size in pixels and
//! the text colour of the desktop's light or dark scheme.

use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::protocol::Cell;

pub const DEFAULT_CELL: Cell = Cell { width: 19, height: 42 };
pub const DARK_FG: &str = "#cdd6f4"; // catppuccin mocha text
pub const LIGHT_FG: &str = "#4c4f69"; // catppuccin latte text
const HELPER_TIMEOUT: Duration = Duration::from_secs(1);

/// A small helper program's stdout, or "" when it fails or takes too long.
pub fn quiet_run(argv: &[&str], timeout: Duration) -> String {
    let Some((program, args)) = argv.split_first() else {
        return String::new();
    };
    let Ok(mut child) = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    else {
        return String::new();
    };
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                if !status.success() {
                    return String::new();
                }
                break;
            }
            Ok(None) if start.elapsed() < timeout => std::thread::sleep(Duration::from_millis(5)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return String::new();
            }
        }
    }
    let mut out = String::new();
    if let Some(mut stdout) = child.stdout.take() {
        use std::io::Read;
        let _ = stdout.read_to_string(&mut out);
    }
    out
}

pub fn parse_cell(spec: &str) -> Option<Cell> {
    let (w, h) = spec.trim().split_once('x')?;
    let (w, h): (u32, u32) = (w.parse().ok()?, h.parse().ok()?);
    ((2..=400).contains(&w) && (4..=800).contains(&h)).then_some(Cell { width: w, height: h })
}

/// Cell size from the pixel size the terminal reports for /dev/tty.
fn tty_cell() -> Option<Cell> {
    use std::os::fd::AsRawFd;
    let tty = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags_nonblocking()
        .open("/dev/tty")
        .ok()?;
    let mut size: libc::winsize = unsafe { std::mem::zeroed() };
    // SAFETY: TIOCGWINSZ writes one winsize into `size`, which outlives the call.
    let rc = unsafe { libc::ioctl(tty.as_raw_fd(), libc::TIOCGWINSZ, &mut size) };
    if rc != 0 || size.ws_row == 0 || size.ws_col == 0 || size.ws_xpixel == 0 || size.ws_ypixel == 0 {
        return None;
    }
    parse_cell(&format!(
        "{}x{}",
        size.ws_xpixel / size.ws_col,
        size.ws_ypixel / size.ws_row
    ))
}

trait NonBlocking {
    fn custom_flags_nonblocking(&mut self) -> &mut Self;
}

impl NonBlocking for std::fs::OpenOptions {
    fn custom_flags_nonblocking(&mut self) -> &mut Self {
        use std::os::unix::fs::OpenOptionsExt;
        self.custom_flags(libc::O_NOCTTY | libc::O_NONBLOCK)
    }
}

/// Cell size in pixels: the option, then tmux, then the tty, then 19x42.
pub fn detect_cell(spec: &str) -> Cell {
    if spec != "auto"
        && let Some(cell) = parse_cell(spec) {
            return cell;
        }
    if std::env::var_os("TMUX").is_some() {
        let out = quiet_run(
            &["tmux", "display", "-p", "#{client_cell_width}x#{client_cell_height}"],
            HELPER_TIMEOUT,
        );
        if let Some(cell) = parse_cell(&out) {
            return cell;
        }
    }
    tty_cell().unwrap_or(DEFAULT_CELL)
}

pub fn is_hex_colour(spec: &str) -> bool {
    spec.len() == 7 && spec.starts_with('#') && spec[1..].bytes().all(|b| b.is_ascii_hexdigit())
}

/// Text colour: the option, else from the desktop's light or dark scheme.
pub fn detect_fg(spec: &str) -> String {
    if is_hex_colour(spec) {
        return spec.to_ascii_lowercase();
    }
    let portal = quiet_run(
        &[
            "gdbus",
            "call",
            "--session",
            "--timeout",
            "1",
            "--dest",
            "org.freedesktop.portal.Desktop",
            "--object-path",
            "/org/freedesktop/portal/desktop",
            "--method",
            "org.freedesktop.portal.Settings.ReadOne",
            "org.freedesktop.appearance",
            "color-scheme",
        ],
        HELPER_TIMEOUT,
    );
    if let Some(at) = portal.find("uint32 ") {
        // 0 no preference (light), 1 dark, 2 light
        if let Some(digit) = portal[at + 7..].chars().next().filter(char::is_ascii_digit) {
            return if digit == '1' { DARK_FG } else { LIGHT_FG }.to_string();
        }
    }
    let scheme = quiet_run(
        &["gsettings", "get", "org.gnome.desktop.interface", "color-scheme"],
        HELPER_TIMEOUT,
    );
    if !scheme.is_empty() {
        return if scheme.contains("dark") { DARK_FG } else { LIGHT_FG }.to_string();
    }
    DARK_FG.to_string() // no desktop to ask: most terminals are dark
}

/// "#rrggbb" as 0..1 floats.
pub fn rgb(fg: &str) -> [f32; 3] {
    let at = |i: usize| u8::from_str_radix(&fg[i..i + 2], 16).unwrap_or(0) as f32 / 255.0;
    if is_hex_colour(fg) {
        [at(1), at(3), at(5)]
    } else {
        [0.8, 0.84, 0.96]
    }
}
