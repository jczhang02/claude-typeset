import { describe, expect, mock, test } from 'claude-code/testing'
import type { On } from 'claude-code'

// How the mod finds typeset-render: a local build, a verified cached copy, or
// a download checked against renderer/checksums.json. The host here keeps a
// file system in memory, runs uname, curl, sha256sum (shasum on macOS), chmod,
// mv and rm over it, and runs a file only as the program it stands for.

const PNG = 'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg=='
const GOOD = 'a'.repeat(64) // the sum of the release asset ('good' below)
const SUMS: Record<string, string> = { good: GOOD, tampered: 'b'.repeat(64), local: 'c'.repeat(64) }
const RELEASE = 'https://github.com/jczhang02/claude-typeset/releases/download/v4.0.0'
const LOCAL = '/renderer/target/release/typeset-render'
const CACHED = '/home/u/.cache/typeset/bin/4.0.0/typeset-render'

const HEALTHY = { ok: true, version: 'typeset-render 4.0.0', engine: 'ratex', cell: { width: 19, height: 42 }, fg: '#4c4f69', problems: [] }

function sums(sum: string) {
  return JSON.stringify({
    version: '4.0.0',
    targets: { 'x86_64-unknown-linux-musl': sum, 'aarch64-unknown-linux-musl': sum, 'aarch64-apple-darwin': sum, 'x86_64-apple-darwin': sum },
  })
}

type Setup = {
  uname?: string // what uname -sm prints
  checksums?: string // renderer/checksums.json, undefined for none
  files?: Record<string, string> // path (a leading '/renderer' is under the plugin) to what it holds
  download?: () => string | undefined // what curl fetches; undefined fails as a 404
  gate?: () => Promise<void> // what curl waits for before it ends (a slow download)
  env?: Record<string, string>
}

type World = {
  files: Map<string, string>
  argv: string[][] // every command run, in order
  ran: string[] // every file run as a program
  unverified: string[] // files run that are not the release asset or a local build
  toasts: string[]
  downloads: number
}

function ran(stdout: string, exitCode = 0, stderr = '') {
  return { value: { exitCode, stdout, stderr, isStdoutTruncated: false, isStderrTruncated: false } }
}

function host(on: On, setup: Setup): World {
  const world: World = { files: new Map(), argv: [], ran: [], unverified: [], toasts: [], downloads: 0 }
  const uname = setup.uname ?? 'Linux x86_64'
  mock.store(on)
  mock.env(on, setup.env ?? { HOME: '/home/u' })
  on('command.register', (_, e) => ({ value: { command: e.name } }))
  on('session.start', (_, e) => ({ cwd: e.cwd }))
  on('ui.toast', (_, e) => {
    world.toasts.push(e.text)

    return { value: undefined }
  })
  // Paths under the plugin come in absolute: kept by what follows its root.
  const name = (path: string) => (path.includes('/renderer/') && !path.startsWith('/home/u/') && !path.startsWith('/xdg/') ? path.slice(path.indexOf('/renderer/')) : path)
  for (const [path, content] of Object.entries(setup.files ?? {})) world.files.set(path, content)
  if (setup.checksums !== undefined) world.files.set('/renderer/checksums.json', setup.checksums)
  on('fs.exists', (_, e) => ({ value: world.files.has(name(e.path)) }))
  on('fs.read', (_, e) => {
    const text = world.files.get(name(e.path))

    return text === undefined ? { deny: `ENOENT: ${e.path}` } : { value: text }
  })
  on('fs.list', () => ({ value: [] }))
  on('process.run', async (_, e) => {
    const argv = [...e.argv]
    world.argv.push(argv)
    const [command = '', ...args] = argv
    const last = args.at(-1) ?? ''
    switch (command) {
      case 'uname':
        return ran(args[0] === '-sm' ? `${uname}\n` : `${uname.split(' ')[0]}\n`)
      case 'mkdir':
      case 'chmod':
        return ran('')
      case 'curl': {
        world.downloads += 1
        await setup.gate?.()
        const body = setup.download?.()
        if (body === undefined) return ran('', 22, 'curl: (22) The requested URL returned error: 404')
        world.files.set(args[args.indexOf('-o') + 1] ?? '', body)

        return ran('')
      }
      case 'sha256sum':
      case 'shasum': {
        const content = world.files.get(last)
        if (content === undefined) return ran('', 1, `${command}: ${last}: No such file or directory`)

        return ran(`${SUMS[content] ?? 'd'.repeat(64)}  ${last}\n`)
      }
      case 'mv': {
        const [from = '', to = ''] = args.slice(-2)
        const content = world.files.get(from)
        if (content === undefined) return ran('', 1, `mv: cannot stat '${from}'`)
        world.files.delete(from)
        world.files.set(to, content)

        return ran('')
      }
      case 'rm':
        world.files.delete(last)

        return ran('')
    }
    // Anything else is a file run as typeset-render.
    const content = world.files.get(name(command))
    if (content === undefined) return { deny: `spawn ${command} ENOENT` }
    world.ran.push(name(command))
    if (content !== 'good' && content !== 'local') world.unverified.push(name(command))
    if (args[0] === '--version') return ran('typeset-render 4.0.0\n')
    if (args[0] === '--check') return ran(JSON.stringify(HEALTHY))
    if (args[0] === '--detect') return ran(JSON.stringify({ cell: HEALTHY.cell, fg: HEALTHY.fg }))
    const request = JSON.parse(e.init?.stdin ?? '{}') as { items: { display: boolean }[] }
    const results = request.items.map(item => ({ png: PNG, columns: item.display ? 12 : 4, rows: 1, textRow: 0 }))

    return ran(JSON.stringify({ cell: HEALTHY.cell, fg: HEALTHY.fg, results }))
  })
  on('ui.render', ($, e) => {
    const { Text } = $.ui.resolve(e)

    return <Text>engine</Text>
  })

  return world
}

