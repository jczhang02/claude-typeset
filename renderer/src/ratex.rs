//! A formula typeset by RaTeX: its ink at SUPERSAMPLE times the terminal's
//! pixel size, in the formula's colour, and where its baseline is.

use std::sync::LazyLock;

use ratex_layout::{LayoutOptions, layout, to_display_list, validate_layout};
use ratex_parser::parser::parse;
use ratex_render::{RenderOptions, render_to_png};
use ratex_types::color::Color;
use ratex_types::display_item::DisplayList;
use ratex_types::math_style::MathStyle;
use regex::Regex;

use crate::fit::{Ink, SUPERSAMPLE};
use crate::tex::OVERHANG;

/// The TeX backend's body size, in TeX points (article, 10pt).
const BODY_PT: f64 = 10.0;

/// The margin drawn around the box for ink that stands out of it, in em:
/// a glyph's overshoot, a CJK stroke or an emoji from the Unicode font,
/// which has no KaTeX metrics, below the depth.
const MARGIN_EM: f64 = 0.5;
/// The margin for a formula built to put ink outside its box (\mathclap,
/// \llap, a negative kern): more than the old backend's 60 pt border.
const OVERHANG_EM: f64 = 10.0;
/// The most pixels RaTeX rasters (its MAX_PNG_PIXELS), less a little for its
/// rounding: a margin never makes a formula too large to draw.
const RASTER_PIXELS: f64 = 0.95 * 16_777_216.0;

/// RaTeX's em in raster pixels for `pt_px` terminal pixels per TeX point.
pub fn em_px(pt_px: f64) -> f64 {
    BODY_PT * pt_px * f64::from(SUPERSAMPLE)
}

static DEFINITION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\\(?:re)?newcommand(\*?)(\s*\{?\s*)\\([A-Za-z]+|.)").unwrap());
static POSITION: LazyLock<Regex> = LazyLock::new(|| Regex::new(r" at position \d+").unwrap());
static OPERATOR: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\\DeclareMathOperator(\*?)\s*\{?\s*(\\[A-Za-z]+)\s*\}?\s*\{((?:[^{}]|\{[^{}]*\})*)\}").unwrap()
});

/// Commands Claude writes often that RaTeX lacks (from physics, mathtools
/// and the like), each defined only where RaTeX does not have it. Only
/// forms that cannot draw something other than what was meant: no
/// \pdv{f}{x}{y} or \dv{x}, whose arguments a macro would take wrongly
/// and draw as something else without a word.
const BUILTIN: &[(&str, &str)] = &[
    ("abs", r"\newcommand{\abs}[1]{\left\lvert #1\right\rvert}"),
    ("norm", r"\newcommand{\norm}[1]{\left\lVert #1\right\rVert}"),
    ("floor", r"\newcommand{\floor}[1]{\left\lfloor #1\right\rfloor}"),
    ("ceil", r"\newcommand{\ceil}[1]{\left\lceil #1\right\rceil}"),
    ("Q", r"\newcommand{\Q}{\mathbb{Q}}"),
    ("C", r"\newcommand{\C}{\mathbb{C}}"),
    ("sgn", r"\newcommand{\sgn}{\operatorname{sgn}}"),
    ("diag", r"\newcommand{\diag}{\operatorname{diag}}"),
    ("mbox", r"\newcommand{\mbox}[1]{\text{#1}}"),
];

/// Whether RaTeX itself already knows `\name` (a \newcommand of it would fail).
fn is_known(name: &str) -> bool {
    parse(&format!(r"\newcommand{{\{name}}}{{x}}x")).is_err_and(|e| e.to_string().contains("redefine"))
}

/// The built-in definitions RaTeX needs (those it lacks), once per run.
fn builtin_macros() -> &'static str {
    static DEFINED: LazyLock<String> = LazyLock::new(|| {
        BUILTIN
            .iter()
            .filter(|(name, _)| !is_known(name))
            .map(|(_, def)| *def)
            .collect::<Vec<_>>()
            .join("\n")
    });
    &DEFINED
}

/// Whether `\name` exists before the user's definitions: RaTeX's or ours.
fn is_defined(name: &str) -> bool {
    BUILTIN.iter().any(|(n, _)| *n == name) || is_known(name)
}

