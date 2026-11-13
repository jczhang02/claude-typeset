//! The program as the mod runs it: JSON in, JSON out.
//!
//! tests/fixtures/geometry.json holds what the Python backend (v3) drew for
//! every formula of the corpus (bin/corpus at v3.0.0) at cell 19x42 and
//! minShrink 0.7: columns, rows and the text row. The new engine must place
//! formulas on the same cells, give or take one, with the baseline on the
//! same row.

use std::io::Write;
use std::process::{Command, Stdio};

use serde_json::{Value, json};

fn run(args: &[&str], stdin: &str) -> Value {
    let mut child = Command::new(env!("CARGO_BIN_EXE_typeset-render"))
        .args(args)
        .env("TMUX", "") // never ask a tmux server the test happens to run under
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("the program starts");
    child.stdin.take().unwrap().write_all(stdin.as_bytes()).unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success(), "exit status {:?}", out.status);
    serde_json::from_slice(&out.stdout).expect("JSON on stdout")
}

fn render(items: Value, options: Value) -> Vec<Value> {
    let answer = run(&[], &json!({ "items": items, "options": options }).to_string());
    answer["results"].as_array().cloned().unwrap()
}

fn options() -> Value {
    json!({ "fg": "#4c4f69", "cell": "19x42", "minShrink": 0.7 })
}

#[test]
fn answers_with_the_cell_and_colour_it_drew_for() {
    let answer = run(
        &[],
        &json!({ "items": [{ "latex": "x", "display": false }], "options": options() }).to_string(),
    );
    assert_eq!(answer["cell"], json!({ "width": 19, "height": 42 }));
    assert_eq!(answer["fg"], "#4c4f69");
    let picture = &answer["results"][0];
    assert_eq!(
        (picture["rows"].as_u64(), picture["textRow"].as_u64()),
        (Some(1), Some(0))
    );
    assert!(picture["columns"].as_u64().unwrap() <= 2);
    assert!(
        picture["png"].as_str().unwrap().starts_with("iVBORw0KGgo"),
        "a base64 PNG"
    );
}

#[test]
fn a_bad_formula_fails_alone() {
    let results = render(
        json!([{ "latex": r"\frac{a}", "display": false }, { "latex": "y", "display": false }]),
        options(),
    );
    assert!(results[0]["error"].is_string());
    assert!(results[1]["png"].is_string());
}

#[test]
fn refuses_empty_and_overlong_formulas() {
    let long = "x".repeat(4001);
    let results = render(
        json!([{ "latex": "  % only a comment", "display": false }, { "latex": long, "display": false }]),
        options(),
    );
    assert_eq!(results[0]["error"], "empty formula");
    assert_eq!(results[1]["error"], "formula longer than 4000 characters");
}

#[test]
fn the_users_macros_are_defined() {
    // \R is built into the engine: the user's definition still wins, and
    // \DeclareMathOperator works though the engine lacks it.
    let macros = "\\newcommand{\\R}{\\mathbb{R}}\n\\newcommand\\Id{I}\n\\DeclareMathOperator{\\tr}{tr}\n\\DeclareMathOperator*{\\argmax}{arg\\,max}";
    let options = json!({ "fg": "#4c4f69", "cell": "19x42", "macros": macros });
    let results = render(
        json!([{ "latex": r"\R^n \Id \tr A \argmax_x f", "display": false }]),
        options.clone(),
    );
    assert!(results[0]["png"].is_string(), "{}", results[0]);
    let health = run(&["--check"], &json!({ "options": options }).to_string());
    assert!(health["problems"].as_array().unwrap().is_empty(), "{health}");
}

#[test]
fn a_runaway_macro_stops() {
    let results = render(json!([{ "latex": r"\def\a{\a}\a", "display": false }]), options());
    assert!(results[0]["error"].is_string());
}

#[test]
fn display_formulas_are_drawn_whole() {
    // minShrink lets an inline fraction shrink into one row; a display one keeps its size.
    let results = render(
        json!([{ "latex": r"\frac{a}{b}", "display": false }, { "latex": r"\frac{a}{b}", "display": true }]),
        options(),
    );
    assert!(results[0]["rows"].as_u64() <= results[1]["rows"].as_u64());
    assert!(results[1]["rows"].as_u64().unwrap() >= 2);
}

#[test]
fn a_narrow_space_shrinks_a_long_formula() {
    let latex = r"a_1 + a_2 + a_3 + a_4 + a_5 + a_6 + a_7 + a_8 + a_9 + a_{10}";
    let wide = render(json!([{ "latex": latex, "display": false }]), options());
    let narrow = render(
        json!([{ "latex": latex, "display": false }]),
        json!({ "fg": "#4c4f69", "cell": "19x42", "maxColumns": 12 }),
    );
    assert!(wide[0]["columns"].as_u64().unwrap() > 12);
    assert!(narrow[0]["columns"].as_u64().unwrap() <= 12, "{}", narrow[0]);
}

#[test]
fn check_reports_a_working_engine() {
    let health = run(&["--check"], &json!({ "options": options() }).to_string());
    assert_eq!(health["ok"], true, "{health}");
    assert_eq!(health["engine"], "ratex");
    assert!(health["problems"].as_array().unwrap().is_empty());
}

#[test]
fn detect_names_a_cell_and_a_colour() {
    let found = run(&["--detect"], "");
    assert!(found["cell"]["width"].as_u64().unwrap() >= 2);
    assert!(found["fg"].as_str().unwrap().starts_with('#'));
}

#[test]
fn places_the_corpus_on_the_cells_the_old_backend_did() {
    let fixture: Value = serde_json::from_str(include_str!("fixtures/geometry.json")).unwrap();
    let items = fixture["items"].as_array().unwrap();
    let request: Vec<Value> = items
        .iter()
        .map(|it| json!({ "latex": it["latex"], "display": it["display"] }))
        .collect();
    let results = render(Value::Array(request), options());
    let (mut compared, mut close, mut same_row) = (0, 0, 0);
    let mut far = Vec::new();
    for (old, new) in items.iter().zip(&results) {
        if old.get("error").is_some() || new.get("error").is_some() {
            continue;
        }
        compared += 1;
        let d = |k: &str| (new[k].as_i64().unwrap() - old[k].as_i64().unwrap()).abs();
        if d("columns") <= 1 && d("rows") <= 1 {
            close += 1;
        } else {
            far.push(old["latex"].as_str().unwrap().chars().take(50).collect::<String>());
        }
        if d("textRow") == 0 {
            same_row += 1;
        }
    }
    let close_share = f64::from(close) / f64::from(compared);
    let row_share = f64::from(same_row) / f64::from(compared);
    eprintln!("{compared} compared, {close} within one cell, {same_row} on the same text row");
    assert!(compared > 1400, "only {compared} formulas drawn by both");
    assert!(close_share >= 0.97, "{close_share:.3} within one cell; far: {far:?}");
    assert!(row_share >= 0.99, "{row_share:.3} on the same text row");
}
