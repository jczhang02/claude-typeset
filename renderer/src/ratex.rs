//! A formula typeset by RaTeX: its ink at SUPERSAMPLE times the terminal's
//! pixel size, in the formula's colour, and where its baseline is.

use std::sync::LazyLock;

use ratex_layout::{LayoutOptions, layout, to_display_list, validate_layout};
use ratex_parser::parser::parse;
use ratex_render::{RenderOptions, render_to_png};
use ratex_types::color::Color;
use ratex_types::math_style::MathStyle;
use regex::Regex;

use crate::fit::{Ink, SUPERSAMPLE};

/// The TeX backend's body size, in TeX points (article, 10pt).
const BODY_PT: f64 = 10.0;

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

/// What goes before every formula: the built-in definitions, then the
/// user's, as RaTeX takes them. The user's are meant to define their names
/// whatever exists already (RaTeX's `\R`, our `\abs`), and
/// \DeclareMathOperator, which KaTeX lacks, becomes \operatorname.
pub fn user_macros(macros: &str) -> String {
    let macros = OPERATOR.replace_all(macros, |c: &regex::Captures| {
        format!(r"\newcommand{{{}}}{{\operatorname{}{{{}}}}}", &c[2], &c[1], &c[3])
    });
    let user = DEFINITION.replace_all(&macros, |c: &regex::Captures| {
        let command = if is_defined(&c[3]) {
            "renewcommand"
        } else {
            "newcommand"
        };
        format!(r"\{command}{}{}\{}", &c[1], &c[2], &c[3])
    });
    let builtin = builtin_macros();
    match (builtin.is_empty(), user.trim().is_empty()) {
        (true, _) => user.into_owned(),
        (false, true) => builtin.to_string(),
        (false, false) => format!("{builtin}\n{user}"),
    }
}

/// `latex` (after the user's `macros`, already through `user_macros`) typeset in `fg`, or why not.
pub fn typeset(latex: &str, display: bool, fg: [f32; 3], pt_px: f64, macros: &str) -> Result<Ink, String> {
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
            POSITION.replace(&text, "").into_owned()
        }
    })?;
    let laid = layout(&ast, &options);
    validate_layout(&laid)?;
    let list = to_display_list(&laid);
    let em = em_px(pt_px);
    let render = RenderOptions {
        font_size: em as f32,
        padding: 0.0,
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
    // The pixmap's top is the top of the box: the baseline is `height` em down.
    let base_row = ((list.height * em) as f32).round().clamp(0.0, height as f32) as u32;

    Ok(Ink {
        rgba,
        width,
        height,
        depth: height as i64 - i64::from(base_row),
    })
}

#[cfg(test)]
mod tests {
    use super::typeset;

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