/// A definition command, or a prefix of one, at the start of the text.
static DEFINER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^\\(?:(?:re)?newcommand|providecommand|[gex]?def|let|DeclareMathOperator|DeclarePairedDelimiter[A-Za-z]*|(?:re)?newenvironment|global|long|protected)(?:[^A-Za-z]|$)",
    )
    .unwrap()
});
static PREFIX_END: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\\(?:global|long|protected)\s*$").unwrap());
/// The name a definition defines.
static DEFINED_NAME: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\\(?:(?:re)?newcommand|providecommand|DeclareMathOperator|DeclarePairedDelimiter[A-Za-z]*|[gex]?def|let)\*?\s*\{?\s*\\([A-Za-z]+)").unwrap()
});
/// What a name whose definition RaTeX cannot take stands for: a command
/// RaTeX does not know, so a formula that uses the name fails (and goes to
/// TeX) rather than drawing RaTeX's own command of that name or ours.
const REFUSED: &str = "typesetrefused";
static REFUSED_USE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"Undefined control sequence: \\typesetrefused([A-Za-z]+)").unwrap());

/// The user's macros cut into definitions: each piece starts at a
/// definition command outside any group (text before the first, such as a
/// comment, goes with it).
fn definitions(macros: &str) -> Vec<&str> {
    let mut pieces = Vec::new();
    let (mut start, mut depth) = (0usize, 0i64);
    let mut chars = macros.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        match c {
            '\\' => {
                if depth == 0
                    && DEFINER.is_match(&macros[i..])
                    && !macros[start..i].trim().is_empty()
                    && !PREFIX_END.is_match(&macros[start..i])
                {
                    pieces.push(&macros[start..i]);
                    start = i;
                }
                chars.next(); // the escaped character: \{ opens no group
            }
            '{' => depth += 1,
            '}' => depth -= 1,
            '%' => while chars.next_if(|&(_, c)| c != '\n').is_some() {},
            _ => {}
        }
    }
    if !macros[start..].trim().is_empty() {
        pieces.push(&macros[start..]);
    }
    pieces
}

/// One definition as RaTeX takes it: \DeclareMathOperator becomes
/// \operatorname, which KaTeX lacks, and a name that exists already (RaTeX's
/// `\R`, our `\abs`) is redefined. The star of \newcommand* (no paragraph
/// in an argument) means nothing in math, and RaTeX would read it as the
/// name being defined: dropped.
fn for_ratex(definition: &str) -> String {
    let definition = OPERATOR.replace_all(definition, |c: &regex::Captures| {
        format!(r"\newcommand{{{}}}{{\operatorname{}{{{}}}}}", &c[2], &c[1], &c[3])
    });
    DEFINITION
        .replace_all(&definition, |c: &regex::Captures| {
            let command = if is_defined(&c[3]) {
                "renewcommand"
            } else {
                "newcommand"
            };
            format!(r"\{command}{}\{}", &c[2], &c[3])
        })
        .into_owned()
}

/// The user's macros as RaTeX takes them: what goes before every formula
/// (the built-in definitions, then the user's), and each user definition
/// RaTeX cannot take, with why. One RaTeX refuses (\newenvironment,
/// \DeclarePairedDelimiter) or reads only in part (an optional argument's
/// default, `[2][x]`, which it would draw as text before every formula) is
/// left out here: a formula that uses it fails in RaTeX and goes to TeX,
/// which has the user's macros whole.
pub fn user_macros(macros: &str) -> (String, Vec<String>) {
    let builtin = builtin_macros();
    let mut taken = builtin.to_string();
    let mut refused = Vec::new();
    for definition in definitions(macros) {
        let converted = for_ratex(definition);
        let candidate = if taken.is_empty() {
            converted
        } else {
            format!("{taken}\n{converted}")
        };
        let why = match parse(&candidate) {
            Ok(nodes) if nodes.is_empty() => {
                taken = candidate;
                continue;
            }
            Ok(_) => "RaTeX would draw part of it as text (an optional argument?)".to_string(),
            Err(e) => POSITION.replace(&e.to_string(), "").into_owned(),
        };
        if let Some(name) = DEFINED_NAME.captures(definition).map(|c| c[1].to_string()) {
            let stand_in = format!(r"\let\{name}\{REFUSED}{name}");
            if parse(&format!("{taken}\n{stand_in}")).is_ok() {
                taken = format!("{taken}\n{stand_in}");
            }
        }
        let head = definition.trim().lines().next().unwrap_or("");
        let head = if head.chars().count() > 60 {
            format!("{}…", head.chars().take(59).collect::<String>())
        } else {
            head.to_string()
        };
        refused.push(format!("{head} is left to TeX: {why}"));
    }
    (taken, refused)
}

