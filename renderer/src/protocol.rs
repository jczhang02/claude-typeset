//! The request the hooks send on stdin and the answer printed on stdout.
//!
//! Request:
//!     {"items": [{"latex": str, "display": bool}, ...],
//!      "options": {"fg": "auto" | "#rrggbb", "cell": "auto" | "WxH",
//!                  "baseline": 0.773, "ptPx": 3.4, "minShrink": 0.8,
//!                  "thicken": 1.8, "maxColumns": 255, "cjkFont": str,
//!                  "packages": [str, ...], "macros": str}}
//!
//! Answer (results in item order; the exit status is 0 even when items fail):
//!     {"cell": {"width", "height"}, "fg": "#rrggbb",
//!      "results": [{"png", "columns", "rows", "textRow"} | {"error"}, ...]}

use serde::Serialize;
use serde_json::Value;

pub const MAX_BOX: u32 = 255; // columns and rows of an Image element
pub const MIN_SCALE: f64 = 0.25; // smaller than this and a formula is "too large"
pub const MAX_LATEX: usize = 4000;
const MAX_USER_PACKAGES: usize = 64;
const MAX_MACROS: usize = 64 * 1024;

#[derive(Debug, Clone)]
pub struct Options {
    pub fg: String,
    pub cell: String,
    pub baseline: f64,
    pub pt_px: f64,
    pub min_shrink: f64,
    pub thicken: f64,
    pub max_columns: u32,
    pub cjk_font: String,
    #[allow(dead_code)] // for the TeX fallback
    pub packages: Vec<String>,
    #[allow(dead_code)] // for the TeX fallback
    pub bad_packages: Vec<String>,
    pub macros: String,
}

#[derive(Debug, Clone)]
pub struct Item {
    pub latex: String,
    pub display: bool,
}

fn number(raw: &Value, name: &str, default: f64, low: f64, high: f64) -> f64 {
    match raw.get(name) {
        Some(Value::Number(n)) => match n.as_f64() {
            Some(v) if (low..=high).contains(&v) => v,
            _ => default,
        },
        _ => default,
    }
}

fn is_package_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= 64 && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

/// The request options, with defaults for missing or bad values.
pub fn parse_options(raw: &Value) -> Options {
    let empty = Value::Object(Default::default());
    let raw = if raw.is_object() { raw } else { &empty };
    let text = |name: &str, default: &str| raw.get(name).and_then(Value::as_str).unwrap_or(default).to_string();
    let mut packages = Vec::new();
    let mut bad_packages = Vec::new();
    if let Some(Value::Array(names)) = raw.get("packages") {
        for name in names {
            match name.as_str().map(str::trim) {
                Some(n) if is_package_name(n) && packages.len() < MAX_USER_PACKAGES => {
                    if !packages.iter().any(|p| p == n) {
                        packages.push(n.to_string());
                    }
                }
                _ => bad_packages.push(name.to_string().chars().take(40).collect()),
            }
        }
    }
    let macros = raw.get("macros").and_then(Value::as_str).unwrap_or("");
    let macros = if macros.len() <= MAX_MACROS {
        macros.trim().to_string()
    } else {
        String::new()
    };

    Options {
        fg: text("fg", "auto"),
        cell: text("cell", "auto"),
        baseline: number(raw, "baseline", 0.773, 0.05, 0.95),
        pt_px: number(raw, "ptPx", 3.4, 0.5, 20.0),
        min_shrink: number(raw, "minShrink", 0.8, MIN_SCALE, 1.0),
        thicken: number(raw, "thicken", 1.8, 1.0, 4.0),
        max_columns: number(raw, "maxColumns", f64::from(MAX_BOX), 1.0, f64::from(MAX_BOX)) as u32,
        cjk_font: text("cjkFont", "").trim().to_string(),
        packages,
        bad_packages,
        macros,
    }
}

pub fn parse_items(raw: &Value) -> Vec<Item> {
    let Some(Value::Array(items)) = raw.get("items") else {
        return Vec::new();
    };
    items
        .iter()
        .map(|item| Item {
            latex: item.get("latex").and_then(Value::as_str).unwrap_or("").to_string(),
            display: item.get("display").and_then(Value::as_bool).unwrap_or(false),
        })
        .collect()
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
pub struct Cell {
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum ItemResult {
    Picture {
        png: String,
        columns: u32,
        rows: u32,
        #[serde(rename = "textRow")]
        text_row: u32,
    },
    Failure {
        error: String,
    },
}

impl ItemResult {
    pub fn failure(error: impl Into<String>) -> Self {
        ItemResult::Failure { error: error.into() }
    }
}

#[derive(Debug, Serialize)]
pub struct Answer {
    pub cell: Cell,
    pub fg: String,
    pub results: Vec<ItemResult>,
}
