//! The TeX fallback: a formula RaTeX cannot typeset (a package command such
//! as siunitx's \SI or physics' \dv) is typeset by TeX Live instead, when
//! `latex` and `dvipng` are on PATH and the texFallback option is on. TeX
//! also draws what RaTeX never can: tikz-cd diagrams in a formula and
//! ```tikz blocks, both through dvips and Ghostscript, since dvipng
//! misplaces TikZ's text.
//!
//! Each formula runs alone (fallback is rare), isolated by sandbox.rs, and
//! its result is kept on disk (cache.rs). The raster follows RaTeX's scale
//! (dpi = ptPx * 72.27 * SUPERSAMPLE), so fit.rs places a TeX formula with
//! the same arithmetic, its baseline on the text baseline.

use std::collections::HashSet;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{LazyLock, Mutex, OnceLock};
use std::time::{Duration, Instant};

use regex::Regex;
use serde_json::{Value, json};

use crate::cache;
use crate::fit::{self, Ink, MAX_PIXELS, SUPERSAMPLE};
use crate::protocol::{Cell, ItemResult, MAX_BOX, MIN_SCALE, Options};
use crate::sandbox::{self, Outcome, Workdir};

/// One formula's latex run; a run that takes longer is the formula's fault.
const FORMULA_TIMEOUT: Duration = Duration::from_secs(6);
/// A TikZ picture's latex run (pgfplots computes its plots in TeX).
const PICTURE_TIMEOUT: Duration = Duration::from_secs(20);
/// dvipng, dvips, gs, kpsewhich.
const TOOL_TIMEOUT: Duration = Duration::from_secs(10);
/// All TeX runs of one request; later items fail with a passing "timeout".
const REQUEST_BUDGET: Duration = Duration::from_secs(30);
const PARALLEL: usize = 4;
/// Part of every cache key: raise it when the preamble or the way a
/// picture is made changes, so no picture of the old one is reused.
const REVISION: u32 = 1;
const LOG_READ: u64 = 4 * 1024 * 1024;
/// A ```tikz block is at most this many rows tall.
pub const TIKZ_ROWS: u32 = 80;

// Space kept around each formula's box, as ink outside the box is cut off at
// the page edge: 1pt holds an italic overhang. A formula built to put ink
// outside its box (\mathclap, \rlap, a CD arrow's side label...) gets
// OVERHANG_BORDER_PT. fit.rs crops the transparent margin away; dvipng's
// depth counts the border, so the baseline stays where it is.
const BORDER_PT: f64 = 1.0;
const OVERHANG_BORDER_PT: f64 = 60.0;
const PICTURE_BORDER_PT: f64 = 4.0; // TikZ's bounding box leaves out half a line width
static OVERHANG: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\\(?:math[clr]lap|[clr]lap|smash|begin\s*\{CD\}|makebox)(?:[^A-Za-z]|$)|\\(?:hspace\*?|kern|mkern|hskip|mskip)\s*\{?\s*-").unwrap()
});
static DIAGRAM: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\\begin\s*\{tikzcd\}").unwrap());

/// Packages of the preamble, in this order, each when installed (preview,
/// amsmath and amssymb are needed). Then the user's `packages`, preview,
/// the user's `macros` and compat.tex. Keep within TeX's 16 math families.
const PACKAGES: &[(&str, &str)] = &[
    // [T1,OT1]: the T1 glyphs (\th, \dh...) are there, OT1 stays the default.
    ("fontenc", "[T1,OT1]"),
    ("lmodern", ""),
    ("amsmath", ""),
    ("amssymb", ""),
    ("mathtools", ""),
    ("mathrsfs", ""),
    ("cancel", ""),
    ("xcolor", ""),
    ("bm", ""),
    ("CJKutf8", ""),
    ("mhchem", "[version=4]"),
    ("siunitx", ""),
    ("dsfont", ""),
    ("stmaryrd", ""),
    ("amscd", ""),
    ("ulem", "[normalem]"),
    ("centernot", ""),
];
const REQUIRED: &[&str] = &["preview", "amsmath", "amssymb"];
// stmaryrd turns the kernel's binary \bigtriangleup/\bigtriangledown into
// big operators; KaTeX and LaTeX keep them binary, so they come back after it.
const STMARYRD_BEFORE: &str = r"\let\tsbigtriangleup\bigtriangleup\let\tsbigtriangledown\bigtriangledown";
const STMARYRD_AFTER: &str = r"\let\bigtriangleup\tsbigtriangleup\let\bigtriangledown\tsbigtriangledown";
/// Packages a ```tikz block may load besides the user's `packages` (tikz
/// and xcolor are loaded anyway).
const TIKZ_PACKAGES: &[&str] = &["pgfplots", "circuitikz", "tikz-3dplot", "tikz", "xcolor"];
const CJK_FONTS: &[&str] = &["gbsn", "bsmi", "ipxm", "mj"];
const COMPAT: &str = include_str!("compat.tex");

