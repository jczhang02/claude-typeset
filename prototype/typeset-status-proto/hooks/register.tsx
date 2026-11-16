import { atom, read, update } from 'claude-code'
import type { EngineInterface, Register } from 'claude-code'

import type { Variant } from '../types'

// PROTOTYPE, throw away. Question: where should /typeset status show, now that
// the pane docks as a sidebar in fullscreen from 110 columns? Four variants,
// switched by `/tsproto a|b|c|d`, all on fake data shaped like statusLines():
//   a  band dialog     the whole report in the band above the prompt (english-coach)
//   b  band bar        one summary line above the prompt, d expands (code-modernization)
//   c  transcript card a bordered row at the transcript's end ($.ui.place)
//   d  notice lines    dim system lines in the transcript ($.ui.log)
// Each starts with "renderer: checking..." and fills in 1.5 s later, as the
// real health check lands.

const PLUGIN = 'typeset-status-proto'
const COMMAND = 'tsproto'
const CHECK_MS = 1500

const open = atom({ plugin: 'typeset-status-proto', key: 'open' } as const, null)
const isExpanded = atom({ plugin: 'typeset-status-proto', key: 'isExpanded' } as const, false)
const isChecked = atom({ plugin: 'typeset-status-proto', key: 'isChecked' } as const, false)

const HEALTH = [
  'renderer: ready (4.0.0, ratex)',
  'TeX fallback: on (found latex, dvipng, dvips, gs, tikz-cd)',
  'cell: 10x22 px (setting: auto), baseline 0.8, 1.333 px/pt, min shrink 0.7, thicken 0.3',
  'colour: #cdd6f4 (setting: auto)',
  'CJK font: Noto Sans CJK SC',
]
const REST = [
  'packages: none extra; macros: 2 definitions',
  'cache: 128 pictures, 0 failed, 0 pending, 14 render calls',
  'diagrams: colour (dark); prompt hint: on',
]

function lines(checked: boolean): string[] {
  return ['on (from settings)', ...(checked ? HEALTH : ['renderer: checking...']), ...REST]
}

// b: the report as sections of aligned rows. A mark is the row's state
// (green ready, red broken, dim waiting); the note after a value is dim.
type Mark = 'ok' | 'bad' | 'wait'
type Row = { label: string; value: string; note?: string; mark?: Mark }
type Section = { title: string; rows: Row[] }

const LABEL_CELLS = 13
const ERRORS = [{ latex: '\\frac{a}{', error: 'Missing close brace' }]

function sections(checked: boolean): Section[] {
  const renderer: Row[] = checked
    ? [
        { label: 'engine', value: 'ready', note: '4.0.0 · ratex', mark: 'ok' },
        { label: 'TeX', value: 'on', note: 'latex · dvipng · dvips · gs · tikz-cd', mark: 'ok' },
        { label: 'cell', value: '10×22 px', note: 'auto · baseline 0.8 · 1.333 px/pt · shrink 0.7 · thicken 0.3' },
        { label: 'colour', value: '#cdd6f4', note: 'auto' },
        { label: 'CJK font', value: 'Noto Sans CJK SC' },
      ]
    : [{ label: 'engine', value: 'checking…', mark: 'wait' }]

  return [
    { title: 'Renderer', rows: renderer },
    {
      title: 'Setup',
      rows: [
        { label: 'drawing', value: 'on', note: 'from settings' },
        { label: 'packages', value: 'none extra' },
        { label: 'macros', value: '2 definitions' },
        { label: 'diagrams', value: 'colour', note: 'dark' },
        { label: 'prompt hint', value: 'on' },
      ],
    },
    {
      title: 'Cache',
      rows: [
        { label: 'pictures', value: '128', note: `${ERRORS.length} failed · 0 pending · 14 render calls` },
      ],
    },
  ]
}

function summary(checked: boolean): string {
  if (!checked) return 'on · renderer checking…'
  const errors = ERRORS.length === 0 ? 'no errors' : `${ERRORS.length} error${ERRORS.length > 1 ? 's' : ''}`

  return `on · ready 4.0.0 · TeX on · 128 pictures · ${errors}`
}

async function startCheck($: EngineInterface) {
  await update($, isChecked, () => false)
  $.clock.after(CHECK_MS, () => {
    void update($, isChecked, () => true)
  })
}

