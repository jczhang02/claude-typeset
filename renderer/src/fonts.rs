//! The font RaTeX draws CJK (and every other character its KaTeX fonts
//! lack) with. RaTeX reads it once, from RATEX_UNICODE_FONT, so this runs
//! before the first formula.

use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};
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

/// Characters one of which a CJK face has: Chinese, Japanese kana, Hangul.
const PROBES: &[char] = &['中', 'あ', '한'];

/// The file of a face as RaTeX takes it (`path` or `path#index`), read, and
/// its index; the last one read is kept, as a CJK font is large.
fn read_face(spec: &str) -> Option<Arc<(Vec<u8>, u32)>> {
    static LAST: Mutex<Option<(String, Option<Arc<(Vec<u8>, u32)>>)>> = Mutex::new(None);
    let mut last = LAST.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((kept, face)) = last.as_ref()
        && kept == spec
    {
        return face.clone();
    }
    let (path, index) = match spec.rsplit_once('#') {
        Some((path, index)) => (path, index.parse().ok()?),
        None => (spec, 0),
    };
    let face = Path::new(path)
        .is_file()
        .then(|| std::fs::read(path).ok())
        .flatten()
        .map(|bytes| Arc::new((bytes, index)));
    *last = Some((spec.to_string(), face.clone()));
    face
}

/// Whether the face `spec` names has a glyph for each of `chars`.
fn has_glyphs(spec: &str, chars: &mut dyn Iterator<Item = char>, any: bool) -> bool {
    let Some(face) = read_face(spec) else {
        return false;
    };
    let Ok(parsed) = ttf_parser::Face::parse(&face.0, face.1) else {
        return false;
    };
    let mut found = chars.map(|c| parsed.glyph_index(c).is_some());
    if any {
        found.any(|has| has)
    } else {
        found.all(|has| has)
    }
}

/// Whether `spec` names a face that draws CJK: a font file, a face index
/// it has, and CJK glyphs in it (not a Latin font, which would leave the
/// text out of the picture without a word).
fn spec_exists(spec: &str) -> bool {
    has_glyphs(spec, &mut PROBES.iter().copied(), true)
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
    Some(crate::cache::base()?.join("cjk-font"))
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
    if let (Some(file), Some(spec)) = (file, &chosen)
        && file.parent().is_some_and(crate::cache::private_dir)
    {
        let _ = crate::cache::write_private(&file, spec.as_bytes());
    }
    chosen
}

/// The font install chose, once it ran.
static INSTALLED: OnceLock<Option<String>> = OnceLock::new();

/// Tells RaTeX which font to use; must run before the first formula.
pub fn install(option: &str) -> Option<String> {
    INSTALLED
        .get_or_init(|| {
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
        })
        .clone()
}

/// A CJK character of `text` the installed font cannot draw, if any: RaTeX
/// would leave it out of the picture without a word, so the formula is
/// better shown as text (or drawn by TeX).
pub fn missing(text: &str) -> Option<char> {
    let mut cjk = text.chars().filter(|&c| needs_cjk(&c.to_string()));
    let first = cjk.clone().next()?;
    match INSTALLED.get().cloned().flatten() {
        Some(spec) => cjk.find(|&c| !has_glyphs(&spec, &mut std::iter::once(c), false)),
        None => Some(first),
    }
}