const reply = (text: string) => ({
  plugin: 'typeset',
  component: 'AssistantMessage' as const,
  requestId: 'msg-1',
  viewport: { columns: 80, rows: 40 },
  props: { text, isFirstOfReply: true },
  surface: 'terminal' as const,
})

const START = { cwd: '/', surface: 'terminal' as const, isInteractive: true }
const RUN = { origin: { kind: 'composer' as const }, presentation: { isFullscreen: false, columns: 80 } }
const STATUS_PANE = {
  plugin: 'typeset',
  component: 'Pane' as const,
  requestId: 'typeset-status',
  props: { title: '/typeset status', isFocused: true, bodyColumns: 76, placement: 'inline' as const, scroll: { offset: 0, bodyRows: 20 }, view: {} },
}

const commands = (world: World, command: string) => world.argv.filter(argv => argv[0] === command)

describe('finding typeset-render', () => {
  test('a local build that runs is used, with nothing downloaded or summed', async ($, on) => {
    const clock = mock.clock(on)
    const world = host(on, { files: { [LOCAL]: 'local' }, checksums: sums(GOOD) })
    await $.session.start(START)
    await clock.advance(0)
    const ui = await $.ui.mount(reply('Has $x$.'))
    await clock.advance(100)
    expect(await ui.findAll({ type: 'Image' })).toHaveLength(1)
    expect(new Set(world.ran)).toEqual(new Set([LOCAL]))
    expect(world.downloads).toBe(0)
    expect(commands(world, 'sha256sum')).toHaveLength(0)
    expect(world.toasts).toHaveLength(0)
    await ui.unmount()
  })

  test('a cached copy whose sum matches is run, without a download', async ($, on) => {
    const clock = mock.clock(on)
    const world = host(on, { files: { [CACHED]: 'good' }, checksums: sums(GOOD) })
    await $.session.start(START)
    await clock.advance(0)
    const ui = await $.ui.mount(reply('Has $x$.'))
    await clock.advance(100)
    expect(await ui.findAll({ type: 'Image' })).toHaveLength(1)
    expect(commands(world, 'sha256sum')).toEqual([['sha256sum', CACHED]])
    expect(new Set(world.ran)).toEqual(new Set([CACHED]))
    expect(world.downloads).toBe(0)
    await ui.unmount()
  })

  test('a cached copy changed since it was put there is deleted and fetched again', async ($, on) => {
    const clock = mock.clock(on)
    const world = host(on, { files: { [CACHED]: 'tampered' }, checksums: sums(GOOD), download: () => 'good' })
    await $.session.start(START)
    await clock.advance(0)
    expect(commands(world, 'rm')[0]).toEqual(['rm', '-f', CACHED])
    expect(world.downloads).toBe(1)
    expect(world.files.get(CACHED)).toBe('good')
    expect(world.unverified).toEqual([])
  })

  test('a download with the right sum is made executable, renamed into place and run', async ($, on) => {
    const clock = mock.clock(on)
    // macOS on Apple silicon, with its own cache folder.
    const world = host(on, { uname: 'Darwin arm64', checksums: sums(GOOD), download: () => 'good', env: { HOME: '/home/u', XDG_CACHE_HOME: '/xdg' } })
    await $.session.start(START)
    const ui = await $.ui.mount(reply('Has $x$ and $y$.'))
    await clock.advance(100)
    expect(await ui.findAll({ type: 'Image' })).toHaveLength(2)

    const file = '/xdg/typeset/bin/4.0.0/typeset-render'
    // The worker, the health check and the poller asked at once: one download.
    expect(world.downloads).toBe(1)
    const curl = commands(world, 'curl')[0] ?? []
    expect(curl.at(-1)).toBe(`${RELEASE}/typeset-render-aarch64-apple-darwin`)
    expect(curl).toContain('-fsSL')
    expect(curl).toContain('--max-time')
    const part = curl[curl.indexOf('-o') + 1] ?? ''
    expect(part.startsWith('/xdg/typeset/bin/4.0.0/')).toBe(true)
    expect(part).not.toBe(file)
    // Summed, then made executable, then renamed: in that order.
    const order = world.argv.map(argv => argv.join(' '))
    const at = (line: string) => order.indexOf(line)
    expect(at(`shasum -a 256 ${part}`)).toBeGreaterThan(at(curl.join(' ')))
    expect(at(`chmod 755 ${part}`)).toBeGreaterThan(at(`shasum -a 256 ${part}`))
    expect(at(`mv -f ${part} ${file}`)).toBeGreaterThan(at(`chmod 755 ${part}`))
    expect(world.files.get(file)).toBe('good')
    expect(world.files.has(part)).toBe(false)
    expect(new Set(world.ran)).toEqual(new Set([file]))
    expect(world.unverified).toEqual([])
    expect(world.toasts).toHaveLength(0)
    await ui.unmount()
  })

  test('a download with the wrong sum is deleted, never run, and told once', async ($, on) => {
    const clock = mock.clock(on)
    const world = host(on, { checksums: sums(GOOD), download: () => 'tampered' })
    await $.session.start(START)
    const ui = await $.ui.mount(reply('Has $x$.'))
    await clock.advance(100)
    expect(await ui.findAll({ type: 'Image' })).toHaveLength(0)
    expect(await ui.find({ type: 'Text', text: /Has/ })).toBeDefined()
    expect(world.files.has(CACHED)).toBe(false)
    expect([...world.files.keys()].filter(path => path.endsWith('.part'))).toEqual([])
    expect(commands(world, 'rm').length).toBeGreaterThan(0)
    expect(commands(world, 'chmod')).toHaveLength(0)
    expect(commands(world, 'mv')).toHaveLength(0)
    expect(world.ran).toEqual([])
    expect(world.toasts).toHaveLength(1)
    expect(world.toasts[0]).toContain('sha256')
    await ui.unmount()
  })

  test('a platform without a release binary says which', async ($, on) => {
    const clock = mock.clock(on)
    const world = host(on, { uname: 'FreeBSD amd64', checksums: sums(GOOD), download: () => 'good' })
    await $.session.start(START)
    await clock.advance(0)
    expect(world.toasts).toHaveLength(1)
    expect(world.toasts[0]).toContain('no renderer for FreeBSD amd64')
    expect(world.downloads).toBe(0)
    expect(world.ran).toEqual([])
  })

  test('before the release PR fills checksums.json there is nothing to download', async ($, on) => {
    const clock = mock.clock(on)
    const world = host(on, { checksums: sums(''), download: () => 'good' })
    await $.session.start(START)
    await clock.advance(0)
    expect(world.toasts).toHaveLength(1)
    expect(world.toasts[0]).toContain('no release binary of typeset-render 4.0.0')
    expect(world.toasts[0]).toContain('cargo build --release')
    expect(world.downloads).toBe(0)
    expect(world.ran).toEqual([])
  })

  test('a missing checksums.json is the same: no release binary to trust', async ($, on) => {
    const clock = mock.clock(on)
    const world = host(on, { download: () => 'good' })
    await $.session.start(START)
    await clock.advance(0)
    expect(world.toasts[0]).toContain('checksums.json is missing')
    expect(world.downloads).toBe(0)
  })

  // A fixed colour and cell size: the formulas asked for while down are filed
  // under the same key after the repair, so nothing may have been kept for them.
  test('a failed download is tried again by the next health check, and formulas then draw', { options: { foreground: '#ff0000', cell: '19x42' } }, async ($, on) => {
    const clock = mock.clock(on)
    let online = false
    const world = host(on, { checksums: sums(GOOD), download: () => (online ? 'good' : undefined) })
    await $.session.start(START)
    const ui = await $.ui.mount(reply('Has $x$.'))
    await clock.advance(100)
    expect(await ui.findAll({ type: 'Image' })).toHaveLength(0)
    expect(world.toasts).toHaveLength(1)
    expect(world.toasts[0]).toContain('could not download')
    expect(world.toasts[0]).toContain('404')

    online = true
    await clock.advance(65_000)
    await clock.advance(100)
    expect(world.downloads).toBe(2)
    expect(await ui.findAll({ type: 'Image' })).toHaveLength(1)
    expect(world.toasts).toHaveLength(1)
    await ui.unmount()
  })

  test('a download refused for its sum is not fetched again at every health check', async ($, on) => {
    const clock = mock.clock(on)
    const world = host(on, { checksums: sums(GOOD), download: () => 'tampered' })
    await $.session.start(START)
    await clock.advance(0)
    expect(world.downloads).toBe(1)
    // Four health checks in the first five minutes, none of them downloads.
    await clock.advance(290_000)
    expect(world.downloads).toBe(1)
    await clock.advance(80_000)
    expect(world.downloads).toBe(2)
    // Then twice as long.
    await clock.advance(480_000)
    expect(world.downloads).toBe(2)
    expect(world.ran).toEqual([])
    expect(world.toasts).toHaveLength(1)
    expect(world.toasts[0]).toContain('sha256')
  })

  // No ui.open answers here, so the pane fails to open and status prints
  // its report, after the health check it waits for.
  test('/typeset status during a download waits for it rather than starting another', async ($, on) => {
    const clock = mock.clock(on)
    let online = false
    let open = () => {}
    const opened = new Promise<void>(resolve => {
      open = resolve
    })
    const world = host(on, {
      checksums: sums(GOOD),
      download: () => (online ? 'good' : undefined),
      gate: () => (online ? opened : Promise.resolve()),
    })
    await $.session.start(START)
    await clock.advance(0)
    expect(world.downloads).toBe(1)
    expect(world.toasts[0]).toContain('could not download')

    // The minute's health check starts a download, which is still running
    // when the status command checks health too.
    online = true
    await clock.advance(65_000)
    expect(world.downloads).toBe(2)
    const status = $.command.run({ ...RUN, command: 'typeset', args: 'status' })
    await clock.advance(0)
    open()
    await status
    await clock.advance(0)
    expect(world.downloads).toBe(2)
    expect(world.files.get(CACHED)).toBe('good')
  })

  test('the status pane opens at once during a download, and is drawn again when it lands', async ($, on) => {
    const clock = mock.clock(on)
    let online = false
    let open = () => {}
    const opened = new Promise<void>(resolve => {
      open = resolve
    })
    const world = host(on, {
      checksums: sums(GOOD),
      download: () => (online ? 'good' : undefined),
      gate: () => (online ? opened : Promise.resolve()),
    })
    on('ui.open', () => ({ value: { isPlaced: true as const } }))
    // The pane reads `status` while drawing; a write to it is what draws the pane again.
    const reads: string[] = []
    let writes = 0
    on('state.get', (_, e, next) => {
      reads.push(e.key)

      return next(e)
    })
    on('state.set', (_, e, next) => {
      if (e.key === 'status') writes += 1

      return next(e)
    })
    await $.session.start(START)
    await clock.advance(0)
    expect(world.downloads).toBe(1)

    online = true
    await clock.advance(65_000)
    expect(world.downloads).toBe(2)
    // The command answers while the download still runs.
    const { text } = await $.command.run({ ...RUN, command: 'typeset', args: 'status' })
    expect(text).toBeUndefined()
    const pane = await $.ui.mount({ ...STATUS_PANE, surface: 'terminal' })
    const lines = async () => (await pane.findAll({ type: 'Text' })).map(found => found.text).join('\n')
    expect(await lines()).toContain('renderer: NOT READY')
    expect(await lines()).toContain('could not download')
    expect(reads).toContain('status')
    const before = writes
    open()
    await clock.advance(0)
    expect(writes).toBeGreaterThan(before)
    expect(await lines()).toContain('renderer: ready (typeset-render 4.0.0, ratex)')
    expect(world.downloads).toBe(2)
    await pane.unmount()
  })
})
