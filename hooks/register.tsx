import type { Elements, EngineInterface, Register, RenderElement, UiOpenResult } from 'claude-code'

import { layoutParagraph, mathRefs, parseMessage, stringWidth, toUnicode } from './layout'
import type { Block, Line, MathRef, PictureState, Run } from './layout'
import { renderMermaid, treeRoute, treeRows } from './mermaid'
import type { DiagramResult, DiagramRole, DiagramSegment } from './mermaid'
import { check, clear, configure, flush, lookup, noteDiagram, onHealth, onProblem, prefetch, readSettings, runWorker, startPolling, stats, theme } from './render-client'
import type { DiagramRef, DiagramState, Health, Settings } from './render-client'

// typeset: draws the LaTeX in Claude's replies as typeset pictures, inline
// with the terminal text and on its baseline, ```mermaid blocks as Unicode
// diagrams (mindmaps and tree-shaped flowcharts as outline trees the renderer
// draws), and ```tikz blocks as pictures (TeX Live drawing them). Blocks without either go to the surface's own Markdown
// element; paragraphs with formulas are wrapped here, line by line
// (layout.ts), since a picture is a box of whole cells that no Text flows
// around. Pictures come from the renderer (typeset-render, the Rust program
// in renderer/) through render-client.ts; a draw never waits for them.
//
// Cost: every redraw request runs this hook for every reply on screen, so a
// reply whose pictures are all ready and unchanged returns the tree it drew
// last time (checked formula by formula, a map lookup each). A reply still
// waiting reads its member of the `redraw` state family, and is drawn again
// when a batch it waited for lands; no other reply is.
//
// While a reply streams, the engine draws its text itself; a turn.step hook
// watches the text go by (passing every chunk on unchanged) and queues each
// formula as soon as it is closed, so its picture is ready when the
// finished block is drawn.

const COMMAND = 'typeset'
const STORE_ENABLED = 'enabled'

// The engine draws an assistant block as one row: a 2-cell column holding the
// reply bullet (only on the block that opens a reply), then the markdown.
// Measured at 80 columns: the bullet at column 0, text from column 2, lines
// wrapped at 78. A hook's tree replaces that whole row, so the tree draws the
// bullet itself, and a block without it starts at column 0, as the engine's.
const BULLET_GUTTER = 2
// A command's output row: a dim "  ⎿  " (5 cells), then the text.
const OUTPUT_PREFIX = '  ⎿  '
const OUTPUT_GUTTER = 5
// The engine's reply bullet: U+23FA on macOS, U+25CF elsewhere.
const BULLET_MAC = '⏺'
const BULLET_OTHER = '●'
// The theme key the engine's Markdown draws inline code in.
const CODE_COLOR = 'permission'
// Diagram colours: Catppuccin blue for boxes, lavender for lines and arrows,
// in the flavour that matches the formulas' colour (Latte on a light
// background, Mocha on a dark one); labels in the body text colour.
const DIAGRAM_COLORS: Record<'light' | 'dark', { border: string; edge: string }> = {
  light: { border: '#1e66f5', edge: '#7287fd' },
  dark: { border: '#89b4fa', edge: '#b4befe' },
}
const DIAGRAM_ROWS = 80
// Borders and edges while the light or dark scheme is not known yet: a theme key.
const DIAGRAM_FALLBACK = 'suggestion'

// The state family a drawn reply reads, so a landed batch redraws it alone.
const REDRAW = { plugin: 'typeset', key: 'redraw' } as const
// The /typeset status pane's id.
const STATUS_PANE = 'typeset-status'
// What the status pane reads while drawing, so a landed health check redraws it.
const STATUS = { plugin: 'typeset', key: 'status' } as const

// Memo bounds: trees by drawn site, parses by text. Above the replies of a
// long session, as one redraw of the whole transcript must find them all
// (a least-recently-used memo smaller than one pass evicts every entry
// before it is asked again). Entries are small: trees share their pictures.
const DRAWN_LIMIT = 5000
const PLAN_LIMIT = 5000