/// The margin around a box of `w` x `h` pixels: `wanted`, or as much of it
/// as keeps the raster within RASTER_PIXELS (none for a box past it).
fn margin_for(w: f64, h: f64, wanted: f64) -> f64 {
    // (w + 2p)(h + 2p) <= RASTER_PIXELS, for the largest p.
    let sum = w + h;
    let room = sum * sum - 4.0 * (w * h - RASTER_PIXELS);
    let most = if room > 0.0 { (room.sqrt() - sum) / 4.0 } else { 0.0 };
    most.clamp(0.0, wanted).floor()
}

/// `latex` (after the user's `macros`, already through `user_macros`) laid
/// out by RaTeX, ready to draw.
fn display_list(latex: &str, display: bool, fg: [f32; 3], macros: &str) -> Result<DisplayList, String> {
    let source = if macros.is_empty() {
        latex.to_string()
    } else {
        format!("{macros}\n{latex}")
    };
    let style = if display { MathStyle::Display } else { MathStyle::Text };
    let colour = Color {
        r: fg[0],
        g: fg[1],
        b: fg[2],
        a: 1.0,
    };
    let options = LayoutOptions::default().with_style(style).with_color(colour);
    // A position would count the definitions before the formula: dropped.
    let ast = parse(&source).map_err(|e| {
        let text = e.to_string();
        if macros.is_empty() {
            text
        } else {
            let text = REFUSED_USE.replace(&text, r"\$1 is defined in the macros option in a way RaTeX cannot read");
            POSITION.replace(&text, "").into_owned()
        }
    })?;
    let laid = layout(&ast, &options);
    validate_layout(&laid)?;
    Ok(to_display_list(&laid))
}

/// `latex` (after the user's `macros`, already through `user_macros`) typeset in `fg`, or why not.
pub fn typeset(latex: &str, display: bool, fg: [f32; 3], pt_px: f64, macros: &str) -> Result<Ink, String> {
    let list = display_list(latex, display, fg, macros)?;
    let em = em_px(pt_px);
    // Ink may stand out of the box: a margin keeps it, and the cut below
    // takes the box and every inked pixel around it.
    let wanted = if OVERHANG.is_match(latex) {
        OVERHANG_EM
    } else {
        MARGIN_EM
    };
    let pad = margin_for(list.width * em, (list.height + list.depth) * em, wanted * em);
    let render = RenderOptions {
        font_size: em as f32,
        padding: pad as f32,
        background_color: Color {
            r: 0.0,
            g: 0.0,
            b: 0.0,
            a: 0.0,
        },
        ..Default::default()
    };
    let png = render_to_png(&list, &render)?;
    let (rgba, width, height) = crate::fit::decode(&png)?;
    let (w, h) = (width as usize, height as usize);
    let margin = (pad as usize).min(w - 1).min(h - 1);
    let box_w = ((list.width * em).ceil() as usize).clamp(1, w - margin);
    let box_h = (((list.height + list.depth) * em).ceil() as usize).clamp(1, h - margin);
    // The box, grown to every inked pixel.
    let (mut left, mut top, mut right, mut bottom) = (margin, margin, margin + box_w, margin + box_h);
    for y in 0..h {
        let row = &rgba[y * w * 4..(y + 1) * w * 4];
        let Some(first) = (0..w).find(|&x| row[x * 4 + 3] != 0) else {
            continue;
        };
        let last = (0..w).rev().find(|&x| row[x * 4 + 3] != 0).unwrap_or(first);
        left = left.min(first);
        right = right.max(last + 1);
        top = top.min(y);
        bottom = bottom.max(y + 1);
    }
    let (cut_w, cut_h) = (right - left, bottom - top);
    let mut cut = Vec::with_capacity(cut_w * cut_h * 4);
    for y in top..bottom {
        cut.extend_from_slice(&rgba[(y * w + left) * 4..(y * w + right) * 4]);
    }
    // The box's top is `pad` down the raster, the baseline `height` em below it.
    let base_row = (list.height * em + pad - top as f64).round().clamp(0.0, cut_h as f64) as i64;

    Ok(Ink {
        rgba: cut,
        width: cut_w as u32,
        height: cut_h as u32,
        depth: cut_h as i64 - base_row,
    })
}