// CSS colour names (KaTeX takes them in \color) that xcolor lacks; xcolor's
// own names keep xcolor's values.
const CSS_COLOURS: &str = "aliceblue f0f8ff antiquewhite faebd7 aqua 00ffff aquamarine 7fffd4 azure f0ffff \
beige f5f5dc bisque ffe4c4 black 000000 blanchedalmond ffebcd blue 0000ff blueviolet 8a2be2 brown a52a2a \
burlywood deb887 cadetblue 5f9ea0 chartreuse 7fff00 chocolate d2691e coral ff7f50 cornflowerblue 6495ed \
cornsilk fff8dc crimson dc143c cyan 00ffff darkblue 00008b darkcyan 008b8b darkgoldenrod b8860b \
darkgray a9a9a9 darkgreen 006400 darkgrey a9a9a9 darkkhaki bdb76b darkmagenta 8b008b \
darkolivegreen 556b2f darkorange ff8c00 darkorchid 9932cc darkred 8b0000 darksalmon e9967a \
darkseagreen 8fbc8f darkslateblue 483d8b darkslategray 2f4f4f darkslategrey 2f4f4f darkturquoise 00ced1 \
darkviolet 9400d3 deeppink ff1493 deepskyblue 00bfff dimgray 696969 dimgrey 696969 dodgerblue 1e90ff \
firebrick b22222 floralwhite fffaf0 forestgreen 228b22 fuchsia ff00ff gainsboro dcdcdc ghostwhite f8f8ff \
gold ffd700 goldenrod daa520 gray 808080 green 008000 greenyellow adff2f grey 808080 honeydew f0fff0 \
hotpink ff69b4 indianred cd5c5c indigo 4b0082 ivory fffff0 khaki f0e68c lavender e6e6fa \
lavenderblush fff0f5 lawngreen 7cfc00 lemonchiffon fffacd lightblue add8e6 lightcoral f08080 \
lightcyan e0ffff lightgoldenrodyellow fafad2 lightgray d3d3d3 lightgreen 90ee90 lightgrey d3d3d3 \
lightpink ffb6c1 lightsalmon ffa07a lightseagreen 20b2aa lightskyblue 87cefa lightslategray 778899 \
lightslategrey 778899 lightsteelblue b0c4de lightyellow ffffe0 lime 00ff00 limegreen 32cd32 linen faf0e6 \
magenta ff00ff maroon 800000 mediumaquamarine 66cdaa mediumblue 0000cd mediumorchid ba55d3 \
mediumpurple 9370db mediumseagreen 3cb371 mediumslateblue 7b68ee mediumspringgreen 00fa9a \
mediumturquoise 48d1cc mediumvioletred c71585 midnightblue 191970 mintcream f5fffa mistyrose ffe4e1 \
moccasin ffe4b5 navajowhite ffdead navy 000080 oldlace fdf5e6 olive 808000 olivedrab 6b8e23 orange ffa500 \
orangered ff4500 orchid da70d6 palegoldenrod eee8aa palegreen 98fb98 paleturquoise afeeee \
palevioletred db7093 papayawhip ffefd5 peachpuff ffdab9 peru cd853f pink ffc0cb plum dda0dd \
powderblue b0e0e6 purple 800080 rebeccapurple 663399 red ff0000 rosybrown bc8f8f royalblue 4169e1 \
saddlebrown 8b4513 salmon fa8072 sandybrown f4a460 seagreen 2e8b57 seashell fff5ee sienna a0522d \
silver c0c0c0 skyblue 87ceeb slateblue 6a5acd slategray 708090 slategrey 708090 snow fffafa \
springgreen 00ff7f steelblue 4682b4 tan d2b48c teal 008080 thistle d8bfd8 tomato ff6347 turquoise 40e0d0 \
violet ee82ee wheat f5deb3 white ffffff whitesmoke f5f5f5 yellow ffff00 yellowgreen 9acd32";

/// The TeX programs found on PATH.
#[derive(Debug, Clone, Copy)]
pub struct Tools {
    pub latex: bool,
    pub dvipng: bool,
    pub dvips: bool,
    pub gs: bool,
}

impl Tools {
    pub fn find() -> Tools {
        Tools {
            latex: sandbox::on_path("latex"),
            dvipng: sandbox::on_path("dvipng"),
            dvips: sandbox::on_path("dvips"),
            gs: sandbox::on_path("gs"),
        }
    }

    /// What a picture through dvips and Ghostscript still lacks, as words.
    fn missing_for_pictures(self) -> Vec<&'static str> {
        [("latex", self.latex), ("dvips", self.dvips), ("gs", self.gs)]
            .iter()
            .filter(|(_, found)| !found)
            .map(|(name, _)| *name)
            .collect()
    }
}

/// Whether a formula is a tikz-cd diagram, which only TeX draws.
pub fn is_diagram(latex: &str) -> bool {
    DIAGRAM.is_match(latex)
}

/// Which of the files the preamble may load this TeX installation has:
/// one kpsewhich call per process (a process serves one request, so one
/// set of user packages), made only when TeX is needed.
fn installed(user_packages: &[String]) -> &'static HashSet<String> {
    static FOUND: OnceLock<HashSet<String>> = OnceLock::new();
    FOUND.get_or_init(|| {
        let mut files: Vec<String> = PACKAGES
            .iter()
            .map(|(name, _)| format!("{name}.sty"))
            .chain(["preview.sty", "tikz.sty", "tikz-cd.sty"].map(String::from))
            .chain(CJK_FONTS.iter().map(|font| format!("c70{font}.fd")))
            .chain(user_packages.iter().map(|name| format!("{name}.sty")))
            .collect();
        files.dedup();
        kpsewhich(&files)
    })
}

/// The names of those of `files` kpsewhich finds (it exits 1 when any one
/// is missing, so its answer is read whatever the exit status).
fn kpsewhich(files: &[String]) -> HashSet<String> {
    let Ok(dir) = Workdir::new() else {
        return HashSet::new();
    };
    let mut argv = vec!["kpsewhich", "-engine=pdftex"];
    argv.extend(files.iter().map(String::as_str));
    sandbox::run(&argv, &dir.path, TOOL_TIMEOUT, Some("out"));
    let out = sandbox::read_text(&dir.file("out"), 1 << 20).unwrap_or_default();
    out.lines()
        .filter_map(|line| Path::new(line.trim()).file_name())
        .map(|name| name.to_string_lossy().into_owned())
        .collect()
}

pub fn has_package(user_packages: &[String], name: &str) -> bool {
    installed(user_packages).contains(&format!("{name}.sty"))
}