// Streaming prerender: how often a streaming block is read again at most,
// how much of it is watched, and the hook budget (ms) kept in reserve.
const SCAN_MS = 100
const SCAN_SHARE = 20 // a scan costing c ms (wall clock) waits at least c * SCAN_SHARE ms for the next
const SCAN_LIMIT = 60_000
const SCAN_RESERVE_MS = 5_000
const SCAN_TRIGGER = /\$|\\[)\]]|```|~~~/

const PROMPT_HINT = [
  '# Math and diagrams in replies',
  'This terminal typesets LaTeX: write $...$ inline (no space just inside the $) and $$...$$ on lines of their own for display.',
  'It also draws ```mermaid blocks (flowchart, sequence, state, class, ER; use mindmap for hierarchies and mind maps) as diagrams, and a ```tikz block (one tikzpicture) as a picture.',
  'Never put math in code spans or code blocks.',
].join('\n')

const DEMO = [
  "**Demo.** Euler's identity $e^{i\\pi} + 1 = 0$ links $e$, $i$ and $\\pi$. For samples $x_i \\in \\mathbb{R}$ the mean is $\\bar{x} = \\frac{1}{n}\\sum_{i=1}^{n} x_i$ and the variance $\\sigma^2 = \\mathbb{E}[(X-\\mu)^2]$.",
  'A tall formula makes only its own line taller: $\\sqrt{\\frac{x^2+1}{y}}$, $\\binom{n}{k}$ and $\\int_0^1 f(x)\\,dx$ sit on the text baseline, and the lines around them keep one row.',
  '$$\\int_{-\\infty}^{\\infty} e^{-x^2}\\,dx = \\sqrt{\\pi}$$',
  '中文混排: 设 $f(x) = x^2$, 则导数 $f\'(x) = 2x$, 且 $\\lim_{h \\to 0} \\frac{f(x+h) - f(x)}{h} = f\'(x)$. 中文字符占两格, 换行照样对齐.',
  '- A list item with $\\alpha + \\beta = \\gamma$ wraps under its own text, not under the marker, when the line is long enough to need it.\n- **Bold text with $E = mc^2$ inside** and `inline code` keep their styles.',
  'Prices like $5 and $10 stay text, and `$x$` in code stays code.',
  'A reply may define its own commands: $\\newcommand{\\R}{\\mathbb{R}}\\DeclareMathOperator{\\tr}{tr}$ then $\\tr A \\in \\R$ (the definition itself draws as nothing).',
  '```mermaid\nflowchart LR\n  A[回复 reply] --> B{formula?}\n  B -->|yes| C[picture]\n  B -->|no| D[text]\n```',
].join('\n\n')

const USAGE = [
  `/${COMMAND} status   backend health, settings, cache`,
  `/${COMMAND} on|off   draw formulas or leave replies as the engine draws them (kept across sessions)`,
  `/${COMMAND} demo     show a sample reply with formulas`,
  `/${COMMAND} clear    forget the drawn formulas and draw them again`,
].join('\n')

type Table = Elements['terminal']
type Frame = { bullet?: string; prefix?: string; width: number; marginTop: number }

let settings: Settings = readSettings({})
let enabled: boolean | undefined // the /typeset on|off choice; undefined: settings.enabled
let bulletGlyph = BULLET_OTHER
let maxProseWidth: number | undefined
let lastColumns: number | undefined // the viewport width replies were last drawn at
let warned = false
// A person at a terminal: answers go to a pane or a toast, not to the
// transcript (where a plugin command's text also reaches the model).
let isTerminal = false
let health: Health | undefined // the last health check's answer, for the status pane
let healthCount = 0
let statusCheck: Promise<unknown> | undefined

const isEnabled = () => enabled ?? settings.enabled

// ---------------------------------------------------------------------------
// Drawing

// What a picture says where the terminal cannot draw it (no graphics, tmux
// without passthrough): the Unicode stand-in, cut to the picture's own
// columns so the line it sits in keeps the width it was laid out for.
function altFor(latex: string, columns: number): string {
  const text = toUnicode(latex).replace(/\s+/g, ' ').trim()
  if (stringWidth(text) <= columns) return text || ' '
  let cut = ''
  for (const char of text) {
    if (stringWidth(cut + char) > columns - 1) break
    cut += char
  }

  return `${cut}\u2026`
}

// A Link's address may be 2048 characters once percent-encoded; a longer
// one would refuse the whole tree, so its text is drawn plain.
function linkFits(href: string): boolean {
  try {
    return encodeURI(href).length <= 2048
  } catch {
    return false
  }
}

function textRun(t: Table, run: Extract<Run, { kind: 'text' }>): RenderElement {
  const { Box, Text, Link } = t
  const { style } = run
  const text = (
    <Text
      weight={style.bold ? 'bold' : undefined}
      italic={style.italic}
      strikethrough={style.strike}
      color={style.code ? CODE_COLOR : undefined}
      wrap="truncate"
    >
      {run.text}
    </Text>
  )
  const body = style.link && linkFits(style.link) ? <Link href={style.link}>{text}</Link> : text

  return lifted(t, run.marginTop, body)
}

/**
 * `body` moved down `marginTop` rows. A run on the line's top row is drawn
 * bare: every element costs the engine work on each scroll and redraw, and
 * a long reply has hundreds of runs.
 */
function lifted(t: Table, marginTop: number, body: RenderElement): RenderElement {
  const { Box } = t
  if (marginTop === 0) return body

  return (
    <Box marginTop={marginTop} flexShrink={0}>
      {body}
    </Box>
  )
}

function drawRun(t: Table, run: Run): RenderElement {
  const { Text, Image } = t
  if (run.kind === 'text') return textRun(t, run)
  if (run.kind === 'fallback') {
    return lifted(
      t,
      run.marginTop,
      <Text dimColor={!run.isError} wrap="truncate">
        {run.text}
      </Text>,
    )
  }

  return lifted(
    t,
    run.marginTop,
    <Image source={{ png: run.picture.png }} columns={run.picture.columns} rows={run.picture.rows} alt={altFor(run.latex, run.picture.columns)} />,
  )
}

function drawLine(t: Table, line: Line): RenderElement {
  const { Box } = t
  const [only] = line.runs
  // A line of one run and one row needs no row of its own.
  if (line.runs.length === 1 && only !== undefined && only.kind !== 'image' && only.marginTop === 0 && line.height === 1) return drawRun(t, only)

  return (
    <Box flexDirection="row" flexShrink={0} height={line.height}>
      {line.runs.map(run => drawRun(t, run))}
    </Box>
  )
}

/** The columns a display formula gets: after its indent and, when it opens a list item, the marker. */
function displayRoom(block: Extract<Block, { kind: 'display' }>, width: number): number {
  return Math.max(1, width - block.indent - stringWidth(block.marker ?? ''))
}

function drawDisplay(t: Table, block: Extract<Block, { kind: 'display' }>, state: PictureState, width: number): RenderElement {
  const { Box, Text, Image } = t
  const { latex, indent, marker } = block
  const isPicture = typeof state === 'object' && 'png' in state
  const body = isPicture ? (
    <Image source={{ png: state.png }} columns={state.columns} rows={state.rows} alt={altFor(latex, state.columns)} />
  ) : (
    <Text dimColor={state === 'pending'}>{toUnicode(latex)}</Text>
  )
  const centred = (
    <Box width={displayRoom(block, width)} justifyContent="center" flexShrink={0}>
      {body}
    </Box>
  )
  if (marker === undefined) {
    return (
      <Box marginLeft={indent} flexShrink={0}>
        {centred}
      </Box>
    )
  }

  // The marker sits on the formula's text row, as a list marker beside text.
  return (
    <Box marginLeft={indent} flexDirection="row" flexShrink={0}>
      <Box marginTop={isPicture ? state.textRow : 0} flexShrink={0}>
        <Text>{marker}</Text>
      </Box>
      {centred}
    </Box>
  )
}

/** A fenced block as the engine would have drawn it: a fence longer than any backtick run inside. */
function fenced(source: string, info: string): string {
  const longest = Math.max(2, ...(source.match(/`+/g) ?? []).map(run => run.length))
  const fence = '`'.repeat(longest + 1)

  return `${fence}${info}\n${source}\n${fence}`
}

