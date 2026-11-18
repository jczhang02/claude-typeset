# Compatibility

What the renderer (`typeset-render`, the Rust program in `renderer/`)
draws, measured on three corpora in `renderer/tests/corpus/`, with RaTeX
alone and with the TeX fallback (TeX Live 2024):

| Corpus | Source | Formulas | Drawn | With TeX Live |
|---|---|---|---|---|
| `katex.txt` | every formula of KaTeX's [support table](https://katex.org/docs/support_table.html) and [supported functions](https://katex.org/docs/supported.html) pages (fetched 2026-10-27) | 1200 | 1194 (99.5 %) | 1197 (99.8 %) |
| `claude.txt` | formulas as Claude writes them: analysis, linear algebra, probability, ML, physics, chemistry, CS, Chinese `\text`, aligned derivations | 239 | 233 (97.5 %) | 239 (100 %) |
| `mathjax.txt` | examples for the MathJax 3 TeX extensions (base, ams, amscd, bbox, boldsymbol, braket, cancel, cases, centernot, color, colorv2, empheq, enclose, extpfeil, gensymb, html, mathtools, mhchem, physics, upgreek, unicode, verb, textmacros, action) | 150 | 93 (62.0 %) | 131 (87.3 %) |

`renderer/tests/fixtures/geometry.json` holds every corpus formula with
the cells the previous backend (TeX, v3) drew it on. `cargo test` in
`renderer/` checks that at least 97 % of the formulas both draw land on
the same cells, give or take one, and 99 % on the same text row, and
reports how many drew through the TeX fallback (47 here). With TeX Live
on `PATH` it also checks that every formula of `claude.txt` draws.

## How a formula gets drawn

1. `hooks/layout.ts` finds the math in a reply and expands the reply's own
   `\newcommand`, `\renewcommand`, `\providecommand`, `\def`, `\gdef`,
   `\let` and `\DeclareMathOperator`, with or without `\global` before
   them. The renderer never sees a reply's definition.
2. `normalize.rs` drops comments, turns a whole `align`, `gather`,
   `multline`, `alignat`, `flalign`, `eqnarray` or `split` into its inner
   form (`aligned`, `gathered`, `alignedat`), a whole `equation` into its
   body, `numcases` into `cases` and `empheq` into its inner form with its
   delimiters or box. `\label`, `\nonumber` and `\notag` go, `\tag{x}`
   draws as a plain (x), `\intertext` splits the lines into blocks one
   under the other. A display formula with a top-level `\\` becomes
   `gathered` (or `aligned` when it has `&`).
