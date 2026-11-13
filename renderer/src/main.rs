//! typeset-render: renders the LaTeX in Claude Code replies to PNG pictures
//! that fill whole terminal cells. The mod runs it once per batch.
//!
//!     typeset-render            JSON request on stdin, JSON answer on stdout
//!     typeset-render --check    JSON health (an optional {"options": {...}} on stdin)
//!     typeset-render --detect   JSON {"cell", "fg"} the 'auto' options resolve to now
//!     typeset-render --version
//!
//! See protocol.rs for the request and the answer.

mod detect;
mod fit;
mod fonts;
mod normalize;
mod protocol;
mod ratex;

use std::io::Read;
use std::panic::{AssertUnwindSafe, catch_unwind};

use serde_json::{Value, json};

use protocol::{Answer, ItemResult, MAX_LATEX, Options};

const VERSION: &str = concat!("typeset-render ", env!("CARGO_PKG_VERSION"));

fn read_stdin() -> Value {
    let mut input = String::new();
    if std::io::stdin().read_to_string(&mut input).is_err() || input.trim().is_empty() {
        return Value::Null;
    }
    serde_json::from_str(&input).unwrap_or(Value::Null)
}

/// One formula, start to end; a panic in the engine fails this item alone.
fn render_item(
    latex: &str,
    display: bool,
    cfg: &Options,
    macros: &str,
    fg: [f32; 3],
    cell: protocol::Cell,
) -> ItemResult {
    if latex.chars().count() > MAX_LATEX {
        return ItemResult::failure(format!("formula longer than {MAX_LATEX} characters"));
    }
    let latex = normalize::normalize(latex, display);
    if latex.trim().is_empty() {
        return ItemResult::failure("empty formula");
    }
    let outcome = catch_unwind(AssertUnwindSafe(|| {
        match ratex::typeset(&latex, display, fg, cfg.pt_px, macros) {
            Ok(ink) => fit::fit(&ink, display, cfg, cell),
            Err(error) => ItemResult::failure(error),
        }
    }));
    outcome.unwrap_or_else(|_| ItemResult::failure("renderer failed on this formula"))
}

fn render(request: &Value) -> Value {
    if !request.is_object() {
        return json!({ "error": "bad request: not a JSON object", "results": [] });
    }
    let cfg = protocol::parse_options(request.get("options").unwrap_or(&Value::Null));
    let items = protocol::parse_items(request);
    let cell = detect::detect_cell(&cfg.cell);
    let fg = detect::detect_fg(&cfg.fg);
    let colour = detect::rgb(&fg);
    if items.iter().any(|item| fonts::needs_cjk(&item.latex)) || fonts::needs_cjk(&cfg.macros) {
        fonts::install(&cfg.cjk_font);
    }
    let macros = ratex::user_macros(&cfg.macros);
    let results = items
        .iter()
        .map(|item| render_item(&item.latex, item.display, &cfg, &macros, colour, cell))
        .collect();

    serde_json::to_value(Answer { cell, fg, results }).unwrap_or(Value::Null)
}

fn check(request: &Value) -> Value {
    let cfg = protocol::parse_options(request.get("options").unwrap_or(&Value::Null));
    let cell = detect::detect_cell(&cfg.cell);
    let fg = detect::detect_fg(&cfg.fg);
    let font = fonts::install(&cfg.cjk_font);
    let mut problems: Vec<String> = Vec::new();
    if !cfg.cjk_font.is_empty() && font.as_deref() != Some(cfg.cjk_font.as_str()) {
        problems.push(format!("cjkFont {} is not a font file", cfg.cjk_font));
    }
    let probe = render_item(r"\frac{a}{b}", false, &cfg, "", detect::rgb(&fg), cell);
    let ok = matches!(probe, ItemResult::Picture { .. });
    if let ItemResult::Failure { error } = &probe {
        problems.push(format!("test formula failed: {error}"));
    }
    if ok && !cfg.macros.is_empty() {
        let macros = ratex::user_macros(&cfg.macros);
        if let ItemResult::Failure { error } = render_item("x", false, &cfg, &macros, detect::rgb(&fg), cell) {
            problems.push(format!("macros: {error}"));
        }
    }
    json!({
        "ok": ok,
        "version": VERSION,
        "engine": "ratex",
        "cjkFont": font,
        "cell": cell,
        "fg": fg,
        "problems": problems,
    })
}

fn main() {
    let mode = std::env::args().nth(1).unwrap_or_default();
    let answer = match mode.as_str() {
        "" => render(&read_stdin()),
        "--check" => check(&read_stdin()),
        "--detect" => {
            let cell = detect::detect_cell("auto");
            json!({ "cell": cell, "fg": detect::detect_fg("auto") })
        }
        "--version" => {
            println!("{VERSION}");
            return;
        }
        other => {
            eprintln!("typeset-render: unknown argument {other}");
            std::process::exit(2);
        }
    };
    println!("{answer}");
}
