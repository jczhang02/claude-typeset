<div align="center">
  <h1>typeset</h1>
  <p><b>Typeset the LaTeX in Claude's replies and draw mermaid diagrams, right in the terminal. Works inside tmux.</b></p>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-blue.svg?style=flat-square" alt="License"></a>
  <img src="https://img.shields.io/badge/Claude_Code-2.1.311%2B-d97757?style=flat-square" alt="Claude Code 2.1.311+">
  <img src="https://img.shields.io/badge/tmux-supported-1bb91f?style=flat-square&logo=tmux&logoColor=white" alt="tmux supported">
  <img src="https://img.shields.io/badge/KaTeX_corpus-99.5%25-8839ef?style=flat-square" alt="KaTeX corpus 99.5%">
</div>

<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="docs/img/hero-dark.webp">
    <img src="docs/img/hero-light.webp" alt="/typeset demo and a reply with a formula and a training-loop flowchart, in Ghostty inside tmux" width="1000">
  </picture>
</p>

> 💡 This is a Claude Code mod: a plugin with a hooks module that changes how replies are drawn in the terminal, nothing else. The stored conversation is never changed, so the model, `ctrl+o`, exports and copies still see the LaTeX and mermaid source.

## Features

- **Math inline with the text**: `$e^{i\pi} + 1 = 0$` becomes a typeset picture on the text baseline, so it reads like a line in a paper, not like source code
- **Display math**: `$$...$$` and `\[...\]` are centred on their own lines; a formula wider than the reply is drawn smaller to fit
- **Works in tmux**: pictures go through tmux's passthrough to Ghostty or kitty, and the cell size comes from tmux itself
- **Fast**: a native renderer brings one new formula to about 4 ms and ten to about 18 ms; redrawing a typeset reply costs under 2 ms
- **Broad coverage**: [RaTeX](https://github.com/erweixin/RaTeX), a KaTeX-compatible engine in Rust, checked against the KaTeX function list (99.5 % drawn), with `mhchem`, `amscd` and your own macros; no TeX installation needed
- **TeX fallback**: with TeX Live installed, a formula RaTeX cannot typeset (`\SI`, `\dv`, `\pdv`, `tikz-cd`) is typeset by TeX instead, on the same baseline
- **Mermaid diagrams as text art**: flowcharts, sequence, state, class, ER and xychart diagrams drawn in box-drawing characters, coloured by role in Catppuccin; mindmaps and tree-shaped flowcharts as vertical outline trees
- **TikZ pictures**: a ` ```tikz ` block is drawn as a picture in the text colour, scaled to the reply (needs TeX Live and Ghostscript)
- **Chinese, Japanese and Korean**: wide characters count as two cells, so text, CJK and formulas wrap together; `\text{面积}` typesets with the system's CJK font
- **Safe by default**: the engine only lays out math; TeX runs in a private folder with no shell escape, no file access outside it, and time and memory limits, so nothing in a reply can read your files, run programs, or change how another formula is drawn

## Quick Start

> Formerly `inline-math`. The command is now `/typeset`, and the options live under `typeset@…` in `pluginConfigs` (move your old `inline-math@…` entry). A stored `/inline-math off` does not carry over: the mod starts on.

You need Claude Code 2.1.311 or later and a terminal with the kitty graphics protocol (Ghostty or kitty). Formulas are drawn by `typeset-render`, a Rust program in `renderer/`; until 4.0.0 is released it is built from a clone (see below) with a Rust toolchain. Tested on Linux with Ghostty and tmux 3.8 (passthrough needs tmux 3.3 or later); macOS should work but is untested.

Chinese, Japanese or Korean text inside a formula needs a CJK font on the system (Noto CJK or Source Han; on macOS the system's own).

TeX Live is optional. With `latex` and `dvipng` on `PATH`, formulas the built-in engine cannot typeset (package commands such as `siunitx`'s `\SI` or `physics`' `\dv`) fall back to TeX. With `dvips`, Ghostscript (`gs`) and the `tikz-cd` package as well, `tikz-cd` diagrams and ` ```tikz ` blocks draw too. On Debian or Ubuntu: `sudo apt install texlive-latex-extra texlive-science texlive-pictures dvipng ghostscript` (`texlive-lang-cjk` for CJK text through TeX); on macOS, MacTeX.

**Install the mod**

Type this at the Claude Code prompt, answer `y` to add the marketplace, then pick a scope (the user scope loads it in every session):

```text
/plugin install typeset --marketplace jczhang02/claude-typeset
```

Or from a shell:

```bash
claude plugin install typeset --marketplace jczhang02/claude-typeset
```

**Inside tmux**, add these lines to `~/.config/tmux/tmux.conf` or `~/.tmux.conf`, then restart tmux:

```tmux
set -g allow-passthrough on
set -g focus-events on
```

**Run**

```text
/typeset status          # What it found: renderer, TeX tools, CJK font, cell size, colour
/typeset demo            # A sample reply: inline, tall, display, Chinese, a list, a macro, a flowchart
/typeset off             # Draw replies as Claude Code does; kept across sessions
/typeset on              # Draw formulas and diagrams again
/typeset clear           # Forget the drawn pictures and typeset them again
```

<details>
<summary><strong>Run from a clone (for development)</strong></summary>

```bash
git clone https://github.com/jczhang02/claude-typeset ~/Documents/dev-tools/claude-typeset
cargo build --release --manifest-path ~/Documents/dev-tools/claude-typeset/renderer/Cargo.toml
claude --plugin-dir ~/Documents/dev-tools/claude-typeset    # this session only
```

For every session, add the folder to `CLAUDE_CODE_PLUGIN_DIRS` in `~/.claude/settings.json` (use `:` between several folders). Claude Code reloads the mod when you save a file. Use either this or the marketplace install, not both.

```json
{ "env": { "CLAUDE_CODE_PLUGIN_DIRS": "~/Documents/dev-tools/claude-typeset" } }
```

Update an installed copy with `claude plugin update typeset@claude-typeset`.

</details>

## tmux

tmux is a first-class target: the mod is developed in Ghostty inside tmux.

- **Pictures** are sent with the kitty graphics protocol. tmux passes them through to the outer terminal only with `allow-passthrough on`; without it, every formula shows as its Unicode stand-in (`x²`, `α + β`, `√π`, `ℝ`) and diagrams still draw, as they are text.
- **Cell size** decides how large a picture is. Inside tmux the mod asks tmux for the client's cell size (`#{client_cell_width}x#{client_cell_height}`), and looks again every few seconds, so it follows the terminal you attach from. Outside tmux it asks the terminal.
- **Check it**: `tmux show -g allow-passthrough` must say `on`, and `/typeset status` shows the cell size it found. If it is wrong, set the `cell` option.

## Features in Detail

### Math

A formula is kept on one text row when it fits at 80 % of its size or more; a taller one (a fraction, a sum with limits) makes only its own line taller. Bold, italic, strikethrough, inline code and links around formulas keep their styles, and list items keep their markers and hanging indents. Until a picture is ready, and when the engine cannot typeset a formula, its Unicode stand-in takes its place. A whole `align`, `gather` or `equation` is drawn as its inner form (`aligned`, `gathered`), without equation numbers; `\tag{x}` draws as a plain (x).

Not math: code spans and code blocks, `\$`, and prices such as `$5 and $10` (the pandoc rule: an opening `$` must be followed by a non-space, a closing `$` must follow a non-space and must not be followed by a digit). Headings, tables and blockquotes are drawn by Claude Code's own Markdown renderer, so a formula inside one shows as its Unicode stand-in.

What the engine draws, and what it cannot, is described in [docs/compatibility.md](docs/compatibility.md). On the corpus in `renderer/tests/corpus/`:

| Corpus | Drawn | With TeX Live | What fails without TeX Live |
| --- | --- | --- | --- |
| KaTeX function list | 1194 / 1200 (99.5 %) | 1197 (99.8 %) | `\includegraphics`, `\reflectbox`, `\mapsfrom` |
| Formulas as Claude writes them | 233 / 239 (97.5 %) | 239 (100 %) | package commands: `siunitx` (`\SI`, `\si`), `physics` (`\dv`, `\pdv`, `\dd`) |
| MathJax extensions | 93 / 150 (62.0 %) | 131 (87.3 %) | plain TeX (`\cases`, `\buildrel`), `\bbox`, `\enclose`, physics, mathtools extras |

`\abs`, `\norm`, `\floor`, `\ceil`, `\Q`, `\C`, `\sgn`, `\diag` and `\mbox` are built in. With TeX Live, a formula the engine refuses goes to TeX, which has `siunitx`, `mhchem`, `mathtools`, `cancel`, the common `physics` commands (`\dv`, `\pdv`, `\dd`, `\vb`, `\qty`, `\bra`, `\ket`...) and the packages you list in `packages`. Without it, define the package commands you use in `macros` (`\newcommand{\dv}[2]{\frac{\mathrm{d}#1}{\mathrm{d}#2}}`).

### Diagrams

A closed ` ```mermaid ` block, at the top level of a reply or in a list item, is drawn by the bundled [beautiful-mermaid](https://www.npmjs.com/package/beautiful-mermaid) library (MIT, `hooks/vendor/`). Boxes are blue and lines lavender (Catppuccin Latte on a light desktop, Mocha on a dark one), labels in the text colour; `diagramColor: false` draws them in the text colour alone. Loops (a retry, a training loop, `A --> A`) are routed around the side of the boxes, with their own arrowheads and labels.

```text
┌────────┐        ┌─────┐     ┌────────┐  ┌──────────┐
│ Claude │        │ Mod │     │ Render │  │ Terminal │
└────┬───┘        └──┬──┘     └────┬───┘  └─────┬────┘
     │  reply text   │             │            │
     │───────────────▶             │            │
     │               │  formulas   │            │
     │               │─────────────▶            │
     │               │  pictures   │            │
     │               ◀╌╌╌╌╌╌╌╌╌╌╌╌╌│            │
     │               │      text + images       │
     │               │──────────────────────────▶
┌────┴───┐        ┌──┴──┐     ┌────┴───┐  ┌─────┴────┐
│ Claude │        │ Mod │     │ Render │  │ Terminal │
└────────┘        └─────┘     └────────┘  └──────────┘
```

A diagram the mod cannot draw correctly is shown as source, with one dim line under it that says why, never as a drawing that says something the source does not:

- Other kinds (`pie`, `gantt`, ...) and blocks the library cannot parse.
- Diagrams that are too big: more than 80 rows, 8000 characters or 150 lines, a flowchart of more than 100 nodes or 60 nodes in subgraphs, or wider than the reply. The reason names each layout tried and the limit it broke, such as `left-right: 82 rows, limit 80; top-down: 806 columns, 170 available`.
- A few loops the layout cannot route: a cycle that enters a subgraph, edges both ways between two nodes of the same layer, an edge written twice, and a node with a self loop plus another loop.
- A flowchart or state diagram whose drawing dropped an edge, an arrowhead or a label, put a line inside a box, or ran a line off the canvas, after the other layouts were tried.

`style`, `classDef`, `class`, `linkStyle` and `click` statements are ignored; entity codes (`#quot;`) and markdown-string labels draw as their text.

A `mindmap`, and a flowchart shaped as a tree, are drawn by the renderer instead (with the [mermaid-text](https://crates.io/crates/mermaid-text) crate) as a vertical outline: the root in a box, every other node on a line of its own under its parent. Laid out as boxes, a tree stacks all nodes of one depth in one column and merges the lines from different parents into one trunk, so you cannot tell whose child a node is, and a wide tree soon runs past 80 rows or the reply's width. The outline keeps every parent visible and grows down:

```text
╭──────────────╮
│ QPN 求解 MAE │
╰───────┬──────╯
        │
        ├── 1 问题: 主线句 1, 已定
        │   ├── 解必须凸
        │   └── 残差逐点依赖 Hessian
        ├── 2 缺口: 主线句 2, 改写待确认
        │   ├── 非凸网络: 可能收敛到凹解
        │   └── 所有神经求解器: 无先验误差估计
        └── 5 证据
            └── ADR 0003 对比实验: 进行中
```

- A flowchart counts as a tree when it has one root, at most one parent per node, no edge labels, no subgraphs, no cycles, and some node with two children or more (a plain chain of steps stays a row of boxes). It is drawn in that order: the root first, each node's children in the order their edges are written, labels as written, shapes dropped.
- Node shapes, icons (`::icon(...)`) and classes (`:::name`) of a mindmap are not drawn. A mindmap with two roots is refused, as mermaid refuses it.
- Labels too long for the reply are cut with `…`. A mindmap that still does not fit, or is more than 80 rows, shows as source with the reason; a tree flowchart that does not is drawn as boxes instead.
- While the renderer draws it (a few milliseconds, usually done while the reply streams), a mindmap shows as source and a flowchart as boxes. Without the renderer a tree flowchart is drawn as boxes and a mindmap shows as source.

### TikZ

A closed ` ```tikz ` block, at the top level of a reply or in a list item, is drawn by TeX as a picture, centred like a display formula. Its body is a whole `tikzpicture` environment or the inside of one:

````markdown
```tikz
\usetikzlibrary{arrows.meta}
\draw[-{Stealth}] (0,0) -- (2,1) node[right] {$f(x)$};
\draw (0,0) circle (0.5);
```
````

- `\usetikzlibrary{...}` lines may come first. `\usepackage` is allowed there only for `pgfplots`, `circuitikz`, `tikz-3dplot` and the packages in your `packages` option; any other fails the block.
- It is drawn in the terminal's text colour (`black` too, which a dark terminal would hide) on a transparent background, scaled down to the reply's width and to at most 80 rows. Other colours (`blue!30`) stay as written.
- It needs TeX Live (`latex`, `dvips`) and Ghostscript. Without them, while it is being drawn, and when TeX refuses it, the block shows as its source with one dim line saying why, as a mermaid block that cannot be drawn does.
- `tikz-cd` diagrams inside `$$...$$` are drawn the same way.

### Reply macros

Claude sometimes defines a command in a reply and uses it later in the same reply:

```latex
$$\newcommand{\norm}[1]{\left\lVert #1 \right\rVert}$$
Then $\norm{x} \le 1$ for every $x$.
```

The mod expands such definitions as text before the engine sees a formula, and the definition itself draws as nothing.

<details>
<summary><strong>Rules</strong></summary>

- `\newcommand`, `\renewcommand`, `\providecommand`, `\def`, `\let` and `\DeclareMathOperator`, also with `\global` before them.
- The scope is one reply (one text block), headings and tables included (as Unicode there). Definitions do not carry over to the next reply.
- A name defined twice takes its last definition, wherever in the reply it is; `\providecommand` does not replace an earlier definition.
- Arguments are pasted as TeX reads them: `\lVert#1` with `x` gives `\lVert x`, not the unknown `\lVertx`. A macro whose body is empty draws as nothing.
- Arguments `#1` to `#9`, with an optional first argument and its default. `\def` takes undelimited parameters only; a `\def` with delimited parameters, `\edef` and `\xdef` are not expanded, and the engine refuses them.
- Expansion stops at 32 levels of nesting or 8000 characters; a formula past either limit is drawn as written, which the engine then refuses.

</details>

### Streaming

Claude Code draws a reply's text itself while it streams, so formulas show as LaTeX source until their text block is finished; a mod cannot replace that live text. Meanwhile the mod reads the stream (without changing it) and queues each formula once its closing `$` has arrived. When the block ends, its formulas are usually typeset already and appear at once. A formula closed just before the block ends can show as its Unicode stand-in for one renderer round trip (a few milliseconds).

### Speed

Measured from process start to exit, as Claude Code runs `typeset-render` (one run per batch, no daemon):

| Case | 1 new formula | 10 new formulas |
| --- | --- | --- |
| Formulas | 4 ms | 18 ms |
| Chemistry (`\ce`) | 9 ms | |
| Chinese text (`\text{速度}`) | 15 ms | |
| TeX fallback (`\SI{1.5}{\metre}`) | 340 ms, then 3 ms from disk | |
| A ` ```tikz ` block | 460 ms, then 4 ms from disk | |

<details>
<summary><strong>How it stays fast</strong></summary>

- **Native engine.** RaTeX parses and lays out a formula in well under a millisecond; the fonts are built into the program. The CJK font is looked up only for a batch with CJK text, and the choice is kept in `~/.cache/typeset/cjk-font`.
- **TeX only when needed.** A formula goes to TeX only when RaTeX refuses it, and a TeX picture is kept on disk in `~/.cache/typeset/tex` (2000 pictures or 200 MB at most), so each one costs a TeX run once. Several TeX runs in one call go four at a time.
- **Batches.** Formulas asked for within 30 ms share one call of up to 16. Formulas found while a reply streams gather for up to 400 ms, also while a call is running, so a long reply costs a few calls.
- **Cheap redraws.** Each reply is parsed once, and a reply whose pictures are ready returns its last drawing. When pictures arrive, only the replies that waited for them are drawn again. Pictures stay in memory for the session.

</details>

## Options

Set them in `/config`, or in `~/.claude/settings.json` under `pluginConfigs`:

```json
{
  "pluginConfigs": {
    "typeset@claude-typeset": {
      "options": {
        "packages": "physics, tensor",
        "macros": "\\newcommand{\\R}{\\mathbb{R}}\n\\DeclareMathOperator{\\tr}{tr}"
      }
    }
  }
}
```

| Option | Default | Meaning |
| --- | --- | --- |
| `enabled` | `true` | Draw formulas and diagrams. `/typeset on` or `off` overrides it. |
| `foreground` | `auto` | Formula colour. `auto` follows the desktop's light or dark scheme (Catppuccin Latte text `#4c4f69`, Mocha text `#cdd6f4`); or `#rrggbb`. |
| `cell` | `auto` | One character cell in pixels. `auto` asks tmux, then the terminal; or `WxH`, such as `19x42`. |
| `baseline` | `0.773` | Where the font's baseline sits in a cell, from the top, as a fraction of the cell height. 0.773 is JetBrains Mono. |
| `ptPx` | `3.4` | Formula size in pixels per TeX point (a formula's body size is 10 pt). 3.4 matches a 12 pt font with 42 px cells. |
| `minShrink` | `0.8` | How far an inline formula may shrink to stay on one row before it takes more rows. |
| `thicken` | `1.8` | Makes the antialiased edges of thin strokes more opaque (alpha $a$ drawn as $1 - (1 - a)^{t}$), so formulas read as firmly as the terminal font. `1` draws them as the engine does. |
| `promptHint` | `true` | Add a short note (under 100 tokens) to Claude's system prompt: write math as `$...$` and `$$...$$`, never in code blocks; mermaid blocks draw as diagrams (`mindmap` for hierarchies), tikz blocks as pictures. |
| `texFallback` | `true` | Typeset a formula the built-in engine refuses with TeX Live, when `latex` and `dvipng` are installed; also what draws `tikz-cd` and ` ```tikz ` blocks. Off: such formulas show as text, tikz blocks as source. |
| `packages` | empty | Extra TeX packages for the TeX fallback, separated by commas or spaces (`physics, tensor`); a ` ```tikz ` block may load them too. One that is not installed is skipped, and `/typeset status` names it. |
| `macros` | empty | Your own definitions for every formula: `\newcommand`, `\DeclareMathOperator`, `\def`, one or more lines. A name the engine already has (`\R`) takes your definition. In `settings.json` write `\n` between lines and `\\` for each backslash. |
| `cjkFont` | empty | A font file for CJK text in formulas: a path, or `path#index` for a collection (`.ttc`). Empty: the system's own, a serif face (Noto Serif CJK, Source Han Serif) first. |
| `diagramColor` | `true` | Colour diagrams by role. Off: the text colour alone. |

## Limitations

- Pictures need the kitty graphics protocol. iTerm2, WezTerm, Alacritty, VS Code's terminal and Windows Terminal show the Unicode stand-ins; diagrams still draw.
- Only the terminal is drawn. The desktop app, VS Code and mobile show replies as they always do.
- Text in a paragraph with a formula copies as the text around the pictures; a picture copies as nothing. Use `ctrl+o` to see and copy the source.
- The formula colour follows the desktop scheme (through the XDG portal or GNOME's setting), not a theme set in the terminal alone. Set `foreground` if they differ.
- After the terminal font size changes, pictures are typeset again within a few seconds; until then the terminal scales them.
- CJK text is drawn in one font: Simplified Chinese glyph forms unless `cjkFont` names another.
- Without TeX Live, LaTeX packages cannot be loaded: a command from `physics`, `siunitx` and the like shows as text unless `macros` defines it, and ` ```tikz ` blocks show as source.
- A formula that falls back to TeX is drawn in TeX's fonts (Latin Modern), which differ a little from the engine's (KaTeX's).

## Troubleshooting

- **Formulas show as Unicode, never as pictures.** Run `/typeset status`; a renderer that is not ready says why (for a clone: build it with `cargo build --release` in `renderer/`). Then `/typeset clear`. If the renderer is ready, check that the terminal is Ghostty or kitty and that `tmux show -g allow-passthrough` says `on`. Restart tmux after you change its config.
- **CJK text in a formula is missing or in the wrong font.** `/typeset status` names the CJK font; set `cjkFont`, or delete `~/.cache/typeset/cjk-font` after installing a new font.
- **The old cache.** Versions before 4.0.0 kept pictures in `~/.cache/claude-inline-math`; it is no longer used and can be deleted.
- **A TeX picture is stale** (after a TeX Live update, say). TeX pictures are kept in `~/.cache/typeset/tex`; delete the folder, then `/typeset clear`. Installing a package needs neither: formulas are typeset again with it.
- **A tikz block shows as source.** The dim line under it says why: TeX Live or Ghostscript missing (`/typeset status` lists the TeX tools), a `\usepackage` that is not allowed, or TeX's own error.
- **Formulas sit too high or too low.** Set `baseline` for your font, and `cell` if tmux reports the wrong size (`tmux display -p '#{client_cell_width}x#{client_cell_height}'`).
- **Formulas are too large or too small.** Change `ptPx`.
- **Formulas look faint or thin beside the text.** Raise `thicken` (2 to 2.5). On a desktop scaled by a fraction (1.25, 1.5) the compositor shrinks the whole window, which thins strokes further.
- **One formula shows as text.** The engine refused it (and TeX too, when installed); `/typeset status` lists recent errors. Check [docs/compatibility.md](docs/compatibility.md).
- **A diagram shows as source.** The dim line under it says why. A wider window helps when it is too wide.
- **Claude writes math in code blocks.** Check that `promptHint` is on; the note applies from the next turn.
- **Lines of base64 appear in the pane, and old lines stay on screen.** Something other than Claude Code wrote to the pane's tty while a picture was being sent, often a hook that sends a desktop notification (`printf '\ePtmux;...' > "#{pane_tty}"`). A picture can be 100 KB or more and goes out in several writes; a sequence that lands between two of them ends the picture early, and tmux draws the rest of it as text. Never write to the tty of a Claude Code pane: send the sequence through another pane, such as a short-lived hidden window with `allow-passthrough all`, as tmux passes each pane's passthrough to the terminal in one piece. Resize or zoom the pane (`prefix z` twice) to redraw it.
- **Something looks wrong.** `/typeset off` draws replies as Claude Code does; `claude --debug` logs why a hook was skipped.

## License

MIT, see [LICENSE](LICENSE). The bundled beautiful-mermaid library is MIT too, see [hooks/vendor/LICENSE](hooks/vendor/LICENSE). The renderer uses RaTeX (MIT) and its bundled KaTeX fonts (OFL), and mermaid-text (MIT) for trees; the TeX fallback runs your own TeX Live.