3. [RaTeX](https://github.com/erweixin/RaTeX) 0.1.15, a KaTeX-compatible
   engine, typesets it at three times the terminal's pixel size, after the
   built-in definitions and the user's `macros`. Its fonts are KaTeX's
   (Computer Modern), built into the program; characters they lack (CJK
   text) are drawn with the system font described below. When RaTeX
   refuses the formula, TeX typesets it instead, if it can (see "TeX
   fallback"); a `tikzcd` environment goes to TeX at once.
4. `fit.rs` crops the ink, scales it down to whole terminal cells with its
   baseline on the text baseline of one cell row (the same arithmetic as
   the previous backend), firms up thin strokes (`thicken`) and encodes a
   PNG.

## Built-in definitions

Commands Claude writes often that RaTeX lacks, defined only where RaTeX
does not have them. A user's `macros` replace them.

| Command | Drawn as |
|---|---|
| `\abs{x}`, `\norm{x}`, `\floor{x}`, `\ceil{x}` | bars, double bars, floor and ceiling brackets that grow with their content |
| `\Q`, `\C` | `\mathbb{Q}`, `\mathbb{C}` (RaTeX has `\R`, `\N`, `\Z`) |
| `\sgn`, `\diag` | operators |
| `\mbox{x}` | `\text{x}` |

Only forms that cannot draw something other than what was meant are
defined: physics' `\dv`, `\pdv` and `\dd` take their arguments in several
ways (`\dv{x}` is an operator, `\dv{f}{x}` a derivative, `\pdv{f}{x}{y}`
a mixed one), and a KaTeX macro that guessed wrong would draw a wrong
formula without a word. The TeX fallback reads them all as physics does;
without TeX Live they show as text, so define the form you use in
`macros`.

## TeX fallback

When RaTeX refuses a formula (a parse or layout error), the option
`texFallback` is on (the default) and `latex` and `dvipng` are on `PATH`,
TeX Live typesets it instead. Otherwise RaTeX's error stands. When TeX
refuses it too, the error names both (`ParseError: ...; TeX: ! ...`).

- **Preamble**, as the previous backend's: `\documentclass[10pt]{article}`
  with `fontenc` (T1, OT1), `lmodern`, `amsmath`, `amssymb`, `mathtools`,
  `mathrsfs`, `cancel`, `xcolor`, `bm`, `CJKutf8`, `mhchem`, `siunitx`,
  `dsfont`, `stmaryrd`, `amscd`, `ulem` and `centernot`, each loaded when
  installed (kpsewhich finds them; `preview`, `amsmath` and `amssymb` are
  needed). Then the user's `packages` (those installed), `preview` for a
  page as tight as the formula, the user's `macros` (each name they define
  first set to undefined, so theirs wins over a package's) and
  `renderer/src/compat.tex`.
- **compat.tex** defines what models write without loading a package:
  physics' `\dv`, `\pdv` (with the mixed `\pdv{f}{x}{y}`), `\dd`, `\vb`,
  `\qty`, `\pqty`, `\bra`, `\ket`, `\braket`, `\comm`, `\order`,
  `\mqty` and more, without the physics package (which redefines `\div`,
  `\Re` and `\Im`); a user who lists `physics` in `packages` gets the
  package's own. siunitx's `\qty{1}{\metre}` still works: a `\qty{...}`
  followed by a second group is siunitx's. KaTeX's and MathJax's aliases
  (`\reals`, `\R`, `\argmax`, `\upalpha`, `\Bra`...) and CSS colour
  names are defined too. A glyph missing from TeX's fonts is an error,
  not a blank.
- **Unicode.** The Unicode symbols models write in formulas (Greek
  letters, `≤`, `→`, `×`, `ℝ`, `²`...) are declared as their TeX math,
  as RaTeX reads them, in math and in `\text`. KaTeX's own syntax (`#`
  colours in `\color`, `\htmlClass`) is not rewritten for TeX.
- **Scale and baseline.** dvipng rasters at `ptPx * 72.27 * 3` dpi, the
  scale RaTeX draws at, with `--depth` (which counts preview's border), so
  `fit.rs` places the picture with the same arithmetic, its baseline on
  the text baseline. A formula too wide for the reply is rastered smaller
  rather than at full size and shrunk.
- **Fonts.** A TeX formula is drawn in Latin Modern, close to but not the
  same as KaTeX's fonts. CJK text goes through CJKutf8 (Korean for Hangul,
  Japanese for kana, else Simplified Chinese), not the system font.
- **One run per formula.** Fallback is rare, so each formula runs alone
  (up to four side by side); a call's TeX runs together get 30 s, after
  which its formulas fail with "timeout" and are asked for again later.
- **Cache.** TeX pictures, and errors that come from the formula itself
  (TeX's own `!` errors, too large; not a time or memory limit, which a
  busy machine may hit), are kept in `$XDG_CACHE_HOME/typeset/tex` (else
  `~/.cache/typeset/tex`), keyed by a SHA-256 of the renderer version,
  the options that change the picture (colour, cell size, baseline,
  `ptPx`, `minShrink`, `thicken`, width, `packages`, `macros`) and the
  formula; an error also by the packages TeX Live has, so installing the
  missing one clears it. At most 2000 entries or 200 MB, the least
  recently used deleted first. RaTeX's pictures are not kept on disk.

On the corpora, the TeX fallback draws `\SI`, `\si`, `\dv`, `\pdv`, `\dd`,
`\vb*`, `\qty`, `\mapsfrom`, `\reflectbox`, plain TeX's `\root` and
`\buildrel`, and most mathtools extras. It still refuses `\cases` and
`\matrix{...\cr...}` (amsmath rejects the old forms), `\moveleft`,
`\mathreflectbox`, and MathJax's own `\bbox`, `\enclose`, `\unicode`,
`\class`, `\style`, `\require` and the like, which LaTeX lacks.

## tikz-cd

`\begin{tikzcd}...\end{tikzcd}` in a formula (as a reply writes it,
inside `$$...$$`) goes straight to TeX with `tikz-cd` loaded, and through
dvips and Ghostscript (`-dSAFER`, `pngalpha`) rather than dvipng, which
misplaces TikZ's text. The depth comes from the box TeX measured, so the
diagram's baseline sits on the text baseline as a formula's does. It needs
`texFallback` on, `latex`, `dvips`, `gs` and the `tikz-cd` package; without
them it fails and says which is missing.

## tikz blocks

A closed ```` ```tikz ```` block (at the top level of a reply or in a list
item) is drawn as a picture, centred like a display formula.

- Its body is a whole `tikzpicture` environment, or the inside of one
  (then wrapped in one). A pasted standalone document's `\documentclass`
  and `document` environment are dropped.
- Lines at its top may be `\usetikzlibrary{...}` (or
  `\usepgfplotslibrary`) and `\usepackage{...}` for `pgfplots`,
  `circuitikz`, `tikz-3dplot` (and `tikz`, `xcolor`) or a package in the
  user's `packages`. Any other `\usepackage`, or one further down, fails
  the block with that reason.
- It is drawn through dvips and Ghostscript in the terminal's text colour
  (`black` is redefined to it, as a dark terminal would hide black) on a
  transparent background, scaled down to the reply's width and to at most
  80 rows; smaller than a quarter of its size, it is too large.
- Without `latex`, `dvips` or `gs`, with `texFallback` off, or when TeX
  refuses it, the block shows as its source with one dim line saying why,
  as a mermaid block that cannot be drawn does. It is also shown as source
  while TeX draws it (about half a second, once: pictures are cached).
- The request protocol asks for it with an item `{"latex": body, "kind":
  "tikz"}`; items without `kind` are formulas, as before.

## Isolation

The LaTeX comes from model output, and TikZ is a programming language, so
TeX runs are made safe by isolation rather than by filtering commands (the
previous backend's command and tikz-cd option filters are gone). Every
latex, dvipng, dvips, gs and kpsewhich run:

- runs in a fresh private folder (mode 0700) under the system's temporary
  folder, deleted afterwards, as its own process group; on Linux it is
  killed when the renderer is (the limits below are kept by the
  renderer), and a folder a killed renderer left is deleted by the first
  TeX run an hour later;
- has `openin_any=p` and `openout_any=p`: TeX may not read or write files
  by absolute path, in a parent folder or starting with a dot, so
  `\openin` or `\input` of `/etc/hostname` finds nothing and `\openout`
  outside the folder is refused; `TEXINPUTS` and the like are unset;
- runs latex with `-no-shell-escape -interaction=nonstopmode
  -halt-on-error`, dvips with `-R2` (no shell, no absolute paths) and
  Ghostscript with `-dSAFER`; dvipng loads the image of a `PSfile`
  special itself, from any path, so a formula whose page has one is
  refused before dvipng runs;
- is killed, with anything it started, after a wall-clock limit (6 s for
  a formula's latex, 20 s for a TikZ picture's, 10 s for the other tools),
  and has a CPU time limit just above it;
- has a 1 GiB address space limit (`RLIMIT_AS`) and a 16 MiB limit on any
  file it writes (`RLIMIT_FSIZE`), set in the child before it starts, and
  is killed when its folder grows past 64 MiB;
- hands back at most 4 MiB of log, and a picture of at most 32 million
  pixels, checked from the PNG header before it is decoded.

The tests check that a formula reading `/etc/hostname` draws nothing of
it and that `\def\a{\a}\a` is stopped by the time limit.

## The user's macros

`macros` go before every formula. A name that already exists (RaTeX's
`\R`, a built-in `\abs`) takes the user's definition: `\newcommand` of it
is read as `\renewcommand`. `\DeclareMathOperator{\tr}{tr}` and
`\DeclareMathOperator*`, which KaTeX lacks, become `\operatorname{tr}`
and `\operatorname*`; the star of `\newcommand*` is dropped.

Each definition is checked on its own. One RaTeX refuses
(`\newenvironment`, `\DeclarePairedDelimiter`) or reads only in part (an
optional argument with its default, `\newcommand{\set}[2][x]{...}`, whose
rest RaTeX would draw as text) is left to TeX: the other formulas draw
as usual, and one that uses the name goes to the TeX fallback, which has
the macros whole (without TeX it shows as text). `/typeset status` names
each definition left to TeX.

## Chinese, Japanese and Korean

CJK text in `\text{...}` is drawn with one system font:

- the `cjkFont` option when it names a font file (`path`, or `path#index`
  for a collection such as `.ttc`);