export const register: Register = on => {
  on('session.start', async ($, e, next) => {
    await $.command
      .register({ name: COMMAND, description: 'PROTOTYPE: /typeset status shown four ways', argumentHint: 'a | b | c | d' })
      .catch(() => {})

    return next(e)
  })

  // Variants a and b leave the band when the person sends anything else.
  on('prompt.submit', async ($, e, next) => {
    const typed = ['composer', 'bridge'].includes(e.origin.kind)
    if (!typed || e.command?.name === COMMAND) return next(e)
    await update($, open, () => null)
    await update($, isExpanded, () => false)

    return next(e)
  })

  on('command.run', { command: COMMAND }, async ($, e) => {
    const verb = (e.args.trim().toLowerCase() || 'a') as Variant
    if (!['a', 'b', 'c', 'd'].includes(verb)) {
      $.ui.toast('try /tsproto a, b, c or d')

      return {}
    }
    await startCheck($)
    if (verb === 'a' || verb === 'b') {
      await update($, open, () => verb)
    } else if (verb === 'c') {
      await $.ui.place({ id: `status-${await $.clock.now()}` })
    } else {
      // A notice row cannot redraw: wait for the check, then log once.
      $.clock.after(CHECK_MS, () => {
        $.ui.log(`/typeset status`)
        for (const line of lines(true)) $.ui.log(`  ${line}`)
      })
    }

    return {}
  })

  on('ui.render', { component: 'AbovePrompt' }, async ($, e, next) => {
    const variant = await read($, open)
    if (variant === null || e.props.hasSurvey || e.surface !== 'terminal') return next(e)
    const checked = await read($, isChecked)
    const { Box, Text, Button } = $.ui.resolve(e)
    const close = () => update($, open, () => null)
    const recheck = () => startCheck($)

    // a: the report as a dialog would show it, keys under it.
    if (variant === 'a') {
      return (
        <Box flexDirection="column">
          <Text bold color="warning">
            /typeset status
          </Text>
          <Box flexDirection="column" marginTop={1} marginBottom={1}>
            {lines(checked).map(line => (
              <Text wrap="wrap">{line}</Text>
            ))}
          </Box>
          <Box>
            <Button key="close" hotkey="x" plain label="close" onPress={close} />
            <Text dimColor>  </Text>
            <Button key="recheck" hotkey="r" plain label="check again" onPress={recheck} />
            <Text dimColor>  · ctrl+x tab for keys · your next prompt closes it</Text>
          </Box>
        </Box>
      )
    }

    // b: one line; d opens the rest in place.
    const expanded = await read($, isExpanded)

    const glyph = (mark: Mark | undefined) =>
      mark === undefined ? (
        <Text>{'  '}</Text>
      ) : (
        <Text color={mark === 'ok' ? 'success' : mark === 'bad' ? 'error' : undefined} dimColor={mark === 'wait'}>
          {mark === 'wait' ? '○ ' : '● '}
        </Text>
      )
    const state: Mark = !checked ? 'wait' : ERRORS.length > 0 ? 'bad' : 'ok'

    return (
      <Box flexDirection="column">
        <Box>
          <Text bold color="warning">
            typeset{' '}
          </Text>
          {glyph(state)}
          <Text wrap="truncate">{summary(checked)}</Text>
          <Text dimColor>{'   '}</Text>
          <Button key="details" hotkey="d" plain label={expanded ? 'less' : 'details'} onPress={() => update($, isExpanded, v => !v)} />
          <Text dimColor>{'  '}</Text>
          <Button key="close" hotkey="x" plain role="dismiss" label="close" onPress={close} />
        </Box>
        {expanded ? (
          <Box flexDirection="column" marginLeft={2}>
            {sections(checked).map(section => {
              // Notes start in one column per section: values padded to the widest one that has a note.
              const cells = Math.max(0, ...section.rows.filter(row => row.note).map(row => row.value.length))

              return (
              <Box flexDirection="column" marginTop={1}>
                <Text bold>{section.title}</Text>
                {section.rows.map(row => (
                  <Box>
                    <Box width={LABEL_CELLS} flexShrink={0}>
                      <Text dimColor>{row.label}</Text>
                    </Box>
                    {glyph(row.mark)}
                    <Box flexShrink={1}>
                      <Text wrap="wrap">
                        {row.note ? row.value.padEnd(cells) : row.value}
                        {row.note ? <Text dimColor>{`   ${row.note}`}</Text> : null}
                      </Text>
                    </Box>
                  </Box>
                ))}
              </Box>
              )
            })}
            {checked && ERRORS.length > 0 ? (
              <Box flexDirection="column" marginTop={1}>
                <Text bold>Recent errors</Text>
                {ERRORS.map(({ latex, error }) => (
                  <Box>
                    <Box width={LABEL_CELLS} flexShrink={0}>
                      <Text color="permission" wrap="truncate">
                        {latex}
                      </Text>
                    </Box>
                    {glyph('bad')}
                    <Box flexShrink={1}>
                      <Text wrap="wrap">{error}</Text>
                    </Box>
                  </Box>
                ))}
              </Box>
            ) : null}
          </Box>
        ) : null}
      </Box>
    )
  })

  // c: a card in the transcript; the model never reads it, and it redraws as the check lands.
  on('ui.render', { component: 'Placed' }, async ($, e, next) => {
    if (e.props.plugin !== PLUGIN || !e.props.id.startsWith('status-')) return next(e)
    const checked = await read($, isChecked)
    const { Box, Text } = $.ui.resolve(e)

    return (
      <Box flexDirection="column" borderStyle="round" borderColor="warning" paddingX={1} marginTop={1}>
        <Text bold color="warning">
          /typeset status
        </Text>
        {lines(checked).map(line => (
          <Text wrap="wrap">{line}</Text>
        ))}
      </Box>
    )
  })
}
