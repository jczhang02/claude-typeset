//! The font RaTeX draws CJK (and every other character its KaTeX fonts
//! lack) with. RaTeX reads it once, from RATEX_UNICODE_FONT, so this runs
//! before the first formula.

use std::path::Path;
use std::time::Duration;

use crate::detect::quiet_run;

/// Family names in order of preference: a serif face first, as the TeX
/// backend drew CJK text in a Song (serif) face.
const PREFERRED: &[&str] = &[
    "Noto Serif CJK SC",
    "Source Han Serif SC",
    "Noto Serif CJK TC",
    "Source Han Serif TC",
    "Noto Sans CJK SC",
    "Source Han Sans SC",
    "Noto Sans CJK TC",
];

const MAC_FONTS: &[&str] = &[
    "/System/Library/Fonts/Supplemental/Songti.ttc#0",
    "/Library/Fonts/Songti.ttc#0",
    "/System/Library/Fonts/STHeiti Light.ttc#0",
    "/System/Library/Fonts/Hiragino Sans GB.ttc#0",
];

/// A face as RaTeX takes it: `path` or `path#index`.
fn spec_exists(spec: &str) -> bool {
    let path = spec.rsplit_once('#').map_or(spec, |(p, _)| p);
    Path::new(path).is_file()
}

/// The best Regular CJK face fontconfig knows, as `path#index`.
fn fontconfig_choice() -> Option<String> {
    let listing = quiet_run(
        &[
            "fc-list",
            ":lang=zh",
            "-f",
            "%{file}|%{index}|%{family[0]}|%{style[0]}\n",
        ],
        Duration::from_secs(3),
    );
    let mut faces: Vec<(usize, String)> = Vec::new();
    for line in listing.lines() {
        let mut parts = line.split('|');
        let (Some(file), Some(index), Some(family), Some(style)) =
            (parts.next(), parts.next(), parts.next(), parts.next())
        else {
            continue;
        };
        let regular = style.eq_ignore_ascii_case("regular") || style.eq_ignore_ascii_case("book");
        let rank = PREFERRED
            .iter()
            .position(|p| p.eq_ignore_ascii_case(family))
            .unwrap_or(if family.contains("CJK") {
                PREFERRED.len()
            } else {
                PREFERRED.len() + 1
            });
        let rank = rank * 2 + usize::from(!regular);
        faces.push((rank, format!("{file}#{index}")));
    }
    faces.sort();
    faces.into_iter().map(|(_, spec)| spec).find(|spec| spec_exists(spec))
}

/// The CJK font to use: the option when it names a file, else the system's best.
pub fn choose(option: &str) -> Option<String> {
    if !option.is_empty() {
        return spec_exists(option).then(|| option.to_string());
    }
    if cfg!(target_os = "macos") {
        return MAC_FONTS
            .iter()
            .find(|spec| spec_exists(spec))
            .map(|spec| spec.to_string());
    }
    fontconfig_choice()
}

/// Whether `text` has characters only a CJK font draws.
pub fn needs_cjk(text: &str) -> bool {
    text.chars().any(|c| {
        matches!(c as u32,
            0x1100..=0x11FF | 0x2E80..=0x9FFF | 0xA960..=0xA97F | 0xAC00..=0xD7FF
            | 0xF900..=0xFAFF | 0xFE30..=0xFE4F | 0xFF00..=0xFFEF | 0x20000..=0x3FFFF)
    })
}

/// Where the system's choice is kept between runs (asking fontconfig takes
/// about 10 ms, longer than a formula).
fn cache_file() -> Option<std::path::PathBuf> {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| Path::new(&home).join(".cache")))?;
    Some(base.join("typeset").join("cjk-font"))
}

/// The system's choice: the one kept from an earlier run while its file
/// still exists, else asked for again and kept.
fn system_choice() -> Option<String> {
    let file = cache_file();
    if let Some(kept) = file.as_ref().and_then(|f| std::fs::read_to_string(f).ok()) {
        let kept = kept.trim();
        if spec_exists(kept) {
            return Some(kept.to_string());
        }
    }
    let chosen = choose("");
    if let (Some(file), Some(spec)) = (file, &chosen) {
        if let Some(dir) = file.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(file, spec);
    }
    chosen
}

/// Tells RaTeX which font to use; must run before the first formula.
pub fn install(option: &str) -> Option<String> {
    if std::env::var_os("RATEX_UNICODE_FONT").is_some() {
        return std::env::var("RATEX_UNICODE_FONT").ok();
    }
    let chosen = if option.is_empty() {
        system_choice()
    } else {
        choose(option)
    };
    if let Some(spec) = &chosen {
        // SAFETY: called on the main thread before any other thread starts.
        unsafe { std::env::set_var("RATEX_UNICODE_FONT", spec) };
    }
    chosen
}