/** A block that cannot be drawn: its source as the engine draws a code block, and one dim line saying why. */
function shownAsSource(t: Table, info: string, source: string, why: string): RenderElement {
  const { Box, Text, Markdown } = t

  return (
    <Box flexDirection="column" flexShrink={0}>
      <Markdown text={fenced(source, info)} />
      <Text dimColor wrap="truncate">
        {why.replace(/\s+/g, ' ').slice(0, 240)}
      </Text>
    </Box>
  )
}

function segmentColor(role: DiagramRole): { color?: string; dimColor?: boolean } {
  if (role === 'dim') return { dimColor: true }
  // No colour: the terminal's own text colour, as the reply's body text has
  // (the theme's 'text' is a fixed grey that a dark terminal hides).
  if (role === 'text' || role === 'label' || !settings.diagramColor) return {}
  const scheme = theme()

  return { color: scheme ? DIAGRAM_COLORS[scheme][role] : DIAGRAM_FALLBACK }
}

function drawSegment(t: Table, segment: DiagramSegment): RenderElement {
  const { Text } = t

  return <Text {...segmentColor(segment.role)}>{segment.text}</Text>
}

/** A diagram as rows of coloured text, or why it cannot be drawn in `room` columns. */
function artOrReason(t: Table, result: DiagramResult, room: number): RenderElement | string {
  const { Box, Text } = t
  if (!result.ok) return result.reason
  if (result.rows.length > DIAGRAM_ROWS) return `${result.rows.length} rows, more than ${DIAGRAM_ROWS}`
  if (result.columns > room) return `${result.columns} columns wide, ${room} available`

  return (
    <Box flexDirection="column" flexShrink={0}>
      {result.rows.map(row => (
        <Box flexShrink={0} height={1}>
          <Text wrap="truncate">{row.map(segment => drawSegment(t, segment))}</Text>
        </Box>
      ))}
    </Box>
  )
}

/** The ref a mermaid block that the renderer draws as a tree is asked for by. */
function treeRef(source: string): DiagramRef {
  return { latex: source, display: true, kind: 'diagram' }
}

/**
 * A mermaid block: the diagram as rows of coloured text; or, where it cannot
 * be drawn (another kind, a parse failure, too big), its source as the
 * engine draws a code block and one dim line saying why.
 *
 * A mindmap, and a flowchart that looks like a tree, are drawn by the
 * renderer as an outline tree. While that is on its way a mindmap shows its
 * source and a flowchart its beautiful-mermaid drawing (else its source);
 * a flowchart the renderer refuses ("not a tree") or fails on is drawn by
 * beautiful-mermaid, a mindmap shows its source and why.
 */