/// The first TeX "!" line, with the undefined name when there is one.
fn first_error(log: &str) -> Option<String> {
    static FOLLOW: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^(?:l\.\d+|<[a-z ]+>) .*?(\\[A-Za-z]+|\\.)\s*$").unwrap());
    let lines: Vec<&str> = log.lines().collect();
    for (k, line) in lines.iter().enumerate() {
        if !line.starts_with('!') {
            continue;
        }
        if line.contains("Undefined control sequence") {
            for follow in lines.iter().skip(k + 1).take(5) {
                if let Some(found) = FOLLOW.captures(follow) {
                    return Some(format!("{line} {}", &found[1]));
                }
            }
        }
        return Some(line.chars().take(300).collect());
    }
    None
}

/// The box a run measured: (width, height, depth) in TeX points.
fn measured(log: &str) -> Option<(f64, f64, f64)> {
    static DIM: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?m)^TSDIM (-?[\d.]+)pt,(-?[\d.]+)pt,(-?[\d.]+)pt$").unwrap());
    // The last: a formula may \typeout a line that looks like it, but
    // only before the real one, which comes after the box.
    let found = DIM.captures_iter(log).last()?;
    Some((found[1].parse().ok()?, found[2].parse().ok()?, found[3].parse().ok()?))
}

/// Why a TeX run gave no picture.
#[derive(Debug)]
enum Failure {
    /// The input's fault: a TeX error or a size is kept on disk, a limit
    /// it ran into is not (see `cached`).
    Refused(String),
    /// A passing reason (a tool, the request's time): not kept.
    Passing(String),
}

/// A ```tikz block, split into what goes into the preamble and the picture.
#[derive(Debug, PartialEq, Eq)]
pub struct TikzSource {
    pub preamble: Vec<String>,
    pub picture: String,
}

/// The block's `\usetikzlibrary` and `\usepackage` lines (allowed only at
/// its top, and \usepackage only for TIKZ_PACKAGES and the user's
/// `packages`), and the picture: a whole tikzpicture, or the inside of one.
/// A pasted standalone document's \documentclass and document environment
/// are dropped.
pub fn parse_tikz(source: &str, user_packages: &[String]) -> Result<TikzSource, String> {
    static USEPACKAGE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^\\usepackage\s*(\[[^\]]*\])?\s*\{([^{}]*)\}\s*$").unwrap());
    static LIBRARY: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^\\use(?:tikz|pgfplots)library\s*(?:\[[^\]]*\])?\s*\{[^{}]*\}\s*$").unwrap());
    static WRAPPER: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"^(?:\\documentclass\s*(?:\[[^\]]*\])?\s*\{[^{}]*\}|\\(?:begin|end)\s*\{document\})\s*$").unwrap()
    });
    static LATE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"\\(?:usepackage|RequirePackage|documentclass)(?:[^A-Za-z]|$)").unwrap());
    static DOCUMENT_END: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\\end\s*\{document\}\s*$").unwrap());

    let mut preamble = Vec::new();
    let mut lines = source.lines().peekable();
    while let Some(line) = lines.peek() {
        let text = line.trim();
        if text.is_empty() || text.starts_with('%') || WRAPPER.is_match(text) {
            lines.next();
            continue;
        }
        if LIBRARY.is_match(text) {
            preamble.push(text.to_string());
        } else if let Some(found) = USEPACKAGE.captures(text) {
            for name in found[2].split(',').map(str::trim).filter(|n| !n.is_empty()) {
                if !TIKZ_PACKAGES.contains(&name) && !user_packages.iter().any(|p| p == name) {
                    return Err(format!(
                        "\\usepackage{{{name}}} is not allowed in a tikz block (only pgfplots, circuitikz, tikz-3dplot and the packages option)"
                    ));
                }
            }
            preamble.push(text.to_string());
        } else {
            break;
        }
        lines.next();
    }
    let rest: Vec<&str> = lines.collect();
    let body = DOCUMENT_END.replace(rest.join("\n").trim_end(), "").trim().to_string();
    if LATE.is_match(&body) {
        return Err("\\usepackage is allowed only at the top of a tikz block".into());
    }
    if body.is_empty() {
        return Err("the tikz block is empty".into());
    }
    let picture = if body.contains(r"\begin{tikzpicture}") {
        body
    } else {
        format!("\\begin{{tikzpicture}}\n{body}\n\\end{{tikzpicture}}")
    };
    Ok(TikzSource { preamble, picture })
}

/// What to typeset with TeX, and for a formula RaTeX refused, why it did.
#[derive(Debug, Clone)]
pub enum Job {
    Formula {
        latex: String,
        display: bool,
        ratex_error: String,
    },
    Diagram {
        latex: String,
        display: bool,
    },
    Tikz {
        source: String,
    },
}

/// The TeX fallback for one request.
pub struct Tex<'a> {
    cfg: &'a Options,
    cell: Cell,
    fg: String,
    tools: Tools,
    deadline: Instant,
    wrote: AtomicBool,
}

impl<'a> Tex<'a> {
    pub fn new(cfg: &'a Options, cell: Cell, fg: &str) -> Self {
        Tex {
            cfg,
            cell,
            fg: fg.to_ascii_uppercase().trim_start_matches('#').to_string(),
            tools: Tools::find(),
            deadline: Instant::now() + REQUEST_BUDGET,
            wrote: AtomicBool::new(false),
        }
    }

    /// Whether a formula RaTeX refused goes to TeX.
    pub fn falls_back(&self) -> bool {
        self.cfg.tex_fallback && self.tools.latex && self.tools.dvipng
    }