- else on Linux the best Regular face fontconfig lists for Chinese: Noto
  Serif CJK SC, Source Han Serif SC, the Traditional Chinese serif faces,
  then the sans faces, then any CJK face. A serif face comes first, as the
  previous backend drew CJK text in a Song (serif) face;
- else on macOS Songti, then STHeiti or Hiragino Sans GB.

A face counts only when it has CJK glyphs: a missing file, a collection
index it lacks (`#99`) or a Latin font is passed over, and `/typeset
status` names a `cjkFont` that is one of these. RaTeX would leave a
character its font lacks out of the picture without a word, so a formula
with a CJK character the chosen face lacks (or with no face at all) is
not drawn by RaTeX: it goes to the TeX fallback, or shows as text.

The choice is kept in `~/.cache/typeset/cjk-font` while its file exists,
and looked up only for a batch with CJK text (fontconfig takes about
10 ms). One font draws all CJK text, so Japanese kanji and Traditional
Chinese use the glyph forms of the chosen face.

## Mermaid trees

Mermaid blocks are drawn in the hooks by beautiful-mermaid, except trees:
a `mindmap`, and a flowchart that is a tree, go to the renderer as items
of kind `diagram` and come back as lines of text drawn by the
mermaid-text crate (0.58.0), an outline under a boxed root
(`renderer/src/diagram.rs`).

