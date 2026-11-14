//! typeset-render: renders the LaTeX in Claude Code replies to PNG pictures
//! that fill whole terminal cells, and mermaid trees to lines of text. The
//! mod runs it once per batch.
//!
//!     typeset-render            JSON request on stdin, JSON answer on stdout
//!     typeset-render --check    JSON health (an optional {"options": {...}} on stdin)
//!     typeset-render --detect   JSON {"cell", "fg"} the 'auto' options resolve to now
//!     typeset-render --version
//!
//! See protocol.rs for the request and the answer.

mod cache;
mod detect;
mod diagram;
mod fit;
mod fonts;
mod normalize;
mod protocol;
mod ratex;
mod sandbox;
mod tex;

use std::io::Read;
use std::panic::{AssertUnwindSafe, catch_unwind};

use serde_json::{Value, json};

use protocol::{Answer, ItemResult, Kind, MAX_LATEX, MAX_TIKZ, Options};

pub const VERSION: &str = concat!("typeset-render ", env!("CARGO_PKG_VERSION"));

fn read_stdin() -> Value {
    let mut input = String::new();
    if std::io::stdin().read_to_string(&mut input).is_err() || input.trim().is_empty() {
        return Value::Null;
    }
    serde_json::from_str(&input).unwrap_or(Value::Null)
}

/// One formula through RaTeX: its picture, or (Err) why RaTeX could not
/// typeset it, which the TeX fallback may still. A panic in the engine
/// fails this formula alone.
fn ratex_item(
    latex: &str,
    display: bool,
    cfg: &Options,
    macros: &str,
    fg: [f32; 3],
    cell: protocol::Cell,
) -> Result<ItemResult, String> {
    if let Some(c) = fonts::missing(latex) {
        return Err(format!("no CJK font found that has {c} (see the cjkFont option)"));
    }
    let outcome = catch_unwind(AssertUnwindSafe(|| {
        ratex::typeset(latex, display, fg, cfg.pt_px, macros)
    }));
    match outcome {
        Ok(Ok(ink)) => Ok(fit::fit(&ink, display, cfg, cell).by("ratex")),
        Ok(Err(error)) => Err(error),
        Err(_) => Err("renderer failed on this formula".into()),
    }
}

/// What becomes of one item: a result now, or a job for TeX.
enum Plan {
    Done(ItemResult),
    Tex(tex::Job),
}

fn plan_item(
    item: &protocol::Item,
    cfg: &Options,
    macros: &str,
    fg: [f32; 3],
    cell: protocol::Cell,
    tex: &tex::Tex,
) -> Plan {
    if item.kind == Kind::Diagram {
        return Plan::Done(match diagram::draw(&item.latex, cfg.max_columns as usize) {
            Ok(drawing) => ItemResult::Drawing {
                lines: drawing.lines,
                columns: drawing.columns as u32,
            },
            Err(error) => ItemResult::failure(error),
        });
    }
    if item.kind == Kind::Tikz {
        if item.latex.chars().count() > MAX_TIKZ {
            return Plan::Done(ItemResult::failure(format!(
                "tikz block longer than {MAX_TIKZ} characters"
            )));
        }
        return Plan::Tex(tex::Job::Tikz {
            source: item.latex.clone(),
        });
    }
    if item.latex.chars().count() > MAX_LATEX {
        return Plan::Done(ItemResult::failure(format!(
            "formula longer than {MAX_LATEX} characters"
        )));
    }
    let latex = normalize::normalize(&item.latex, item.display);
    if latex.trim().is_empty() {
        return Plan::Done(ItemResult::failure("empty formula"));
    }
    if tex::is_diagram(&latex) {
        // RaTeX cannot draw tikz-cd: straight to TeX.
        return Plan::Tex(tex::Job::Diagram {
            latex,
            display: item.display,
        });
    }
    match ratex_item(&latex, item.display, cfg, macros, fg, cell) {
        Ok(result) => Plan::Done(result),
        Err(error) if tex.falls_back() => Plan::Tex(tex::Job::Formula {
            latex,
            display: item.display,
            ratex_error: error,
        }),
        Err(error) => Plan::Done(ItemResult::failure(error)),
    }
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
    // A diagram is text: only pictures need the CJK font.
    let pictured = |item: &&protocol::Item| item.kind != Kind::Diagram;
    if items.iter().filter(pictured).any(|item| fonts::needs_cjk(&item.latex)) || fonts::needs_cjk(&cfg.macros) {
        fonts::install(&cfg.cjk_font);
    }
    let (macros, _) = ratex::user_macros(&cfg.macros);
    let tex = tex::Tex::new(&cfg, cell, &fg);
    let plans: Vec<Plan> = items
        .iter()
        .map(|item| plan_item(item, &cfg, &macros, colour, cell, &tex))
        .collect();
    let jobs: Vec<tex::Job> = plans
        .iter()
        .filter_map(|plan| match plan {
            Plan::Tex(job) => Some(job.clone()),
            Plan::Done(_) => None,
        })
        .collect();
    let mut typeset = tex.run_all(&jobs).into_iter();
    let results = plans
        .into_iter()
        .map(|plan| match plan {
            Plan::Done(result) => result,
            Plan::Tex(_) => typeset
                .next()
                .unwrap_or_else(|| ItemResult::failure("renderer failed on this formula")),
        })
        .collect();

    serde_json::to_value(Answer { cell, fg, results }).unwrap_or(Value::Null)
}

/// One formula through RaTeX alone, for the checks.
fn probe(latex: &str, cfg: &Options, macros: &str, fg: [f32; 3], cell: protocol::Cell) -> ItemResult {
    ratex_item(&normalize::normalize(latex, false), false, cfg, macros, fg, cell).unwrap_or_else(ItemResult::failure)
}

fn check(request: &Value) -> Value {
    let cfg = protocol::parse_options(request.get("options").unwrap_or(&Value::Null));
    let cell = detect::detect_cell(&cfg.cell);
    let fg = detect::detect_fg(&cfg.fg);
    let font = fonts::install(&cfg.cjk_font);
    let mut problems: Vec<String> = Vec::new();
    if !cfg.cjk_font.is_empty() && font.as_deref() != Some(cfg.cjk_font.as_str()) {
        problems.push(format!("cjkFont {} is not a font file with CJK glyphs", cfg.cjk_font));
    }
    let test = probe(r"\frac{a}{b}", &cfg, "", detect::rgb(&fg), cell);
    let ok = matches!(test, ItemResult::Picture { .. });
    if let ItemResult::Failure { error } = &test {
        problems.push(format!("test formula failed: {error}"));
    }
    if ok && !cfg.macros.is_empty() {
        let (macros, refused) = ratex::user_macros(&cfg.macros);
        problems.extend(refused.iter().map(|why| format!("macros: {why}")));
        if let ItemResult::Failure { error } = probe("x", &cfg, &macros, detect::rgb(&fg), cell) {
            problems.push(format!("macros: {error}"));
        }
    }
    // TeX's problems are notes: formulas draw without it.
    let (tex, notes) = tex::health(&cfg, cell, &fg);
    problems.extend(notes);
    json!({
        "ok": ok,
        "version": VERSION,
        "engine": "ratex",
        "texFallback": cfg.tex_fallback,
        "tex": tex,
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