    /// The results of `jobs`, in order, a few side by side.
    pub fn run_all(&self, jobs: &[Job]) -> Vec<ItemResult> {
        let results: Mutex<Vec<Option<ItemResult>>> = Mutex::new(vec![None; jobs.len()]);
        let next = AtomicUsize::new(0);
        let workers = jobs.len().min(PARALLEL);
        std::thread::scope(|scope| {
            for _ in 0..workers {
                scope.spawn(|| {
                    loop {
                        let k = next.fetch_add(1, Ordering::Relaxed);
                        let Some(job) = jobs.get(k) else { break };
                        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.run(job)))
                            .unwrap_or_else(|_| ItemResult::failure("renderer failed on this formula"));
                        results.lock().unwrap_or_else(|e| e.into_inner())[k] = Some(outcome);
                    }
                });
            }
        });
        if self.wrote.load(Ordering::Relaxed) {
            cache::prune();
        }
        results
            .into_inner()
            .unwrap_or_else(|e| e.into_inner())
            .into_iter()
            .map(|r| r.unwrap_or_else(|| ItemResult::failure("renderer failed on this formula")))
            .collect()
    }

    fn run(&self, job: &Job) -> ItemResult {
        match job {
            Job::Formula {
                latex,
                display,
                ratex_error,
            } => self.cached(
                "formula",
                *display,
                latex,
                || self.formula(latex, *display, false),
                |error| format!("{ratex_error}; TeX: {error}"),
            ),
            Job::Diagram { latex, display } => {
                if !self.cfg.tex_fallback {
                    return ItemResult::failure("tikz-cd needs the TeX fallback, which texFallback turns off");
                }
                let missing = self.tools.missing_for_pictures();
                if !missing.is_empty() {
                    return ItemResult::failure(format!(
                        "tikz-cd needs TeX Live, dvips and Ghostscript: {} not found",
                        missing.join(", ")
                    ));
                }
                if !has_package(&self.cfg.packages, "tikz-cd") {
                    return ItemResult::failure("tikz-cd is not installed");
                }
                self.cached(
                    "diagram",
                    *display,
                    latex,
                    || self.formula(latex, *display, true),
                    |e| e,
                )
            }
            Job::Tikz { source } => {
                if !self.cfg.tex_fallback {
                    return ItemResult::failure("tikz pictures need the TeX fallback, which texFallback turns off");
                }
                let missing = self.tools.missing_for_pictures();
                if !missing.is_empty() {
                    return ItemResult::failure(format!(
                        "tikz pictures need TeX Live and Ghostscript: {} not found",
                        missing.join(", ")
                    ));
                }
                let parsed = match parse_tikz(source, &self.cfg.packages) {
                    Ok(parsed) => parsed,
                    Err(error) => return ItemResult::failure(error),
                };
                self.cached("tikz", true, source, || self.tikz(&parsed), |e| e)
            }
        }
    }

    /// `make`'s result, from the disk cache when it holds it; a failure's
    /// text through `explain`. Only results that depend on the input alone
    /// are kept. A picture stays right when a package is installed later,
    /// a refusal may not (the package was missing), so a refusal is filed
    /// under the installed packages too; a picture is found without
    /// asking kpsewhich.
    fn cached(
        &self,
        kind: &str,
        display: bool,
        latex: &str,
        make: impl FnOnce() -> Result<ItemResult, Failure>,
        explain: impl FnOnce(String) -> String,
    ) -> ItemResult {
        let picture_key = cache::key(&self.key_parts(kind, display, latex, Value::Null));
        if let Some(hit) = cache::get(&picture_key) {
            return hit;
        }
        let mut found: Vec<&String> = installed(&self.cfg.packages).iter().collect();
        found.sort();
        let refusal_key = cache::key(&self.key_parts(kind, display, latex, json!(found)));
        if let Some(hit) = cache::get(&refusal_key) {
            return hit;
        }
        let (result, key) = match make() {
            Ok(picture) => (picture.by("tex"), Some(&picture_key)),
            Err(Failure::Refused(error)) => {
                // As in the old backend, a TeX error or a size is kept; a
                // time or memory limit may be the machine's (busy, a cold
                // TeX Live) and is not.
                let lasting = error.starts_with('!') || error.starts_with("too large");
                (ItemResult::failure(explain(error)), lasting.then_some(&refusal_key))
            }
            Err(Failure::Passing(error)) => (ItemResult::failure(error), None),
        };
        if let Some(key) = key
            && cache::put(key, &result)
        {
            self.wrote.store(true, Ordering::Relaxed);
        }
        result
    }

    /// Everything a TeX picture depends on (`installed`: the packages TeX
    /// Live has, for a refusal; null for a picture).
    fn key_parts(&self, kind: &str, display: bool, latex: &str, installed: Value) -> Value {
        let cfg = self.cfg;
        json!([
            crate::VERSION,
            REVISION,
            kind,
            display,
            latex,
            self.fg,
            [self.cell.width, self.cell.height],
            cfg.baseline,
            cfg.pt_px,
            cfg.min_shrink,
            cfg.thicken,
            cfg.max_columns,
            cfg.packages,
            cfg.macros,
            installed,
        ])
    }

    /// A run's time limit: its own, or what is left of the request's.
    fn time_for(&self, own: Duration) -> Result<(Duration, bool), Failure> {
        let left = self.deadline.saturating_duration_since(Instant::now());
        if left < Duration::from_millis(200) {
            return Err(Failure::Passing(
                "timeout: the TeX time of this batch is used up".into(),
            ));
        }
        Ok(if left < own { (left, true) } else { (own, false) })
    }

    /// The preamble, up to and with \begin{document}; `extra` lines come
    /// after the built-in packages.
    fn preamble(&self, extra: &[String]) -> String {
        let have = installed(&self.cfg.packages);
        let has = |name: &str| have.contains(&format!("{name}.sty"));
        let mut lines = vec![r"\documentclass[10pt]{article}".to_string()];
        for (name, options) in PACKAGES {
            if !has(name) {
                continue;
            }
            if *name == "stmaryrd" {
                lines.push(STMARYRD_BEFORE.into());
            }
            lines.push(format!(r"\usepackage{options}{{{name}}}"));
            if *name == "stmaryrd" {
                lines.push(STMARYRD_AFTER.into());
            }
        }
        lines.extend(extra.iter().cloned());
        for name in &self.cfg.packages {
            if has(name) {
                lines.push(format!(r"\usepackage{{{name}}}"));
            }
        }
        lines.push(r"\usepackage[active,tightpage]{preview}".into());
        if !self.cfg.macros.is_empty() {
            // The user's definitions win over a package's (physics defines
            // \Tr, \abs, \qty): each name they define is first \let to
            // \relax, which LaTeX takes as not defined. \relax ends a
            // definition left open-ended.
            lines.push(user_first(&self.cfg.macros));
            lines.push(self.cfg.macros.clone());
            lines.push(r"\relax".into());
        }
        if has("xcolor") {
            let pairs: Vec<&str> = CSS_COLOURS.split_whitespace().collect();
            lines.push(
                pairs
                    .chunks(2)
                    .map(|pair| {
                        format!(
                            r"\providecolor{{{}}}{{HTML}}{{{}}}",
                            pair[0],
                            pair[1].to_ascii_uppercase()
                        )
                    })
                    .collect(),
            );
        }
        lines.push(COMPAT.into());
        lines.push(r"\pagestyle{empty}".into());
        lines.push(r"\begin{document}".into());
        lines.join("\n") + "\n"
    }

    /// The CJK environment around `text` when it has CJK and CJKutf8 is
    /// there: Korean for Hangul, Japanese for kana, else Simplified Chinese.
    fn cjk(&self, text: &str) -> (String, String) {
        let have = installed(&self.cfg.packages);
        if !crate::fonts::needs_cjk(text) || !have.contains("CJKutf8.sty") {
            return (String::new(), String::new());
        }
        let hangul = text
            .chars()
            .any(|c| matches!(c as u32, 0x1100..=0x11FF | 0x3130..=0x318F | 0xAC00..=0xD7AF));
        let kana = text
            .chars()
            .any(|c| matches!(c as u32, 0x3040..=0x30FF | 0x31F0..=0x31FF));
        let font = if hangul && have.contains("c70mj.fd") {
            "mj"
        } else if kana && have.contains("c70ipxm.fd") {
            "ipxm"
        } else {
            "gbsn"
        };
        (format!("\\begin{{CJK}}{{UTF8}}{{{font}}}"), r"\end{CJK}".into())
    }

    /// The colour the picture is drawn in: xcolor writes its own colour,
    /// which makes dvipng ignore -fg, so it is set in the document too.
    fn colour(&self) -> String {
        if installed(&self.cfg.packages).contains("xcolor.sty") {
            format!(r"\color[HTML]{{{}}}", self.fg)
        } else {
            String::new()
        }
    }

    fn full_dpi(&self) -> f64 {
        self.cfg.pt_px * 72.27 * f64::from(SUPERSAMPLE)
    }

    /// The dpi to raster a page of `w` x `h` pt at: the full scale, or less
    /// for one that is drawn smaller (`scale`) or would raster past MAX_PIXELS.
    fn dpi_for(&self, w: f64, h: f64, scale: f64) -> f64 {
        let full = self.full_dpi();
        let area = (w * h * (full / 72.27).powi(2)).max(1.0);
        let fits = (0.8 * MAX_PIXELS as f64 / area).sqrt();
        (full * scale.min(fits).min(1.0)).round().max(1.0)
    }

    /// Writes f.tex and runs latex on it: the log, or why there is none.
    fn latex(&self, dir: &Workdir, document: &str, own: Duration) -> Result<String, Failure> {
        std::fs::write(dir.file("f.tex"), document)
            .map_err(|e| Failure::Passing(format!("renderer could not write the TeX file: {e}")))?;
        let (limit, by_budget) = self.time_for(own)?;
        let argv = [
            "latex",
            "-no-shell-escape",
            "-interaction=nonstopmode",
            "-halt-on-error",
            "f.tex",
        ];
        let outcome = sandbox::run(&argv, &dir.path, limit, None);
        let log = sandbox::read_text(&dir.file("f.log"), LOG_READ).unwrap_or_default();
        match outcome {
            Outcome::TimedOut if by_budget => Err(Failure::Passing(
                "timeout: the TeX time of this batch is used up".into(),
            )),
            Outcome::TimedOut => Err(Failure::Refused(format!("TeX took longer than {} s", own.as_secs()))),
            Outcome::TooMuchOutput => Err(Failure::Refused("TeX wrote too much".into())),
            Outcome::NotStarted => Err(Failure::Passing("renderer could not start latex".into())),
            _ if first_error(&log).is_some() => Err(Failure::Refused(first_error(&log).unwrap_or_default())),
            Outcome::Signalled => Err(Failure::Refused("TeX stopped at its memory or file size limit".into())),
            Outcome::Exited(0) => Ok(log),
            Outcome::Exited(code) => Err(Failure::Refused(format!("latex exited {code}"))),
        }
    }

    /// A formula typeset by TeX: through dvipng, or for a tikz-cd diagram
    /// (`diagram`) through dvips and Ghostscript.
    fn formula(&self, latex: &str, display: bool, diagram: bool) -> Result<ItemResult, Failure> {
        let dir = Workdir::new().map_err(|e| Failure::Passing(format!("renderer could not make a folder: {e}")))?;
        let style = if display { r"\displaystyle" } else { r"\textstyle" };
        let border = if !diagram && OVERHANG.is_match(latex) {
            OVERHANG_BORDER_PT
        } else {
            BORDER_PT
        };
        let (cjk_open, cjk_close) = self.cjk(latex);
        let colour = self.colour();
        // The newline ends a trailing comment. A diagram is no macro
        // argument (\sbox's would fix the catcode of its &), and is
        // typeset again for its page rather than reused from the box.
        let (open, draw) = if diagram {
            (r"\setbox0\hbox{", format!("{{{colour}${style} {latex}\n$}}"))
        } else {
            (r"\sbox0{", r"\usebox0".to_string())
        };
        let extra = if diagram {
            vec![r"\usepackage{tikz-cd}".to_string()]
        } else {
            Vec::new()
        };
        let document = format!(
            "{preamble}{cjk_open}\\setlength\\PreviewBorder{{{border}pt}}\\begin{{preview}}{open}{colour}${style} {latex}\n$}}\
             \\typeout{{TSDIM \\the\\wd0,\\the\\ht0,\\the\\dp0}}{draw}\\end{{preview}}{cjk_close}\n\\end{{document}}\n",
            preamble = self.preamble(&extra),
        );
        let own = if diagram { PICTURE_TIMEOUT } else { FORMULA_TIMEOUT };
        let log = self.latex(&dir, &document, own)?;
        let (wd, ht, dp) = measured(&log).ok_or_else(|| Failure::Refused("TeX made no page".into()))?;

        // Drawn smaller to fit the reply (or not at all below MIN_SCALE):
        // rastered at that scale, three times the terminal's pixels still.
        let (cw, ch) = (f64::from(self.cell.width), f64::from(self.cell.height));
        let pt_px = self.cfg.pt_px;
        let scale = 1f64
            .min((f64::from(self.cfg.max_columns) * cw - 5.0) / (wd * pt_px).max(1.0))
            .min((f64::from(MAX_BOX) * ch - 2.0) / ((ht + dp) * pt_px).max(1.0));
        if scale < MIN_SCALE {
            return Err(Failure::Refused("too large".into()));
        }
        let dpi = self.dpi_for(wd + 2.0 * border, ht + dp + 2.0 * border, scale);
        let ink = if diagram {
            self.diagram_ink(&dir, dpi, dp + border)?
        } else {
            self.dvipng(&dir, dpi)?
        };
        Ok(fit::fit_at(&ink, dpi / 72.27 / pt_px, display, self.cfg, self.cell))
    }

    /// Page 1 of f.dvi through dvipng, with its depth below the baseline.
    fn dvipng(&self, dir: &Workdir, dpi: f64) -> Result<Ink, Failure> {
        let [r, g, b] = crate::detect::rgb(&format!("#{}", self.fg));
        let fg = format!("rgb {r:.4} {g:.4} {b:.4}");
        let dpi = format!("{dpi}");
        let argv = [
            "dvipng",
            "-T",
            "tight",
            "-bg",
            "Transparent",
            "--depth",
            "-z",
            "1",
            // No formula needs PostScript: Ghostscript is never started for it.
            "--noghostscript",
            "-D",
            &dpi,
            "-fg",
            &fg,
            "-o",
            "p%d.png",
            "f.dvi",
        ];
        let outcome = sandbox::run(&argv, &dir.path, TOOL_TIMEOUT, Some("dvipng.out"));
        if !outcome.ok() {
            return Err(Failure::Passing(format!("dvipng {outcome}")));
        }
        static DEPTH: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\[1[^\[\]]*?depth=(-?\d+)").unwrap());
        let out = sandbox::read_text(&dir.file("dvipng.out"), 1 << 20).unwrap_or_default();
        let depth: i64 = DEPTH
            .captures(&out)
            .and_then(|c| c[1].parse().ok())
            .ok_or_else(|| Failure::Passing("dvipng gave no depth".into()))?;
        let png = std::fs::read(dir.file("p1.png")).map_err(|_| Failure::Passing("dvipng drew nothing".into()))?;
        let (rgba, width, height) = fit::decode(&png).map_err(Failure::Refused)?;
        Ok(Ink {
            rgba,
            width,
            height,
            depth,
        })
    }

    /// Page 1 of f.dvi through Ghostscript, `below` pt of it under the
    /// baseline (the box's depth and the border): Ghostscript reports no
    /// depth, and its page is the tight page preview set.
    fn diagram_ink(&self, dir: &Workdir, dpi: f64, below: f64) -> Result<Ink, Failure> {
        let png = self.ghostscript(dir, dpi)?;
        let (rgba, width, height) = fit::decode(&png).map_err(Failure::Refused)?;
        Ok(Ink {
            rgba,
            width,
            height,
            depth: (below * dpi / 72.27).round() as i64,
        })
    }

    /// Page 1 of f.dvi through dvips (-R2: no shell, no absolute paths) and
    /// Ghostscript (-dSAFER), as a PNG with a transparent background.
    fn ghostscript(&self, dir: &Workdir, dpi: f64) -> Result<Vec<u8>, Failure> {
        let dvips = sandbox::run(
            &["dvips", "-q", "-R2", "-o", "f.ps", "f.dvi"],
            &dir.path,
            TOOL_TIMEOUT,
            None,
        );
        if !dvips.ok() {
            return Err(Failure::Passing(format!("dvips {dvips}")));
        }
        let resolution = format!("-r{dpi}");
        let gs = sandbox::run(
            &[
                "gs",
                "-dSAFER",
                "-dBATCH",
                "-dNOPAUSE",
                "-dQUIET",
                "-dLastPage=1",
                "-sDEVICE=pngalpha",
                &resolution,
                "-dTextAlphaBits=4",
                "-dGraphicsAlphaBits=4",
                "-sOutputFile=p1.png",
                "f.ps",
            ],
            &dir.path,
            TOOL_TIMEOUT,
            None,
        );
        if !gs.ok() {
            return Err(Failure::Passing(format!("Ghostscript {gs}")));
        }
        std::fs::read(dir.file("p1.png")).map_err(|_| Failure::Passing("Ghostscript drew nothing".into()))
    }

    /// A ```tikz block, in the terminal's text colour (black too, which a
    /// dark terminal would hide), scaled down to the reply's width and at
    /// most TIKZ_ROWS rows.
    fn tikz(&self, source: &TikzSource) -> Result<ItemResult, Failure> {
        let dir = Workdir::new().map_err(|e| Failure::Passing(format!("renderer could not make a folder: {e}")))?;
        let mut extra = vec![r"\usepackage{tikz}".to_string()];
        extra.extend(source.preamble.iter().cloned());
        let colour = self.colour();
        let black = if colour.is_empty() {
            String::new()
        } else {
            format!(r"\definecolor{{black}}{{HTML}}{{{}}}", self.fg)
        };
        let (cjk_open, cjk_close) = self.cjk(&source.picture);
        let document = format!(
            "{preamble}{cjk_open}\\setlength\\PreviewBorder{{{PICTURE_BORDER_PT}pt}}\\begin{{preview}}{colour}{black}%\n\
             \\setbox0\\hbox{{%\n{picture}%\n}}\\typeout{{TSDIM \\the\\wd0,\\the\\ht0,\\the\\dp0}}\\box0\\end{{preview}}{cjk_close}\n\\end{{document}}\n",
            preamble = self.preamble(&extra),
            picture = source.picture,
        );
        let log = self.latex(&dir, &document, PICTURE_TIMEOUT)?;
        let (wd, ht, dp) = measured(&log).ok_or_else(|| Failure::Refused("TeX made no page".into()))?;
        let (cw, ch) = (f64::from(self.cell.width), f64::from(self.cell.height));
        let pt_px = self.cfg.pt_px;
        let scale = 1f64
            .min((f64::from(self.cfg.max_columns) * cw - 5.0) / (wd * pt_px).max(1.0))
            .min((f64::from(TIKZ_ROWS) * ch - 2.0) / ((ht + dp) * pt_px).max(1.0));
        if scale < MIN_SCALE {
            return Err(Failure::Refused(format!(
                "too large to fit the reply ({wd:.0} x {:.0} pt)",
                ht + dp
            )));
        }
        let page = 2.0 * PICTURE_BORDER_PT;
        let dpi = self.dpi_for(wd + page, ht + dp + page, scale);
        let png = self.ghostscript(&dir, dpi)?;
        let (rgba, width, height) = fit::decode(&png).map_err(Failure::Refused)?;
        let ink = Ink {
            rgba,
            width,
            height,
            depth: 0,
        };
        Ok(fit::fit_block(
            &ink,
            dpi / 72.27 / pt_px,
            self.cfg,
            self.cell,
            TIKZ_ROWS,
        ))
    }
}