function drawDiagram(t: Table, block: Extract<Block, { kind: 'mermaid' }>, width: number, diagram: (ref: DiagramRef, width: number) => DiagramState): RenderElement {
  const { Box, Markdown } = t
  const indent = Math.min(block.indent, width - 1)
  const room = Math.max(1, width - indent)
  noteDiagram()
  const route = treeRoute(block.source)
  const state = route ? diagram(treeRef(route.source), room) : undefined
  const tree = typeof state === 'object' && 'lines' in state ? artOrReason(t, treeRows(state.lines), room) : undefined
  const source = () => <Markdown text={fenced(block.source, 'mermaid')} />
  let body: RenderElement
  if (typeof tree === 'object') {
    body = tree
  } else if (route?.kind === 'mindmap') {
    const why = tree ?? (typeof state === 'object' && 'error' in state ? state.error : undefined)
    body = why === undefined ? source() : shownAsSource(t, 'mermaid', block.source, `mermaid mindmap shown as source: ${why.slice(0, 200)}`)
  } else {
    const art = artOrReason(t, renderMermaid(block.source, room), room)
    body = typeof art === 'object' ? art : state === 'pending' ? source() : shownAsSource(t, 'mermaid', block.source, `mermaid diagram shown as source: ${art.slice(0, 200)}`)
  }

  return indent > 0 ? (
    <Box marginLeft={indent} width={room} flexShrink={0}>
      {body}
    </Box>
  ) : (
    body
  )
}

/** The columns a tikz block gets: after its indent. */
function tikzRoom(block: Extract<Block, { kind: 'tikz' }>, width: number): number {
  return Math.max(1, width - Math.min(block.indent, width - 1))
}

/**
 * A ```tikz block: the picture TeX drew of it, centred as a display
 * formula; its source while the picture is on its way; and where it cannot
 * be drawn (no TeX Live, a TeX error), its source and one dim line saying
 * why, as for a mermaid block.
 */
function drawTikz(t: Table, block: Extract<Block, { kind: 'tikz' }>, state: PictureState, width: number): RenderElement {
  const { Box, Image, Markdown } = t
  const indent = Math.min(block.indent, width - 1)
  const room = tikzRoom(block, width)
  const body =
    typeof state === 'object' && 'png' in state ? (
      <Box width={room} justifyContent="center" flexShrink={0}>
        <Image source={{ png: state.png }} columns={state.columns} rows={state.rows} alt={state.columns >= 12 ? '[TikZ picture]' : '[TikZ]'} />
      </Box>
    ) : state === 'pending' ? (
      <Markdown text={fenced(block.source, 'tikz')} />
    ) : (
      shownAsSource(t, 'tikz', block.source, `tikz picture shown as source: ${state.error.slice(0, 200)}`)
    )

  return indent > 0 ? (
    <Box marginLeft={indent} width={room} flexShrink={0}>
      {body}
    </Box>
  ) : (
    body
  )
}

// One blank row where the source had a blank line between two blocks, as the
// engine's markdown draws it; none where it had none ("Steps:\n1. ...", the
// items of a tight list).
function drawBlocks(
  t: Table,
  blocks: Block[],
  picture: (ref: MathRef, width: number) => PictureState,
  diagram: (ref: DiagramRef, width: number) => DiagramState,
  width: number,
): RenderElement[] {
  const { Box, Markdown } = t

  return blocks.map((block, i) => {
    const marginTop = i > 0 && block.gapBefore ? 1 : 0
    let body: RenderElement
    if (block.kind === 'markdown') {
      const indent = Math.min(block.indent ?? 0, width - 1)
      body =
        indent > 0 ? (
          <Box marginLeft={indent} width={width - indent} flexShrink={0}>
            <Markdown text={block.text} />
          </Box>
        ) : (
          <Markdown text={block.text} />
        )
    } else if (block.kind === 'display') {
      body = drawDisplay(t, block, picture({ latex: block.latex, display: true }, displayRoom(block, width)), width)
    } else if (block.kind === 'mermaid') {
      body = drawDiagram(t, block, width, diagram)
    } else if (block.kind === 'tikz') {
      body = drawTikz(t, block, picture({ latex: block.source, display: true, kind: 'tikz' }, tikzRoom(block, width)), width)
    } else {
      // A picture on a continuation line sits after the hang: it gets the rest.
      const room = Math.max(1, width - block.indent - stringWidth(block.marker ?? ''))
      const lines = layoutParagraph(block, ref => picture(ref, room), width)

      // Its lines straight in the block's column: one element fewer per paragraph.
      return (
        <Box flexDirection="column" marginTop={marginTop} flexShrink={0}>
          {lines.map(line => drawLine(t, line))}
        </Box>
      )
    }

    return (
      <Box flexDirection="column" marginTop={marginTop} flexShrink={0}>
        {body}
      </Box>
    )
  })
}

