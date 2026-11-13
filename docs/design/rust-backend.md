# Design: a Rust renderer in place of the Python backend (v4.0.0)

Agreed 2026-11-12. Goals, in order of weight: simpler install (no Python, no
Pillow, TeX Live optional), one codebase in Rust and TypeScript, faster
formulas.

## Formulas

- The Python backend (`bin/`, its tests and tools) is deleted. A Rust program,
  `typeset-render` (crate in `renderer/`), does all rendering. The hooks keep
  parsing replies, layout and drawing, and call the program with
  `$.process.run`, one run per batch, JSON on stdin and stdout (the protocol of
  the old `client.py`). No daemon.
- Engine: RaTeX first. TeX Live is an optional fallback, used only when RaTeX
  fails on a formula (parse or layout error), and only when `latex` and
  `dvipng` are found. Option `texFallback` (default on) turns it off.
- CJK in `\text`: the program finds a CJK font (`fc-match :lang=zh` on Linux,
  system font folders on macOS), Regular weight first. Option `cjkFont`
  overrides it.
- Disk cache only for TeX fallback pictures, in `~/.cache/typeset`
  (`$XDG_CACHE_HOME/typeset`). Environment variables are `TYPESET_*`. The old
  `~/.cache/claude-inline-math` is left alone; the README says it can be
  deleted.
- Pictures still reach `Image` as base64 PNG (they are never written to the
  session transcript).
- Options: `minShrink`, `thicken`, `baseline`, `ptPx` keep their meaning and
  defaults (`baseline` is the terminal font's, not the engine's; RaTeX's em
  is the TeX body size, 10 pt, so `ptPx` means what it did); `packages` applies
  only to the TeX fallback; `macros` goes to both engines.

## TikZ (TeX fallback only)

- Formulas: `\begin{tikzcd}` keeps working inside `$$`.
- New: a ```` ```tikz ```` block holds a TikZ picture (the whole
  `tikzpicture` environment or its body). `\usetikzlibrary{...}` is allowed;
  `\usepackage` only for pgfplots, circuitikz, tikz-3dplot and the user's
  `packages`.
- Safety by isolation instead of command filtering (the tikz-cd option filter
  is removed): `-no-shell-escape`, `openin_any=p`, `openout_any=p`, a fresh
  temporary folder per run, a time limit, a 1 GiB memory limit and an output
  size cap.
- Drawn through dvips and Ghostscript `-dSAFER`, in the terminal foreground
  colour on a transparent background, scaled down to the terminal width, at most
  80 rows. Without TeX Live or Ghostscript the source is shown.

## Mermaid

- `mindmap` is drawn by the mermaid-text crate inside `typeset-render`.
- A tree-shaped flowchart (one root, at most one parent per node, no edge
  labels) is converted to a mindmap and drawn the same way (always vertical).
- Every other diagram stays with the vendored beautiful-mermaid in TypeScript.
- The prompt hint asks the model to use `mindmap` for hierarchies.
- A diagram that fails says why for each layout tried.

## Commands

- `/typeset status` opens a temporary pane (`$.ui.open`, closed with Esc),
  laid out like `/btw`. `/typeset on|off|clear` answer with a toast. No
  subcommand output reaches the model.

## Distribution

- Platforms: Linux x64 and arm64, macOS arm64 and x64. No Windows.
- GitHub Actions builds on a `v*` tag and uploads to the Release, then opens a
  PR that writes the four sha256 sums into the repo. Users get binaries only
  after that PR is merged.
- On first use the mod downloads the binary for its platform with `curl`,
  checks the sha256 and only then runs it. A local `cargo build` in
  `renderer/` is used first when present. A failed download shows formulas as
  source, with the reason.
- Version 4.0.0. Pushing the tag and merging the PR wait for the owner.

## Acceptance

- Every formula's baseline sits exactly on the terminal text baseline.
- Columns and rows equal the old backend's, or differ by one cell.
- The owner approves side-by-side pictures (old | new) of `bin/corpus`,
  the old ones rendered from the last commit with Python.

## Steps (one PR each, stacked with `gh stack`)

1. Rust program: RaTeX, the JSON protocol, hooks switched over; Python deleted.
2. TeX fallback, tikz-cd and ```` ```tikz ```` blocks.
3. mermaid-text: `mindmap` and tree-shaped flowcharts.
4. Build and release workflow; first-use download.
5. `/typeset status` pane and toasts.
6. Side-by-side pictures and acceptance.
