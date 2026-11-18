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
- CJK in `\text`: the program finds a CJK font (`fc-list :lang=zh` on Linux,
  ranked serif first, then sans, Regular weight first; system font folders on
  macOS). Option `cjkFont` overrides it. The choice is kept in
  `~/.cache/typeset/cjk-font`.
- Disk cache only for TeX fallback results, in `~/.cache/typeset/tex`
  (`$XDG_CACHE_HOME/typeset`): pictures, and TeX's own `!` errors and "too
  large" (an error is also keyed by the installed packages, so installing the
  missing one clears it); never time or memory limits. The downloaded program
  lives in `~/.cache/typeset/bin/<version>/`. No `TYPESET_*` variables were
  needed: the old `INLINE_MATH_*` ones served the daemon. The program reads
  only the system's (`XDG_CACHE_HOME`, `HOME`, `PATH`, `TMUX`) and RaTeX's
  `RATEX_UNICODE_FONT`, through which it hands RaTeX the CJK font (one set
  outside wins over `cjkFont`). The old `~/.cache/claude-inline-math`
  is left alone; the README says it can be deleted.
- Pictures still reach `Image` as base64 PNG (they are never written to the
  session transcript).
- Options: `minShrink`, `thicken`, `baseline`, `ptPx` keep their meaning and
  defaults (`baseline` is the terminal font's, not the engine's; RaTeX's em
  is the TeX body size, 10 pt, so `ptPx` means what it did); `packages` applies
  only to the TeX fallback; `macros` goes to both engines.
- Both engines draw at three times the terminal's pixel size (RaTeX at
  `10 * ptPx * 3` px per em, dvipng at `ptPx * 72.27 * 3` dpi with
  `--depth`), and both go through `fit.rs`, a port of the old `fit()`, so the
  baseline arithmetic is the old backend's for every picture.

## TikZ (TeX fallback only)

- Formulas: `\begin{tikzcd}` keeps working inside `$$`.
- New: a ```` ```tikz ```` block holds a TikZ picture (the whole
  `tikzpicture` environment or its body). `\usetikzlibrary{...}` (and
  `\usepgfplotslibrary`) is allowed; `\usepackage` only for pgfplots,
  circuitikz, tikz-3dplot (and `tikz`, `xcolor`) and the user's `packages`. A
  pasted standalone document loses its `\documentclass` and `document`
  wrapper. The allow-list is a policy, not the safety boundary.
- Safety by isolation instead of command filtering (the tikz-cd option filter
  is removed): `-no-shell-escape`, `openin_any=p`, `openout_any=p`, a fresh
  temporary folder per run, its own process group, a time limit (6 s for a
  formula's latex, 20 s for a TikZ picture's, 10 s for each other tool, 30 s
  for all TeX runs of one request), a 1 GiB memory limit and output size
  caps (16 MiB per file, 64 MiB per folder, 4 MiB of log).
- Drawn through dvips and Ghostscript `-dSAFER`, in the terminal foreground
  colour on a transparent background, scaled down to the terminal width, at most
  80 rows (`black` is redefined to the foreground colour). Without TeX Live
  or Ghostscript the source is shown, with the reason.

## Mermaid

- `mindmap` is drawn by the mermaid-text crate inside `typeset-render`.
- A tree-shaped flowchart (one root, at most one parent per node, no edge
  labels, no edge both ways, no subgraphs, no cycles, and some node with two
  children or more) is converted to a mindmap and drawn the same way (always
  vertical). A plain chain stays boxes and arrows, which read better there.
  A chain from the root written in one statement, with branches off it, is
  drawn as steps (`●`) with their branches, not as nested levels.
  While the renderer draws it, or when it does not fit, a tree flowchart is
  drawn by beautiful-mermaid and a mindmap shows as source.
- Every other diagram stays with the vendored beautiful-mermaid in TypeScript.
- The prompt hint asks the model to use `mindmap` for hierarchies.
- A diagram that fails says why for each layout tried.

## Commands

- `/typeset status` (and `/typeset` alone) opens a one-line summary in the
  band above the prompt (`AbovePrompt`), in every layout, as the built-in
  `/status` sits there; `d` expands the report, `x` or the next prompt closes
  it (Escape never reaches a band). It shows the last health check at once
  and is drawn again when a fresh one lands.
  `/typeset on|off|clear`, a mistyped argument and a failed hook answer with
  a toast. No subcommand output reaches the model, except `/typeset demo`,
  whose output is a sample reply to draw. Without a terminal (`claude -p`)
  they print text, as before.

## Distribution

- Platforms: Linux x64 and arm64, macOS arm64 and x64. No Windows.
- GitHub Actions builds on a `v*` tag and uploads to the Release, then opens a
  PR that writes the four sha256 sums into the repo. Users get binaries only
  after that PR is merged.
- On first use the mod downloads the binary for its platform with `curl`,
  checks the sha256 and only then runs it. A local `cargo build` in
  `renderer/` is used first when present. A failed download shows formulas as
  their Unicode stand-ins, with the reason in a toast and `/typeset status`;
  it is retried every minute, or after a failed sha256 check first after 5
  minutes, doubling up to an hour. The version downloaded comes from
  `renderer/checksums.json`; CI checks that it and `Cargo.toml` agree,
  and that `plugin.json` names that version exactly when the sums are
  there (the checksums PR moves it), as Claude Code updates an installed
  copy only when `plugin.json`'s version changes.
- Version 4.0.0. Pushing the tag and merging the PR wait for the owner.

## Acceptance

- Every formula's baseline sits exactly on the terminal text baseline.
- Columns and rows equal the old backend's, or differ by one cell. Checked by
  `cargo test` against `renderer/tests/fixtures/geometry.json`: at least 97 %
  within one cell and 99 % on the same text row (measured: 97.8 % and
  99.6 %); the formulas outside are engine differences listed in
  `docs/compatibility.md`.
- The owner approves side-by-side pictures (old | new) of `bin/corpus` (now
  `renderer/tests/corpus/`), the old ones rendered from the last commit with
  Python.

## Steps (one PR each, stacked with `gh stack`)

1. Rust program: RaTeX, the JSON protocol, hooks switched over; Python deleted.
2. TeX fallback, tikz-cd and ```` ```tikz ```` blocks.
3. mermaid-text: `mindmap` and tree-shaped flowcharts.
4. Build and release workflow; first-use download.
5. `/typeset status` pane and toasts.
6. Side-by-side pictures and acceptance.

## Status

As of 2026-11-13, steps 1 to 5 are built, each on its own local branch
stacked on the one before: `feat/rust-renderer`, `feat/tex-fallback`,
`feat/mermaid-tree`, `feat/release-binaries`, `feat/status-pane`. None is
pushed and no pull request is open yet.

Still to do, all the owner's:

- Step 6: the side-by-side pictures (old | new) of the corpus and their
  approval.
- Opening the stacked pull requests, merging them, pushing the `v4.0.0` tag
  and merging the checksums pull request. Until then there is no binary to
  download; a clone builds its own.
- The workflows and the macOS code paths have not run yet.