/** The width the text of a block gets, as the engine wraps its own Markdown. */
function frameWidth(columns: number | undefined, gutter: number): number {
  const room = Math.max(10, (columns ?? 80) - gutter)

  return maxProseWidth === undefined ? room : Math.min(room, maxProseWidth)
}

function drawFrame(t: Table, frame: Frame, blocks: RenderElement[]): RenderElement {
  const { Box, Text } = t
  const lead =
    frame.bullet !== undefined ? (
      <Box minWidth={BULLET_GUTTER} flexShrink={0}>
        <Text color="text">{frame.bullet}</Text>
      </Box>
    ) : frame.prefix !== undefined ? (
      <Box minWidth={OUTPUT_GUTTER} flexShrink={0}>
        <Text dimColor>{frame.prefix}</Text>
      </Box>
    ) : null

  return (
    <Box flexDirection="row" alignItems="flex-start" marginTop={frame.marginTop} width="100%">
      {lead}
      <Box flexDirection="column" width={frame.width}>
        {blocks}
      </Box>
    </Box>
  )
}

/**
 * What to do with a block of markdown: draw it (formulas as pictures), hand
 * the engine a text whose formulas became Unicode (all of them sat in
 * headings, tables or quotes, which the engine's markdown draws), or leave it.
 */
type Plan = { blocks: Block[] } | { text: string } | null

const plans = new Map<string, Plan>()

