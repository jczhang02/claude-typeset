//! The program as the mod runs it: JSON in, JSON out.
//!
//! tests/fixtures/geometry.json holds what the Python backend (v3) drew for
//! every formula of the corpus (bin/corpus at v3.0.0) at cell 19x42 and
//! minShrink 0.7: columns, rows and the text row. The new engine must place
//! formulas on the same cells, give or take one, with the baseline on the
//! same row.
//!
//! The TeX fallback's tests need TeX Live on PATH (latex and dvipng; dvips,
//! Ghostscript and tikz-cd for pictures); without it they print a note and
//! pass. Each run gets a cache folder of its own, so no test sees another's
//! TeX pictures and none reaches ~/.cache.

use std::io::Write;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

/// A cache folder no other run shares.
fn fresh_cache() -> std::path::PathBuf {
    static COUNT: AtomicU32 = AtomicU32::new(0);
    std::env::temp_dir().join(format!(
        "typeset-test-cache-{}-{}",
        std::process::id(),
        COUNT.fetch_add(1, Ordering::Relaxed)
    ))
}

fn run(args: &[&str], stdin: &str) -> Value {
    let cache = fresh_cache();
    let answer = run_with_cache(args, stdin, &cache);
    let _ = std::fs::remove_dir_all(&cache);
    answer
}

fn run_with_cache(args: &[&str], stdin: &str, cache: &std::path::Path) -> Value {
    let mut child = Command::new(env!("CARGO_BIN_EXE_typeset-render"))
        .args(args)
        .env("TMUX", "") // never ask a tmux server the test happens to run under
        .env("XDG_CACHE_HOME", cache)
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

fn on_path(program: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|path| std::env::split_paths(&path).any(|dir| dir.join(program).is_file()))
}

/// Whether the TeX fallback can run here; prints a note when it cannot.
fn tex_here(test: &str) -> bool {
    let found = on_path("latex") && on_path("dvipng");
    if !found {
        eprintln!("note: {test} skipped, latex or dvipng is not on PATH");
    }
    found
}