/// `\let` each command the user's macros define to `\relax`, which LaTeX
/// takes as not defined, so their \newcommand wins over a package's.
fn user_first(macros: &str) -> String {
    static DEFINES: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"\\(?:newcommand|DeclareMathOperator|DeclarePairedDelimiter(?:XPP|X)?)\s*\*?\s*\{?\s*(\\[A-Za-z]+)")
            .unwrap()
    });
    let mut seen = Vec::new();
    for found in DEFINES.captures_iter(macros) {
        let name = found[1].to_string();
        if !seen.contains(&name) {
            seen.push(name);
        }
    }
    seen.iter().map(|name| format!(r"\let{name}\relax")).collect()
}

/// What `--check` reports about TeX: the tools, tikz-cd, the user's
/// packages TeX lacks, and a test formula through TeX.
pub fn health(cfg: &Options, cell: Cell, fg: &str) -> (Value, Vec<String>) {
    let tools = Tools::find();
    let mut problems = Vec::new();
    let mut tikz_cd = false;
    let mut works = Value::Null;
    if tools.latex {
        tikz_cd = has_package(&cfg.packages, "tikz-cd");
        for name in REQUIRED {
            if !has_package(&cfg.packages, name) {
                problems.push(format!("TeX fallback: LaTeX package {name} not found"));
            }
        }
        for name in &cfg.packages {
            if !has_package(&cfg.packages, name) {
                problems.push(format!("LaTeX package {name} (in packages) not found"));
            }
        }
        if cfg.tex_fallback && tools.dvipng && problems.is_empty() {
            let tex = Tex::new(cfg, cell, fg);
            let probe = tex.formula("x^2", false, false);
            works = Value::Bool(matches!(probe, Ok(ItemResult::Picture { .. })));
            match probe {
                Ok(ItemResult::Failure { error }) | Err(Failure::Refused(error) | Failure::Passing(error)) => {
                    problems.push(format!("TeX fallback test failed: {error}"));
                }
                Ok(ItemResult::Picture { .. }) => {}
            }
        }
    }
    for name in &cfg.bad_packages {
        problems.push(format!("packages: {name} is not a package name"));
    }
    let report = json!({
        "latex": tools.latex,
        "dvipng": tools.dvipng,
        "dvips": tools.dvips,
        "gs": tools.gs,
        "tikzCd": tikz_cd,
        "works": works,
    });
    (report, problems)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::parse_options;

    fn tex_here() -> bool {
        let tools = Tools::find();
        if !(tools.latex && tools.dvipng) {
            eprintln!("note: latex or dvipng not on PATH, TeX test skipped");
            return false;
        }
        true
    }

    fn options() -> Options {
        parse_options(&json!({ "fg": "#000000", "cell": "19x42" }))
    }

    /// The last row with ink at least half opaque, in flat-bottomed glyphs
    /// that sit on the baseline, is the row just above the baseline dvipng
    /// reports, through the preview border and the crop.
    #[test]
    fn the_baseline_is_where_the_glyphs_sit() {
        if !tex_here() {
            return;
        }
        for pt_px in [2.0, 3.4, 4.1] {
            let mut cfg = options();
            cfg.pt_px = pt_px;
            let tex = Tex::new(&cfg, Cell { width: 19, height: 42 }, "#000000");
            for latex in ["HEL", r"\mathrm{T}"] {
                let dir = Workdir::new().unwrap();
                let document = format!(
                    "{}\\setlength\\PreviewBorder{{1pt}}\\begin{{preview}}$\\textstyle {latex}$\\end{{preview}}\n\\end{{document}}\n",
                    tex.preamble(&[])
                );
                tex.latex(&dir, &document, FORMULA_TIMEOUT).unwrap();
                let ink = fit::crop_to_ink(&tex.dvipng(&dir, tex.full_dpi().round()).unwrap());
                let base_row = ink.height as i64 - ink.depth;
                let w = ink.width as usize;
                let last_solid = (0..ink.height as usize)
                    .rev()
                    .find(|&y| (0..w).any(|x| ink.rgba[(y * w + x) * 4 + 3] >= 128))
                    .unwrap() as i64;
                assert_eq!(last_solid, base_row - 1, "{latex} at {pt_px} px/pt");
            }
        }
    }

    /// The same through dvips and Ghostscript, as tikz-cd diagrams are
    /// drawn: the depth computed from TeX's box puts the baseline where the
    /// glyphs sit.
    #[test]
    fn a_diagram_baseline_is_where_the_glyphs_sit() {
        let tools = Tools::find();
        if !tex_here() || !tools.dvips || !tools.gs {
            eprintln!("note: dvips or gs not on PATH, Ghostscript baseline test skipped");
            return;
        }
        for pt_px in [2.0, 3.4, 4.1] {
            let mut cfg = options();
            cfg.pt_px = pt_px;
            let tex = Tex::new(&cfg, Cell { width: 19, height: 42 }, "#000000");
            let dir = Workdir::new().unwrap();
            let document = format!(
                "{}\\setlength\\PreviewBorder{{1pt}}\\begin{{preview}}\\setbox0\\hbox{{$\\textstyle HEL$}}\\typeout{{TSDIM \\the\\wd0,\\the\\ht0,\\the\\dp0}}\\box0\\end{{preview}}\n\\end{{document}}\n",
                tex.preamble(&[])
            );
            let log = tex.latex(&dir, &document, FORMULA_TIMEOUT).unwrap();
            let (_, _, dp) = measured(&log).unwrap();
            let ink = fit::crop_to_ink(&tex.diagram_ink(&dir, tex.full_dpi().round(), dp + 1.0).unwrap());
            let base_row = ink.height as i64 - ink.depth;
            let w = ink.width as usize;
            let last_solid = (0..ink.height as usize)
                .rev()
                .find(|&y| (0..w).any(|x| ink.rgba[(y * w + x) * 4 + 3] >= 128))
                .unwrap() as i64;
            assert_eq!(last_solid, base_row - 1, "at {pt_px} px/pt");
        }
    }

    #[test]
    fn a_tikz_block_takes_libraries_and_some_packages() {
        let user = vec!["tensor".to_string()];
        let parsed = parse_tikz(
            "% a plot\n\\usetikzlibrary{arrows.meta, calc}\n\\usepackage[siunitx]{circuitikz}\n\\draw (0,0) -- (1,1);",
            &user,
        )
        .unwrap();
        assert_eq!(
            parsed.preamble,
            vec![
                r"\usetikzlibrary{arrows.meta, calc}",
                r"\usepackage[siunitx]{circuitikz}"
            ]
        );
        assert_eq!(
            parsed.picture,
            "\\begin{tikzpicture}\n\\draw (0,0) -- (1,1);\n\\end{tikzpicture}"
        );
        let whole = parse_tikz("\\begin{tikzpicture}\\node {a};\\end{tikzpicture}", &user).unwrap();
        assert_eq!(whole.picture, "\\begin{tikzpicture}\\node {a};\\end{tikzpicture}");
        assert!(parse_tikz("\\usepackage{tensor}\n\\draw (0,0);", &user).is_ok());
        let refused = parse_tikz("\\usepackage{shellesc}\n\\draw (0,0);", &user).unwrap_err();
        assert!(refused.contains("shellesc"), "{refused}");
        let late = parse_tikz("\\draw (0,0);\n\\usepackage{pgfplots}", &user).unwrap_err();
        assert!(late.contains("only at the top"), "{late}");
    }

    #[test]
    fn a_pasted_standalone_document_is_unwrapped() {
        let parsed = parse_tikz(
            "\\documentclass[tikz]{standalone}\n\\usepackage{tikz}\n\\begin{document}\n\\begin{tikzpicture}\n\\draw (0,0) circle (1);\n\\end{tikzpicture}\n\\end{document}",
            &[],
        )
        .unwrap();
        assert_eq!(parsed.preamble, vec![r"\usepackage{tikz}"]);
        assert_eq!(
            parsed.picture,
            "\\begin{tikzpicture}\n\\draw (0,0) circle (1);\n\\end{tikzpicture}"
        );
    }

    #[test]
    fn errors_name_the_undefined_command() {
        let log = "! Undefined control sequence.\nl.12 $\\textstyle \\foo\n";
        assert_eq!(first_error(log).unwrap(), r"! Undefined control sequence. \foo");
        assert_eq!(
            user_first(r"\newcommand{\abs}[1]{|#1|}\DeclareMathOperator{\Tr}{Tr}"),
            r"\let\abs\relax\let\Tr\relax"
        );
    }
}