#[cfg(test)]
mod tests {
    use super::{em_px, typeset};
    use ratex_layout::{LayoutOptions, layout, to_display_list};
    use ratex_parser::parser::parse;
    use ratex_types::math_style::MathStyle;

    /// RaTeX's box of `latex` in raster pixels: (width, height + depth).
    fn box_px(latex: &str, display: bool, pt_px: f64) -> (f64, f64) {
        let style = if display { MathStyle::Display } else { MathStyle::Text };
        let list = to_display_list(&layout(
            &parse(latex).unwrap(),
            &LayoutOptions::default().with_style(style),
        ));
        let em = em_px(pt_px);
        (list.width * em, (list.height + list.depth) * em)
    }

    /// The margin for ink outside the box never makes a formula too large
    /// to draw: a tall block that fits RaTeX's raster without it draws.
    #[test]
    fn a_tall_block_draws_whatever_its_margin() {
        let rows: Vec<String> = (0..24)
            .map(|i| format!(r"f_{{{i}}}(x) &= \int_0^{{{i}}} \frac{{x^{{{i}}}}}{{1+t^2}}\,dt + \sum_{{k=0}}^{{n}} \binom{{n}}{{k}} a_k"))
            .collect();
        let block = format!(r"\begin{{aligned}}{}\end{{aligned}}", rows.join(r" \\ "));
        let (w, h) = box_px(&block, true, 3.4);
        // It fits alone, not with 2 em on each side (as 913aee9 drew it).
        assert!(w * h < 16_777_216.0, "the box alone fits: {w} x {h}");
        assert!((w + 408.0) * (h + 408.0) > 16_777_216.0, "{w} x {h}");
        typeset(&block, true, [0.0, 0.0, 0.0], 3.4, "").unwrap();
        let wide = format!(r"\llap{{\text{{a}}}}{block}");
        typeset(&wide, true, [0.0, 0.0, 0.0], 3.4, "").unwrap();
    }

    /// \llap'd text far wider than a few em keeps its first letters.
    #[test]
    fn ink_far_beside_the_box_is_kept() {
        let label = typeset(r"\text{long label here}", true, [0.0, 0.0, 0.0], 8.0, "").unwrap();
        let lapped = typeset(r"\llap{\text{long label here}}x", true, [0.0, 0.0, 0.0], 8.0, "").unwrap();
        assert!(
            lapped.width > label.width,
            "{} px for the lapped label and x, {} px for the label alone",
            lapped.width,
            label.width
        );
    }

    /// Ink below RaTeX's box (a script letter's tail, a glyph of the
    /// Unicode font, which has no KaTeX metrics) is kept, not cut at the box.
    #[test]
    fn ink_below_the_box_is_kept() {
        for latex in [r"\mathcal{F}", r"\mathscr{F}", r"\not="] {
            let (_, h) = box_px(latex, false, 3.4);
            let ink = typeset(latex, false, [0.0, 0.0, 0.0], 3.4, "").unwrap();
            assert!(
                f64::from(ink.height) > h.ceil(),
                "{latex}: {} rows of ink, box {h}",
                ink.height
            );
        }
    }

    /// The last row with ink at least half opaque, in a flat-bottomed
    /// glyph that sits on the baseline, is the row just above `base_row`:
    /// the baseline `fit` computes is where RaTeX drew it.
    #[test]
    fn the_baseline_is_where_the_glyphs_sit() {
        for pt_px in [2.0, 3.4, 4.1] {
            for latex in ["H", "E", r"\mathrm{T}", "HEL"] {
                let ink = typeset(latex, false, [0.0, 0.0, 0.0], pt_px, "").unwrap();
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
}