/// Whether TeX pictures (tikz-cd, ```tikz blocks) can be drawn here.
fn pictures_here(test: &str) -> bool {
    let found = tex_here(test) && on_path("dvips") && on_path("gs") && on_path("kpsewhich") && {
        let out = Command::new("kpsewhich").arg("tikz-cd.sty").output();
        out.is_ok_and(|o| o.status.success())
    };
    if !found {
        eprintln!("note: {test} skipped, dvips, Ghostscript or tikz-cd is missing");
    }
    found
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
fn a_starred_newcommand_is_read() {
    let options = json!({ "fg": "#4c4f69", "cell": "19x42", "texFallback": false, "macros": "\\newcommand*{\\pp}{p}" });
    let results = render(json!([{ "latex": r"\pp + y", "display": false }]), options);
    assert_eq!(results[0]["engine"], "ratex", "{}", results[0]);
}

#[test]
fn a_runaway_macro_stops() {
    let results = render(
        json!([{ "latex": r"\def\a{\a}\a", "display": false }]),
        json!({ "fg": "#4c4f69", "cell": "19x42", "texFallback": false }),
    );
    assert!(results[0]["error"].is_string());
}

#[test]
fn a_package_formula_falls_back_to_tex() {
    let items = json!([
        { "latex": r"\SI{9.81}{\meter\per\second\squared}", "display": false },
        { "latex": r"\dv{f}{x} + \qty(\frac{a}{b})", "display": true },
        { "latex": "x^2", "display": false },
        // Unicode RaTeX takes, in math and in \text, beside a package command.
        { "latex": "\\SI{3}{\\metre} × α ≤ x² \\text{at 25°C}", "display": false },
    ]);
    let off = render(
        items.clone(),
        json!({ "fg": "#4c4f69", "cell": "19x42", "texFallback": false }),
    );
    assert!(off[0]["error"].as_str().unwrap().contains(r"\SI"), "{}", off[0]);
    if !tex_here("a_package_formula_falls_back_to_tex") {
        return;
    }
    let on = render(items, options());
    assert_eq!(on[0]["engine"], "tex", "{}", on[0]);
    assert_eq!(on[1]["engine"], "tex", "{}", on[1]);
    assert_eq!(on[2]["engine"], "ratex");
    assert_eq!(on[3]["engine"], "tex", "{}", on[3]);
    // On one text row, as the old TeX backend drew it (8 columns, 1 row).
    assert_eq!((on[0]["rows"].as_u64(), on[0]["textRow"].as_u64()), (Some(1), Some(0)));
    assert!((7..=9).contains(&on[0]["columns"].as_u64().unwrap()), "{}", on[0]);
}

#[test]
fn ink_beside_the_box_is_kept() {
    // \mathclap's ink stands out of a box of no width: the old backend drew
    // these 7 and 6 columns wide.
    let results = render(
        json!([
            { "latex": r"\displaystyle\sum_{\mathclap{1\le i\le j\le n}} x_{ij}", "display": false },
            { "latex": r"\mathclap{\sum} \quad \mathllap{x} \mathrlap{y} \quad \clap{z} \llap{w} \rlap{v}", "display": false },
        ]),
        options(),
    );
    assert!(results[0]["columns"].as_u64().unwrap() >= 7, "{}", results[0]);
    assert!(results[1]["columns"].as_u64().unwrap() >= 6, "{}", results[1]);
}

#[test]
fn a_mirrored_box_is_drawn_as_its_content() {
    if !tex_here("a_mirrored_box_is_drawn_as_its_content") {
        return;
    }
    // dvipng cannot mirror, scale or rotate: the content is drawn instead
    // of an empty page or the options as text.
    let results = render(
        json!([
            { "latex": r"\reflectbox{KaTeX}", "display": false },
            { "latex": r"\reflectbox{$x^2$}", "display": false },
            { "latex": r"\ndownarrow \nuparrow", "display": false },
        ]),
        options(),
    );
    assert_eq!(results[0]["engine"], "tex", "{}", results[0]);
    assert!(results[0]["columns"].as_u64().unwrap() >= 5, "{}", results[0]);
    assert!(results[1]["columns"].as_u64().unwrap() >= 2, "{}", results[1]);
    assert!(results[2]["columns"].as_u64().unwrap() <= 6, "{}", results[2]);
}

#[test]
fn tex_cannot_read_files_outside_its_folder() {
    if !tex_here("tex_cannot_read_files_outside_its_folder") {
        return;
    }
    // Were the file read, its first line and a 200 pt rule would draw: far
    // more than the two columns of a lone x.
    let read = r"\newread\r \openin\r=/etc/hostname \ifeof\r x\else \read\r to\l \text{\l}\rule{200pt}{10pt}\fi";
    let results = render(
        json!([
            { "latex": read, "display": false },
            { "latex": r"\input{/etc/hostname}", "display": false },
            { "latex": r"\input{../../../../../../etc/hostname}", "display": false },
        ]),
        options(),
    );
    assert!(
        results[0]["error"].is_string() || results[0]["columns"].as_u64() <= Some(2),
        "{}",
        results[0]
    );
    assert!(results[1]["error"].is_string(), "{}", results[1]);
    assert!(results[2]["error"].is_string(), "{}", results[2]);
}

#[test]
fn tex_in_an_endless_loop_is_stopped_by_the_time_limit() {
    if !tex_here("tex_in_an_endless_loop_is_stopped_by_the_time_limit") {
        return;
    }
    let started = Instant::now();
    let cache = fresh_cache();
    let request = json!({
        "items": [
            { "latex": r"\def\a{\a}\a", "display": false },
            { "latex": r"\nosuchcommand", "display": false },
        ],
        "options": options(),
    });
    let results = run_with_cache(&[], &request.to_string(), &cache)["results"].clone();
    let kept = std::fs::read_dir(cache.join("typeset").join("tex")).map_or(0, |dir| dir.count());
    let _ = std::fs::remove_dir_all(&cache);
    let error = results[0]["error"].as_str().unwrap();
    assert!(error.contains("TeX took longer than"), "{error}");
    assert!(started.elapsed() < Duration::from_secs(15), "{:?}", started.elapsed());
    // TeX's own error is kept on disk; the time limit, which a busy machine
    // may hit too, is not.
    assert!(
        results[1]["error"].as_str().unwrap().contains("TeX: !"),
        "{}",
        results[1]
    );
    assert_eq!(kept, 1);
}

#[test]
fn a_tikz_cd_diagram_draws_through_tex() {
    if !pictures_here("a_tikz_cd_diagram_draws_through_tex") {
        return;
    }
    let diagram = r#"\begin{tikzcd} A \arrow[r, "f"] \arrow[d] & B \arrow[d] \\ C \arrow[r] & D \end{tikzcd}"#;
    let results = render(json!([{ "latex": diagram, "display": true }]), options());
    assert_eq!(results[0]["engine"], "tex", "{}", results[0]);
    assert!(results[0]["rows"].as_u64().unwrap() >= 3, "{}", results[0]);
    let off = render(
        json!([{ "latex": diagram, "display": true }]),
        json!({ "fg": "#4c4f69", "cell": "19x42", "texFallback": false }),
    );
    assert!(off[0]["error"].as_str().unwrap().contains("texFallback"), "{}", off[0]);
}

#[test]
fn a_tikz_block_draws_as_a_picture_that_fits() {
    if !pictures_here("a_tikz_block_draws_as_a_picture_that_fits") {
        return;
    }
    let picture = "\\usetikzlibrary{arrows.meta}\n\\draw[-{Stealth}] (0,0) -- (12,3) node[right] {$x^2$};\n\\draw (0,0) circle (1);";
    let tall = "\\draw (0,0) -- (0,40);";
    let results = render(
        json!([
            { "latex": picture, "kind": "tikz" },
            { "latex": tall, "kind": "tikz" },
            { "latex": "\\usepackage{shellesc}\n\\draw (0,0) -- (1,1);", "kind": "tikz" },
        ]),
        json!({ "fg": "#4c4f69", "cell": "19x42", "maxColumns": 40 }),
    );
    assert_eq!(results[0]["engine"], "tex", "{}", results[0]);
    assert!(results[0]["columns"].as_u64().unwrap() <= 40, "{}", results[0]);
    assert!(results[0]["png"].as_str().unwrap().starts_with("iVBORw0KGgo"));
    assert_eq!(results[1]["rows"].as_u64(), Some(80), "{}", results[1]);
    assert!(
        results[2]["error"].as_str().unwrap().contains("shellesc"),
        "{}",
        results[2]
    );
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
    assert_eq!(health["texFallback"], true);
    assert_eq!(health["tex"]["latex"], on_path("latex"));
    assert_eq!(health["tex"]["gs"], on_path("gs"));
    assert!(health["problems"].as_array().unwrap().is_empty(), "{health}");
    if tex_here("check_reports_a_working_engine") {
        assert_eq!(health["tex"]["works"], true, "{health}");
    }
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
    let (mut compared, mut close, mut same_row, mut by_tex) = (0, 0, 0, 0);
    let mut far = Vec::new();
    let mut missing = Vec::new();
    for (old, new) in items.iter().zip(&results) {
        if new["engine"] == "tex" {
            by_tex += 1;
        }
        if old["corpus"] == "claude" && old.get("error").is_none() && new.get("error").is_some() {
            missing.push(old["latex"].as_str().unwrap().to_string());
        }
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
    eprintln!(
        "{compared} compared, {close} within one cell, {same_row} on the same text row; {by_tex} drawn by the TeX fallback"
    );
    if tex_here("the TeX part of the corpus test") {
        assert!(
            missing.is_empty(),
            "Claude's formulas the old backend drew: {missing:?}"
        );
        assert!(by_tex >= 6, "only {by_tex} drawn by TeX");
    }
    assert!(compared > 1400, "only {compared} formulas drawn by both");
    assert!(close_share >= 0.97, "{close_share:.3} within one cell; far: {far:?}");
    assert!(row_share >= 0.99, "{row_share:.3} on the same text row");
}

/// A ```mermaid block's source, from tests/fixtures/mermaid.
fn mermaid(name: &str) -> String {
    let path = format!("{}/tests/fixtures/mermaid/{name}.mmd", env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(path).unwrap()
}

fn diagram(source: &str, max_columns: u32) -> Value {
    let results = render(
        json!([{ "latex": source, "kind": "diagram" }]),
        json!({ "maxColumns": max_columns }),
    );
    results[0].clone()
}

fn drawn_lines(answer: &Value) -> Vec<String> {
    let lines = answer["lines"]
        .as_array()
        .unwrap_or_else(|| panic!("no drawing: {answer}"));
    lines.iter().map(|line| line.as_str().unwrap().to_string()).collect()
}

/// The text of every `id["label"]` in a flowchart.
fn flowchart_labels(source: &str) -> Vec<String> {
    let label = regex::Regex::new(r#"\["([^"]*)"\]"#).unwrap();
    label.captures_iter(source).map(|c| c[1].to_string()).collect()
}

#[test]
fn tree_flowcharts_draw_as_the_mindmaps_they_are() {
    // Claude's own diagrams, written both ways: about 40 nodes, Chinese labels.
    for (tree, mind, size) in [
        ("tree-short", "mindmap-short", (60, 38)),
        ("tree-long", "mindmap-long", (67, 39)),
    ] {
        let flow = diagram(&mermaid(tree), 255);
        let lines = drawn_lines(&flow);
        assert_eq!(lines, drawn_lines(&diagram(&mermaid(mind), 255)), "{tree} and {mind}");
        assert_eq!((flow["columns"].as_u64().unwrap(), lines.len() as u64), size, "{tree}");
        // Every label as written, each on a line of its own.
        for label in flowchart_labels(&mermaid(tree)) {
            assert!(
                lines
                    .iter()
                    .any(|line| line.ends_with(&format!("── {label}")) || line == &format!("│ {label} │")),
                "{tree}: {label} not drawn as written"
            );
        }
    }
}

#[test]
fn a_flowchart_that_is_not_a_tree_is_left_to_the_hooks() {
    let results = render(
        json!([
            { "latex": "graph TD\n  A --> C\n  B --> C", "kind": "diagram" },
            { "latex": "flowchart LR\n  A -->|yes| B", "kind": "diagram" },
            { "latex": "x", "display": false },
        ]),
        options(),
    );
    assert_eq!(results[0]["error"], "not a tree: C has two parents");
    assert_eq!(results[1]["error"], "not a tree: an edge has a label");
    assert!(results[2]["png"].is_string(), "a formula in the same batch draws");
}

#[test]
fn a_diagram_keeps_to_max_columns() {
    let source = mermaid("tree-long");
    let narrow = diagram(&source, 40);
    let lines = drawn_lines(&narrow);
    assert!(narrow["columns"].as_u64().unwrap() <= 40);
    assert!(lines.iter().any(|line| line.ends_with('…')), "long labels cut");
    assert_eq!(diagram(&source, 16)["error"], "tree is 67 columns wide, 16 available");
}