/** Can this text hold a formula or a diagram at all? A scan, no parse. */
function mayDraw(text: string): boolean {
  return /\$\S|\\[([]/.test(text) || /mermaid|tikz/i.test(text)
}

function makePlan(text: string): Plan {
  const blocks = parseMessage(text)
  if (blocks.some(block => block.kind !== 'markdown')) return { blocks }
  const joined = blocks.map(block => (block.kind === 'markdown' ? block.text : '')).join('\n\n')

  return joined !== text && joined.trim() !== '' ? { text: joined } : null
}

/** The plan for a text, parsed once (least recently used parse first out). */
function plan(text: string): Plan {
  const known = plans.get(text)
  if (known !== undefined) {
    plans.delete(text)
    plans.set(text, known)

    return known
  }
  const made = makePlan(text)
  plans.set(text, made)
  if (plans.size > PLAN_LIMIT) plans.delete(plans.keys().next().value as string)

  return made
}

/** Every picture (and tree) the blocks will ask for, at the width each is drawn in (as drawBlocks asks). */
function eachPicture(blocks: Block[], width: number, ask: (ref: MathRef | DiagramRef, width: number) => void) {
  for (const block of blocks) {
    if (block.kind === 'mermaid') {
      const route = treeRoute(block.source)
      if (route) ask(treeRef(route.source), Math.max(1, width - Math.min(block.indent, width - 1)))
    }
    if (block.kind === 'display') ask({ latex: block.latex, display: true }, displayRoom(block, width))
    if (block.kind === 'tikz') ask({ latex: block.source, display: true, kind: 'tikz' }, tikzRoom(block, width))
    if (block.kind !== 'paragraph') continue
    const room = Math.max(1, width - block.indent - stringWidth(block.marker ?? ''))
    for (const inline of block.inlines) if (inline.kind === 'math') ask({ latex: inline.latex, display: false }, room)
  }
}

// What a site was drawn with: its text, frame, and every picture state it
// was given; equal states now mean an equal tree.
type Asked = { ref: MathRef | DiagramRef; width: number; state: PictureState | DiagramState }
type Drawn = { text: string; frameKey: string; asked: Asked[]; tree: RenderElement; isPending: boolean }

const drawn = new Map<string, Drawn>()

function frameKey(frame: Frame): string {
  return [frame.bullet ?? '', frame.prefix ?? '', frame.width, frame.marginTop, theme() ?? '', settings.diagramColor].join('|')
}

/**
 * Draws blocks with their formulas for the site `site` (the instance's
 * requestId, which a landed batch redraws), or returns the tree drawn last
 * time when nothing it showed has changed. `isPending`: some formula is
 * still on its way, so the site must stay subscribed to its redraw.
 */
function drawMarkdown(t: Table, site: string, text: string, blocks: Block[], frame: Frame): Drawn {
  const key = frameKey(frame)
  const last = drawn.get(site)
  if (last && last.text === text && last.frameKey === key && last.asked.every(({ ref, width, state }) => lookup(ref, width, site) === state)) {
    drawn.delete(site)
    drawn.set(site, last)

    return last
  }
  const asked: Asked[] = []
  const picture = (ref: MathRef, width: number) => {
    const state = lookup(ref, width, site)
    asked.push({ ref, width, state })

    return state
  }
  const diagram = (ref: DiagramRef, width: number) => {
    const state = lookup(ref, width, site)
    asked.push({ ref, width, state })

    return state
  }
  // Ask for every formula at once, so one batch carries them all. A tikz
  // block is asked for at its own width as it is drawn (it goes alone).
  for (const ref of mathRefs(blocks)) if (ref.kind !== 'tikz') picture(ref, frame.width)
  const tree = drawFrame(t, frame, drawBlocks(t, blocks, picture, diagram, frame.width))
  const made: Drawn = { text, frameKey: key, asked, tree, isPending: asked.some(({ state }) => state === 'pending') }
  drawn.delete(site)
  drawn.set(site, made)
  if (drawn.size > DRAWN_LIMIT) drawn.delete(drawn.keys().next().value as string)

  return made
}

// ---------------------------------------------------------------------------
// Streaming prerender

type Watched = { text: string; isCooling: boolean; isDirty: boolean }

/** Queues every formula closed so far in a streaming block, at the widths its draw will ask. */
function prerender(text: string) {
  // Not through the plan memo: a partial text is never drawn.
  const found = makePlan(text)
  if (found === null || !('blocks' in found)) return
  const width = lastColumns === undefined ? 255 : frameWidth(lastColumns, BULLET_GUTTER)
  eachPicture(found.blocks, width, (ref, room) => prefetch(ref, room))
}

// ---------------------------------------------------------------------------
// Status

/** How many definitions the `macros` option holds. */
function definitionCount(macros: string): number {
  return (macros.match(/\\(?:(?:re)?newcommand|providecommand|[gex]?def|let|DeclareMathOperator)(?![A-Za-z])/g) ?? []).length
}

/** One line on the TeX fallback: off, missing, or on with the tools it found. */
function texLine(health: Health): string {
  if (!settings.texFallback) return 'TeX fallback: off (texFallback)'
  const tex = health.tex
  if (tex === undefined) return 'TeX fallback: not reported'
  const tools: [string, boolean][] = [
    ['latex', tex.latex],
    ['dvipng', tex.dvipng],
    ['dvips', tex.dvips],
    ['gs', tex.gs],
    ['tikz-cd', tex.tikzCd],
  ]
  const missing = tools.filter(([, found]) => !found).map(([name]) => name)
  if (!tex.latex || !tex.dvipng) return `TeX fallback: not available (missing ${missing.join(', ')}; install TeX Live for package commands and tikz)`
  const found = tools.filter(([, found]) => found).map(([name]) => name)

  return `TeX fallback: ${tex.works === false ? 'on, but its test failed' : 'on'} (found ${found.join(', ')}${missing.length > 0 ? `; missing ${missing.join(', ')}` : ''})`
}

/** The status report, one line each, grouped as the pane shows it: state and renderer, TeX, cell and colour, fonts, setup, cache, diagrams, errors. */
function statusLines(health: Health | undefined): string[] {
  const now = stats()
  const source = enabled === undefined ? 'from settings' : `by /${COMMAND} ${enabled ? 'on' : 'off'}`
  const lines = [`${isEnabled() ? 'on' : 'off'} (${source})`]
  if (health === undefined) {
    lines.push('renderer: checking...')
  } else {
    const cell = health.cell ? `${health.cell.width}x${health.cell.height} px` : 'unknown'
    lines.push(
      `renderer: ${health.ok ? 'ready' : 'NOT READY'}${health.version ? ` (${health.version}, ${health.engine ?? 'unknown engine'})` : ''}`,
      ...health.problems.map(problem => `  problem: ${problem}`),
      texLine(health),
      `cell: ${cell} (setting: ${settings.cell}), baseline ${settings.baseline}, ${settings.ptPx} px/pt, min shrink ${settings.minShrink}, thicken ${settings.thicken}`,
      `colour: ${health.fg ?? 'unknown'} (setting: ${settings.foreground})`,
      `CJK font: ${health.cjkFont ?? 'none found'}${settings.cjkFont ? ` (setting: ${settings.cjkFont})` : ''}`,
    )
  }
  lines.push(
    `packages: ${settings.packages.length > 0 ? settings.packages.join(', ') : 'none extra'}; macros: ${definitionCount(settings.macros)} definitions`,
    `cache: ${now.pictures} pictures, ${now.failed} failed, ${now.pending} pending, ${now.batches} render calls`,
    `diagrams: ${settings.diagramColor ? `colour (${theme() ?? 'theme not known yet'})` : 'monochrome'}; prompt hint: ${settings.promptHint ? 'on' : 'off'}`,
  )
  if (now.errors.length > 0) {
    lines.push('recent errors:')
    for (const { latex, error } of now.errors.slice(-5)) {
      lines.push(`  ${latex.slice(0, 60)}: ${error.slice(0, 160)}`)
    }
  }

  return lines
}

/**
 * Opens the status pane at once, with the last health check's answer, and
 * checks again; the pane is drawn anew when that check lands. False when no
 * pane can be shown here (the call failed, or no surface places it), and the
 * caller prints the report instead.
 */
async function openStatus($: EngineInterface, columns: number): Promise<boolean> {
  if (!isTerminal) return false
  // Body rows the report wants: the title, a blank row, its lines as they
  // wrap a little inside the terminal's width (the frame), a blank row, the hint.
  const room = Math.max(20, columns - 4)
  const rows = statusLines(health).reduce((sum, line) => sum + Math.max(1, Math.ceil(stringWidth(line) / room)), 0) + 4
  let opened: UiOpenResult
  try {
    opened = await $.ui.open({ id: STATUS_PANE, title: `/${COMMAND} status`, focus: true, closeOnEscape: true, holdToasts: true, rows })
  } catch {
    return false
  }
  if (!opened.isPlaced) {
    // An unplaced pane stays open and would show up later, unasked.
    void $.ui.close({ id: STATUS_PANE }).catch(() => {})

    return false
  }
  // One check at a time: a pane opened again meanwhile waits for the same one.
  if (statusCheck === undefined) {
    statusCheck = check($)
      .catch(() => undefined)
      .finally(() => {
        statusCheck = undefined
      })
  }

  return true
}

// ---------------------------------------------------------------------------
// Hooks

export const register: Register = (on, options) => {
  settings = readSettings(options)
  configure(settings, options)
  warned = false
  health = undefined

  on('session.start', async ($, e, next) => {
    isTerminal = e.isInteractive && e.surface === 'terminal'
    // Every health check, the polling's too, is kept for the status pane and
    // draws it again: it opens on the latest answer.
    onHealth(found => {
      health = found
      healthCount += 1
      void $.state.set(STATUS, healthCount).catch(() => {})
    })
    onProblem(reason => {
      if (warned) return
      warned = true
      $.ui.toast(`typeset: formulas shown as text (${reason.slice(0, 200)}). Run /${COMMAND} status for details.`, {
        timeoutMs: 10_000,
      })
    })
    // The worker first: nothing awaited below may keep formulas from drawing.
    // Health at once too, so a broken backend is told once and never run per formula.
    void runWorker($)
    void check($)
    startPolling($)
    await $.command
      .register({
        name: COMMAND,
        description: 'Typeset LaTeX in replies: status, on, off, demo, clear',
        argumentHint: 'status | on | off | demo | clear',
      })
      .catch(() => {})
    const stored = await $.store.get(STORE_ENABLED).catch(() => undefined)
    enabled = typeof stored === 'boolean' ? stored : undefined
    const prose = await $.settings.read({ key: 'maxProseWidth' }).catch(() => undefined)
    maxProseWidth = typeof prose === 'number' && prose >= 40 ? prose : undefined
    void $.process
      .run(['uname', '-s'], { timeoutMs: 5000 })
      .then(({ stdout }) => {
        bulletGlyph = stdout.trim() === 'Darwin' ? BULLET_MAC : BULLET_OTHER
      })
      .catch(() => {})

    return next(e)
  })

  on('prompt.compose', async ($, e, next) => {
    const composed = await next(e)
    if (!settings.promptHint || !isEnabled() || !e.surfaces.includes('terminal')) return composed

    return {
      sections: [
        ...composed.sections.filter(section => section.id !== 'typeset:hint'),
        { id: 'typeset:hint', text: PROMPT_HINT, scope: 'session' as const },
      ],
    }
  })

  // Nothing a subcommand answers reaches the model on a terminal: status opens
  // a pane, on, off and clear a toast. Without a terminal (`claude -p`) they
  // print, as the demo always does (its output is a reply to draw).
  on('command.run', { command: COMMAND }, async ($, e) => {
    const verb = e.args.trim().toLowerCase()
    const tell = (text: string) => {
      if (!isTerminal) return { text }
      $.ui.toast(text) // drawn under the plugin's name

      return {}
    }
    if (verb === 'on' || verb === 'off') {
      enabled = verb === 'on'
      await $.store.set(STORE_ENABLED, enabled).catch(() => {})
      $.ui.invalidate('ui.render')

      return tell(enabled ? 'on: formulas in replies are drawn as pictures.' : 'off: replies are drawn as the engine draws them.')
    }
    if (verb === 'demo') return { text: DEMO }
    if (verb === 'clear') {
      clear()
      drawn.clear()
      plans.clear()
      $.ui.invalidate('ui.render')

      return tell('forgot the drawn formulas; each is drawn again when it shows.')
    }
    if (verb === '' || verb === 'status') {
      if (await openStatus($, e.presentation.columns)) return {}

      return { text: statusLines(await check($)).join('\n') }
    }

    if (isTerminal) return tell(`unknown argument "${verb}": try status, on, off, demo or clear.`)

    return { text: `Unknown argument "${verb}".\n${USAGE}` }
  }).catch(($, e) => {
    if (!isTerminal) return { text: `/${COMMAND} ${e.args} failed.` }
    $.ui.toast(`/${COMMAND} ${e.args} failed.`)

    return {}
  })

  on('ui.render', { component: 'AssistantMessage' }, async ($, e, next) => {
    try {
      if (e.surface !== 'terminal' || !isEnabled() || e.props.isSummary || !mayDraw(e.props.text)) return next(e)
      const found = plan(e.props.text)
      if (found === null) return next(e)
      // Spreading props carries isSummary and onScreen on as received.
      if ('text' in found) return next({ ...e, props: { ...e.props, text: found.text } })
      if (e.viewport?.columns !== undefined) lastColumns = e.viewport.columns
      const gutter = e.props.isFirstOfReply ? BULLET_GUTTER : 0
      const frame: Frame = {
        bullet: e.props.isFirstOfReply ? bulletGlyph : undefined,
        width: frameWidth(e.viewport?.columns, gutter),
        // The engine's addMargin, which the props do not carry: 1 in the
        // transcript as drawn today (a blank row above each block).
        marginTop: 1,
      }
      const made = drawMarkdown($.ui.resolve(e), e.requestId, e.props.text, found.blocks, frame)
      // Subscribes this reply alone to the batch that brings its pictures.
      if (made.isPending) await $.state.get({ ...REDRAW, id: e.requestId }).catch(() => undefined)

      return made.tree
    } catch {
      return next(e)
    }
  }).catch(($, e, next) => next(e))

  // Watches a reply stream by (every chunk passed on as it came, in order)
  // and queues each formula once it is closed, at most every SCAN_MS per
  // block (longer when a scan is costly), so its picture is ready when the
  // finished block is drawn. When a block ends (a chunk of another kind or
  // block, or the stream's end), what closed since its last scan is queued
  // at once and everything queued is sent without waiting: its draw is next.
  on('turn.step', async function* ($, e, next) {
    const stream = next(e)
    if (e.agentId !== undefined || !isEnabled()) return yield* stream
    const blocks = new Map<number, Watched>()
    let isOver = false
    let streaming: Watched | undefined // the text block the last chunk belonged to
    const scan = (watched: Watched) => {
      watched.isDirty = false
      if (isOver || next.budget.remainingMs < SCAN_RESERVE_MS) return
      watched.isCooling = true
      // Wall clock: a rescan runs from a timer, where the hook's budget does not count.
      const started = performance.now()
      try {
        prerender(watched.text)
      } catch {
        // a prerender is only ever a head start
      }
      const cost = performance.now() - started
      $.clock.after(Math.max(SCAN_MS, cost * SCAN_SHARE), () => {
        watched.isCooling = false
        if (watched.isDirty) scan(watched)
      })
    }
    const finish = (watched: Watched | undefined) => {
      if (watched?.isDirty) scan(watched)
      flush()
    }
    try {
      for await (const chunk of stream) {
        // An engine chunk (an envelope, a block's start or end marker) ends nothing by itself.
        const watched = chunk.kind === 'text' ? (blocks.get(chunk.index) ?? { text: '', isCooling: false, isDirty: false }) : chunk.kind === 'engine' ? streaming : undefined
        if (streaming !== undefined && watched !== streaming) finish(streaming)
        streaming = watched
        if (chunk.kind === 'text' && watched !== undefined) {
          blocks.set(chunk.index, watched)
          const seen = watched.text.slice(-1) + chunk.text
          watched.text += chunk.text
          if (watched.text.length <= SCAN_LIMIT && SCAN_TRIGGER.test(seen)) {
            if (watched.isCooling) watched.isDirty = true
            else scan(watched)
          }
        }
        yield chunk
      }
      // What closed since the last scan, once: the block is drawn next.
      for (const watched of blocks.values()) if (watched.isDirty) scan(watched)
      flush()
    } finally {
      isOver = true
    }

    return await stream.result
  })

  // The status pane, laid out as /btw's answer: the command in bold, the
  // report, then a dim key hint.
  on('ui.render', { component: 'Pane', requestId: STATUS_PANE }, async ($, e) => {
    const { Box, Text } = $.ui.resolve(e)
    await $.state.get(STATUS).catch(() => undefined)

    return (
      <Box flexDirection="column">
        <Text bold color="warning">
          /{COMMAND} status
        </Text>
        <Box flexDirection="column" marginTop={1} marginBottom={1}>
          {statusLines(health).map(line => (
            <Text>{line}</Text>
          ))}
        </Box>
        <Text dimColor>Esc close</Text>
      </Box>
    )
  }).catch(($, e, next) => next(e))

  // The demo's output row is drawn by the same code as a reply.
  on('ui.render', { component: 'CommandOutput', props: { command: COMMAND } }, async ($, e, next) => {
    try {
      if (e.surface !== 'terminal' || !isEnabled() || e.props.isErrored || e.props.args.trim().toLowerCase() !== 'demo') return next(e)
      const found = plan(e.props.text)
      if (found === null || !('blocks' in found)) return next(e)
      const frame: Frame = { prefix: OUTPUT_PREFIX, width: frameWidth(e.viewport?.columns, OUTPUT_GUTTER), marginTop: 0 }
      const made = drawMarkdown($.ui.resolve(e), e.requestId, e.props.text, found.blocks, frame)
      if (made.isPending) await $.state.get({ ...REDRAW, id: e.requestId }).catch(() => undefined)

      return made.tree
    } catch {
      return next(e)
    }
  }).catch(($, e, next) => next(e))
}
