//! What a formula is turned into before an engine sees it, whichever engine:
//! a terminal shows one formula at a time, so a whole `align` or `equation`
//! becomes its inner form, equation numbers go, and a display formula with
//! top-level `\\` becomes one block of lines. Ported from the Python
//! backend's `prepare()` (its engine-independent part).

use std::sync::LazyLock;

use regex::Regex;

/// What each whole environment becomes (`None`: its body alone).
fn whole_env(name: &str) -> Option<Option<&'static str>> {
    Some(match name {
        "align" | "align*" | "flalign" | "flalign*" | "eqnarray" | "eqnarray*" | "split" => Some("aligned"),
        "gather" | "gather*" | "multline" | "multline*" => Some("gathered"),
        "alignat" | "alignat*" => Some("alignedat"),
        "equation" | "equation*" | "displaymath" | "math" => None,
        "numcases" | "subnumcases" => Some("cases"),
        "empheq" => Some("empheq"),
        _ => return None,
    })
}

static BEGIN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*\\begin\s*\{([A-Za-z]+\*?)\}").unwrap());
// What may follow a whole environment and still leave it whole: the sentence
// punctuation the hooks append to a display formula ("\,{.}", "\text{。}").
static TRAILING: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*(?:\\,\{[^{}]*\}|\\text\{[^{}]*\})?\s*$").unwrap());
static NUMCASES_LHS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*\{((?:[^{}]|\{[^{}]*\})*)\}").unwrap());
static EMPHEQ_HEAD: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*(?:\[((?:[^\[\]]|\{[^{}]*\})*)\])?\s*\{([A-Za-z]+\*?)\}").unwrap());
static ALIGNAT_COUNT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*(\{\d+\})").unwrap());
static INTERTEXT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?:\\\\\s*)?\\(?:short)?intertext\s*\{((?:[^{}]|\{[^{}]*\})*)\}\s*").unwrap());
static LABEL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\\label\s*\{[^{}]*\}").unwrap());
static NONUMBER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\\(?:nonumber|notag)([^A-Za-z]|$)").unwrap());
static TAG_STAR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\\tag\*\s*\{([^{}]*)\}").unwrap());
static TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\\tag\s*\{([^{}]*)\}").unwrap());
static BLANK_LINES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\n[ \t]*(?:\n[ \t]*)+").unwrap());

/// (name, body, tail) when all of `latex` is one \begin{name}...\end{name}
/// (then perhaps trailing punctuation, the tail).
fn whole_environment(latex: &str) -> Option<(String, String, String)> {
    let start = BEGIN.captures(latex)?;
    let name = start.get(1)?.as_str().to_string();
    let open_end = start.get(0)?.end();
    let edge = Regex::new(&format!(r"\\(begin|end)\s*\{{{}\}}", regex::escape(&name))).ok()?;
    let mut depth = 0;
    for found in edge.captures_iter(latex) {
        depth += if &found[1] == "begin" { 1 } else { -1 };
        if depth == 0 {
            let whole = found.get(0)?;
            let tail = &latex[whole.end()..];
            if !TRAILING.is_match(tail) {
                return None;
            }
            return Some((
                name,
                latex[open_end..whole.start()].to_string(),
                tail.trim().to_string(),
            ));
        }
    }
    None
}

/// What empheq's left= or right= option puts beside the equations, as \left/\right.
fn empheq_side(options: &str, side: &str) -> String {
    let pattern = Regex::new(&format!(r"{side}\s*=\s*(\{{(?:[^{{}}]|\{{[^{{}}]*\}})*\}}|[^,]*)")).unwrap();
    let mut value = pattern
        .captures(options)
        .map(|c| c[1].trim().to_string())
        .unwrap_or_default();
    if value.starts_with('{') && value.ends_with('}') && value.len() >= 2 {
        value = value[1..value.len() - 1].to_string();
    }
    let word = if side == "left" { "left" } else { "right" };
    let delim = Regex::new(r"\\empheq(?:big)?([lr][A-Za-z]+)").unwrap();
    let mut count = 0;
    let replaced = delim.replace_all(&value, |c: &regex::Captures| {
        count += 1;
        let d = match &c[1] {
            "lbrace" => r"\{",
            "rbrace" => r"\}",
            "lbrack" => "[",
            "rbrack" => "]",
            "langle" => r"\langle",
            "rangle" => r"\rangle",
            "lparen" => "(",
            "rparen" => ")",
            "lvert" => r"\lvert",
            "rvert" => r"\rvert",
            "lVert" => r"\lVert",
            "rVert" => r"\rVert",
            _ => ".",
        };
        format!(r"\{word}{d}")
    });
    let replaced = replaced.into_owned();
    if count > 0 {
        replaced
    } else if side == "left" {
        format!(r"{replaced}\left.")
    } else {
        format!(r"\right.{replaced}")
    }
}

