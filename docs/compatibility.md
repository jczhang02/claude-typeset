# Compatibility

What the renderer (`typeset-render`, the Rust program in `renderer/`)
draws, measured on three corpora in `renderer/tests/corpus/`:

| Corpus | Source | Formulas | Drawn |
|---|---|---|---|
| `katex.txt` | every formula of KaTeX's [support table](https://katex.org/docs/support_table.html) and [supported functions](https://katex.org/docs/supported.html) pages (fetched 2026-10-27) | 1200 | 1194 (99.5 %) |
| `claude.txt` | formulas as Claude writes them: analysis, linear algebra, probability, ML, physics, chemistry, CS, Chinese `\text`, aligned derivations | 239 | 233 (97.5 %) |
| `mathjax.txt` | examples for the MathJax 3 TeX extensions (base, ams, amscd, bbox, boldsymbol, braket, cancel, cases, centernot, color, colorv2, empheq, enclose, extpfeil, gensymb, html, mathtools, mhchem, physics, upgreek, unicode, verb, textmacros, action) | 150 | 93 (62.0 %) |

`renderer/tests/fixtures/geometry.json` holds every corpus formula with
the cells the previous backend (TeX, v3) drew it on. `cargo test` in
`renderer/` checks that at least 97 % of the formulas both draw land on
the same cells, give or take one, and 99 % on the same text row.

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
   text) are drawn with the system font described below.
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
a mixed one), and a macro that guessed wrong would draw a wrong formula
without a word. Those show as text; define the form you use in `macros`.

## The user's macros

`macros` go before every formula. A name that already exists (RaTeX's
`\R`, a built-in `\abs`) takes the user's definition: `\newcommand` of it
is read as `\renewcommand`. `\DeclareMathOperator{\tr}{tr}` and
`\DeclareMathOperator*`, which KaTeX lacks, become `\operatorname{tr}`
and `\operatorname*`. `/typeset status` names a definition RaTeX refuses.

## Chinese, Japanese and Korean

CJK text in `\text{...}` is drawn with one system font:

- the `cjkFont` option when it names a font file (`path`, or `path#index`
  for a collection such as `.ttc`);
- else on Linux the best Regular face fontconfig lists for Chinese: Noto
  Serif CJK SC, Source Han Serif SC, the Traditional Chinese serif faces,
  then the sans faces, then any CJK face. A serif face comes first, as the
  previous backend drew CJK text in a Song (serif) face;
- else on macOS Songti, then STHeiti or Hiragino Sans GB.

The choice is kept in `~/.cache/typeset/cjk-font` while its file exists,
and looked up only for a batch with CJK text (fontconfig takes about
10 ms). One font draws all CJK text, so Japanese kanji and Traditional
Chinese use the glyph forms of the chosen face.

## What does not draw

A formula the engine refuses shows as its Unicode stand-in, and
`/typeset status` lists the error. From the corpora:

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