- `mindmap`: nodes by indentation, the first one the root. The text inside
  a shape (`((text))`, `[text]`, `{{text}}`, `))text((` and the rest) is
  drawn without the shape; quotes, markdown-string backticks and `<br>`
  go, and a markdown string over several lines is one line of label (one
  that never closes is refused). `::icon(...)`, `:::class` lines, `%%` comments and front matter are
  skipped. A second root is refused ("mindmap has more than one root").
- Flowcharts (`flowchart`, `graph`, any direction): parsed by the crate
  (long arrows such as `--->`, `----` and `-..->` first made the short
  ones it reads, outside labels), drawn as a tree when there is one root, one parent at most per node, no
  edge label, no edge both ways, no subgraph, no cycle, and some node with
  two children or more. The answer for any other flowchart starts with
  "not a tree", and the hooks draw it as boxes.
- A tree flowchart whose root starts a chain written in one statement
  (`A --> B --> C --> D`, four nodes or more, and the only statement that
  is a chain) is drawn as steps: one `● ` line per node of the chain, a `│`
  line between them, and each step's other children as an outline under
  it. Edges written one per statement, fan-outs with `&`, and two or more
  chains (a hierarchy written as paths) draw the outline as before.
- The drawing keeps to the reply's width (at most 255 columns) by cutting
  long labels with `…`, and to 80 rows; otherwise it is refused with its
  size.

`renderer/tests/fixtures/mermaid/` holds two trees Claude wrote (about 40
nodes, Chinese labels), each as a flowchart and as a mindmap; the tests
check that both forms draw the same lines (60 x 38 and 67 x 39 cells) and
that every label appears as written.

## What does not draw

A formula the engine refuses (and TeX, when installed) shows as its
Unicode stand-in, and `/typeset status` lists the error. From the
corpora, without TeX Live (the TeX fallback draws most package commands
and LaTeX commands below; see "TeX fallback"):

| Kind | Commands |
|---|---|
| Package commands | siunitx (`\SI`, `\si`), physics (`\dv`, `\pdv`, `\dd`, `\vb`, `\qty`, `\pqty`, `\grad`, `\curl`, `\comm`, `\order`, `\mqty`, `\Res`, `\ketbra`), mathtools (`\prescript`, `\splitfrac`, `\smashoperator`, `\adjustlimits`, `\cramped`, `\mathmbox`, `\MoveEqLeft`, `\Aboxed`, `\vdotswithin`, `\shoveleft`, `\DeclarePairedDelimiter`, `multlined`, `lgathered`), cancel (`\cancelto`), upgreek (`\upalpha`), gensymb (`\celsius`), `\iiiint`, `\ndownarrow`, `\mapsfrom` |
| Plain TeX | `\root ... \of`, `\buildrel ... \over`, `\matrix{..\cr..}`, `\pmatrix`, `\cases`, `\moveleft`, `\raise`, `\lower` |
| MathJax only | `\bbox`, `\enclose`, `\unicode`, `\class`, `\style`, `\data`, `\mathtip`, `\texttip`, `\toggle`, `\require`, `\mmlToken`, `\Rule`, `\Space`, `\Newextarrow` |
| LaTeX not in KaTeX | `\eqref`, `\ref`, `\sideset`, `\leftroot`, `\uproot`, `\mspace`, `\cline`, `\definecolor`, `\iddots`, `\reflectbox`, `\mathreflectbox`, `\includegraphics` |

## Known differences from the previous backend

The previous backend (v3) typeset with real TeX. On the corpora, 98 % of
the formulas land on the same cells give or take one; the rest differ
because the engines do:

- mhchem's reaction arrows are shorter.
- `\ ` (a control space) is narrower.
- A run of Unicode operators (`⊕ ⊗ ≠ ≤`) is set tighter.
- `\hline` and `\hdashline` in a matrix take less height (one row less
  for a two-row matrix).
- `a\newline b` in an inline formula stays on one line.