fn unwrap_once(latex: &str) -> String {
    let Some((name, body, tail)) = whole_environment(latex) else {
        return latex.to_string();
    };
    let Some(inner) = whole_env(&name) else {
        return latex.to_string();
    };
    let Some(inner) = inner else {
        return format!("{}{tail}", body.trim());
    };
    match inner {
        "cases" => {
            let Some(lhs) = NUMCASES_LHS.captures(&body) else {
                return latex.to_string();
            };
            let end = lhs.get(0).map_or(0, |m| m.end());
            format!(r"{}\begin{{cases}}{}\end{{cases}}{tail}", &lhs[1], &body[end..])
        }
        "empheq" => {
            let Some(head) = EMPHEQ_HEAD.captures(&body) else {
                return latex.to_string();
            };
            let env = head[2].to_string();
            if !matches!(whole_env(&env), Some(Some(i)) if i != "cases" && i != "empheq") {
                return latex.to_string();
            }
            let options = head.get(1).map_or("", |m| m.as_str()).to_string();
            let end = head.get(0).map_or(0, |m| m.end());
            let mut inside = unwrap_once(&format!(r"\begin{{{env}}}{}\end{{{env}}}", &body[end..]));
            if options.contains("box") {
                inside = format!(r"\boxed{{{inside}}}");
            }
            if options.contains("left") || options.contains("right") {
                inside = format!(
                    "{}{inside}{}",
                    empheq_side(&options, "left"),
                    empheq_side(&options, "right")
                );
            }
            format!("{inside}{tail}")
        }
        _ => {
            if (inner == "aligned" || inner == "alignedat")
                && let Some(parts) = interjections(&body) {
                    // \intertext: blocks one under the other.
                    let blocks: Vec<String> = parts
                        .iter()
                        .enumerate()
                        .map(|(k, part)| {
                            if k % 2 == 0 {
                                format!(r"\begin{{{inner}}}{part}\end{{{inner}}}")
                            } else {
                                format!(r"\text{{{part}}}")
                            }
                        })
                        .collect();
                    return format!(r"\begin{{array}}{{l}}{}\end{{array}}{tail}", blocks.join(r"\\"));
                }
            if inner == "alignedat" {
                let Some(count) = ALIGNAT_COUNT.captures(&body) else {
                    return latex.to_string();
                };
                let end = count.get(0).map_or(0, |m| m.end());
                return format!(
                    r"\begin{{alignedat}}{}{}\end{{alignedat}}{tail}",
                    &count[1],
                    &body[end..]
                );
            }
            format!(r"\begin{{{inner}}}{body}\end{{{inner}}}{tail}")
        }
    }
}

/// body split at its \intertext{text} and \shortintertext{text}: lines, text, lines, ...
fn interjections(body: &str) -> Option<Vec<String>> {
    let mut parts = Vec::new();
    let mut start = 0;
    for found in INTERTEXT.captures_iter(body) {
        let whole = found.get(0)?;
        parts.push(body[start..whole.start()].to_string());
        parts.push(found[1].to_string());
        start = whole.end();
    }
    if parts.is_empty() {
        return None;
    }
    parts.push(body[start..].to_string());
    Some(parts)
}

/// A whole align, gather, equation (also nested, as equation > split) as its inner form.
fn unwrap_environment(latex: &str) -> String {
    let mut latex = latex.to_string();
    for _ in 0..4 {
        let unwrapped = unwrap_once(&latex);
        if unwrapped == latex {
            break;
        }
        latex = unwrapped;
    }
    latex
}

/// Drop \label, \nonumber, \notag; draw \tag{x} as a plain (x).
fn strip_numbering(latex: &str) -> String {
    let latex = LABEL.replace_all(latex, "");
    let latex = NONUMBER.replace_all(&latex, "$1");
    let latex = TAG_STAR.replace_all(&latex, r"\qquad\text{$1}");
    TAG.replace_all(&latex, r"\qquad\text{($1)}").into_owned()
}

/// Drop TeX comments as TeX does: from an unescaped % through the line
/// break and the next line's leading blanks.
fn strip_comments(latex: &str) -> String {
    let mut out = String::with_capacity(latex.len());
    let mut chars = latex.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            out.push(c);
            if let Some(next) = chars.next() {
                out.push(next);
            }
        } else if c == '%' {
            for skipped in chars.by_ref() {
                if skipped == '\n' {
                    break;
                }
            }
            while matches!(chars.peek(), Some(' ' | '\t')) {
                chars.next();
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// A display formula with top-level \\ becomes gathered (or aligned with &).
fn stack_lines(latex: &str) -> String {
    let bytes: Vec<char> = latex.chars().collect();
    let (mut depth, mut envs, mut breaks, mut amps) = (0i32, 0i32, false, false);
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if c == '\\' && i + 1 < bytes.len() {
            let mut j = i + 1;
            if bytes[j].is_ascii_alphabetic() {
                while j < bytes.len() && bytes[j].is_ascii_alphabetic() {
                    j += 1;
                }
            } else {
                j += 1;
            }
            let name: String = bytes[i + 1..j].iter().collect();
            match name.as_str() {
                "begin" => envs += 1,
                "end" => envs -= 1,
                "\\" if depth == 0 && envs == 0 => breaks = true,
                _ => {}
            }
            i = j;
            continue;
        }
        match c {
            '{' => depth += 1,
            '}' => depth -= 1,
            '&' if depth == 0 && envs == 0 => amps = true,
            _ => {}
        }
        i += 1;
    }
    if !breaks {
        return latex.to_string();
    }
    let inner = if amps { "aligned" } else { "gathered" };
    format!(r"\begin{{{inner}}}{latex}\end{{{inner}}}")
}

/// The formula as an engine should see it.
pub fn normalize(latex: &str, display: bool) -> String {
    let latex = strip_comments(&latex.replace('\r', ""));
    let latex = BLANK_LINES.replace_all(&latex, "\n");
    let latex = strip_numbering(&unwrap_environment(latex.trim()));
    if display { stack_lines(&latex) } else { latex }
}

#[cfg(test)]
mod tests {
    use super::normalize;

    #[test]
    fn a_whole_align_becomes_aligned_without_numbers() {
        assert_eq!(
            normalize(r"\begin{align} a &= b \label{x} \\ c &= d \tag{2} \end{align}", true),
            r"\begin{aligned} a &= b  \\ c &= d \qquad\text{(2)} \end{aligned}"
        );
    }

    #[test]
    fn an_equation_keeps_its_body_and_trailing_punctuation() {
        assert_eq!(
            normalize(r"\begin{equation}E=mc^2\end{equation}\,{.}", true),
            r"E=mc^2\,{.}"
        );
    }

    #[test]
    fn display_lines_are_stacked() {
        assert_eq!(
            normalize(r"a=b \\ c=d", true),
            r"\begin{gathered}a=b \\ c=d\end{gathered}"
        );
        assert_eq!(
            normalize(r"a&=b \\ c&=d", true),
            r"\begin{aligned}a&=b \\ c&=d\end{aligned}"
        );
        assert_eq!(
            normalize(r"\begin{matrix}a\\b\end{matrix}", true),
            r"\begin{matrix}a\\b\end{matrix}"
        );
    }

    #[test]
    fn comments_go() {
        assert_eq!(normalize("a % note\n  + b", false), "a + b");
        assert_eq!(normalize(r"50\% + x", false), r"50\% + x");
    }

    #[test]
    fn intertext_splits_an_align() {
        let out = normalize(r"\begin{align*} a &= b \intertext{so} c &= d \end{align*}", true);
        assert!(
            out.starts_with(r"\begin{array}{l}\begin{aligned} a &= b \end{aligned}\\\text{so}"),
            "{out}"
        );
    }
}
