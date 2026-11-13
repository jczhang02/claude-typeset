import type { EngineInterface } from 'claude-code'

// Finds the typeset-render program to run, in this order:
//
// 1. a local build, renderer/target/release/typeset-render, when it runs
//    (`cargo build --release` in a clone);
// 2. a copy downloaded before, ~/.cache/typeset/bin/<version>/typeset-render
//    ($XDG_CACHE_HOME respected), when its sha256 is the one in
//    renderer/checksums.json;
// 3. a fresh download of the release asset for this platform with curl,
//    into a temporary name in that folder, checked against the same sum,
//    made executable and only then renamed into place.
//
// A file is never run before its sum matched. The version is checksums.json's
// own: the release workflow writes it with the four sums, so the sums, the
// asset's URL and the hooks that speak to the program all come from one
// commit. A sum that is not 64 hex digits (the placeholders before the
// release PR is merged) means there is no release binary yet.
//
// $.fs reads at most 4 MiB and writes only text, and the program is larger,
// so the download, the sum, the mode and the rename all run as commands. Two
// sessions downloading at once each write a name of their own, and rename(2)
// within one folder puts a whole, verified file in place either way.

const REPO = 'https://github.com/jczhang02/claude-typeset/releases/download'
const PROGRAM = 'typeset-render'
const LOCAL = 'renderer/target/release/typeset-render'
const CHECKSUMS = 'renderer/checksums.json'
// uname -sm, as Linux and macOS print it, to the Rust target built for it.
const TARGETS: Record<string, string> = {
  'Linux x86_64': 'x86_64-unknown-linux-musl',
  'Linux aarch64': 'aarch64-unknown-linux-musl',
  'Linux arm64': 'aarch64-unknown-linux-musl',
  'Darwin arm64': 'aarch64-apple-darwin',
  'Darwin x86_64': 'x86_64-apple-darwin',
}
const VERSION_TIMEOUT_MS = 10_000
const TOOL_TIMEOUT_MS = 30_000
// curl gives up first, so its own message is the one told.
const CURL_MAX_SECONDS = 240
const DOWNLOAD_TIMEOUT_MS = (CURL_MAX_SECONDS + 30) * 1000
// A temporary file this old was left by a session that died while downloading.
const STALE_PART_MS = 3_600_000

type Sums = { version: string; targets: Record<string, unknown> }

// The search under way or done, shared by every caller of one module (the
// worker, --check and the poller start together), so a session downloads once.
// Only a found path is kept; a failure is searched again on the next call.
let found: Promise<string> | undefined

/** The renderer's path, found (and downloaded) once; rejects with the reason it cannot run. */
export function renderer($: EngineInterface): Promise<string> {
  if (found === undefined) {
    const search = locate($)
    found = search
    search.catch(() => {
      if (found === search) found = undefined
    })
  }

  return found
}

/** Searches again on the next call: the program found has stopped running. */
export function forgetRenderer() {
  found = undefined
}

/** The Rust target for `uname -sm` output, or undefined for a platform without a release binary. */
export function targetOf(uname: string): string | undefined {
  return TARGETS[uname.trim().split(/\s+/).slice(0, 2).join(' ')]
}

/** checksums.json's version and the sum for `target`, or why there is no release binary to use. */
export function sumFor(text: string, target: string): { version: string; sum: string } | { failure: string } {
  let sums: Sums
  try {
    sums = JSON.parse(text) as Sums
  } catch {
    return { failure: `${CHECKSUMS} is not JSON` }
  }
  const version = typeof sums?.version === 'string' ? sums.version : ''
  // It goes into a path and a URL: plain version numbers only.
  if (!/^\d+\.\d+\.\d+(?:-[0-9A-Za-z.]+)?$/.test(version)) return { failure: `${CHECKSUMS} names no version` }
  const sum = sums.targets?.[target]
  if (typeof sum !== 'string' || !/^[0-9a-f]{64}$/.test(sum)) {
    return { failure: `no release binary of ${PROGRAM} ${version} for ${target} yet (from a clone: cargo build --release in renderer/)` }
  }

  return { version, sum }
}

function lastLine(text: string): string {
  return text.trim().split('\n').filter(Boolean).pop()?.slice(0, 200) ?? ''
}

async function exists($: EngineInterface, path: string): Promise<boolean> {
  return $.fs.exists(path).catch(() => false)
}

/** Runs a tool to its end; rejects with its last line of output when it fails. */
async function tool($: EngineInterface, argv: string[], timeoutMs = TOOL_TIMEOUT_MS): Promise<string> {
  let run: Awaited<ReturnType<EngineInterface['process']['run']>>
  try {
    run = await $.process.run(argv, { timeoutMs })
  } catch (err) {
    throw new Error(`${argv[0]}: ${String(err).slice(0, 200)}`)
  }
  if (run.exitCode !== 0) throw new Error(`${argv[0]} exited ${run.exitCode}${lastLine(run.stderr) ? `: ${lastLine(run.stderr)}` : ''}`)

  return run.stdout
}

/** What `path --version` printed, or undefined when it does not run. */
async function versionOf($: EngineInterface, path: string): Promise<string | undefined> {
  try {
    const out = (await tool($, [path, '--version'], VERSION_TIMEOUT_MS)).trim()

    return out.startsWith(`${PROGRAM} `) ? out : undefined
  } catch {
    return undefined
  }
}

/** The file's sha256, by sha256sum (Linux) or shasum (macOS). */
async function sha256($: EngineInterface, path: string, os: string): Promise<string> {
  const argv = os === 'Darwin' ? ['shasum', '-a', '256', path] : ['sha256sum', path]
  const sum = /^[0-9a-fA-F]{64}\b/.exec(await tool($, argv))?.[0]
  if (sum === undefined) throw new Error(`${argv[0]} printed no sum`)

  return sum.toLowerCase()
}

async function remove($: EngineInterface, path: string) {
  await tool($, ['rm', '-f', path]).catch(() => {})
}

/** The cache folder: $XDG_CACHE_HOME, else ~/.cache. */
async function cacheHome($: EngineInterface): Promise<string> {
  const xdg = await $.env.get('XDG_CACHE_HOME').catch(() => undefined)
  if (xdg?.startsWith('/')) return xdg.replace(/\/+$/, '')
  const home = await $.env.get('HOME').catch(() => undefined)
  if (!home?.startsWith('/')) throw new Error('HOME is not set, so there is no cache folder to download the renderer to')

  return `${home.replace(/\/+$/, '')}/.cache`
}

/** Deletes temporary files that sessions which died while downloading left in `dir`. */
async function sweep($: EngineInterface, dir: string) {
  try {
    const now = await $.clock.now()
    for (const entry of await $.fs.list(dir)) {
      if (entry.name.endsWith('.part') && now - entry.mtimeMs > STALE_PART_MS) await remove($, `${dir}/${entry.name}`)
    }
  } catch {
    // only tidiness
  }
}

/** Downloads the asset into a name of its own, checks it, and renames it to `file`. */
async function download($: EngineInterface, dir: string, file: string, asset: string, url: string, sum: string, os: string) {
  await tool($, ['mkdir', '-p', dir])
  await sweep($, dir)
  const part = `${dir}/.${PROGRAM}.${Date.now().toString(36)}${Math.random().toString(36).slice(2, 10)}.part`
  try {
    try {
      await tool($, ['curl', '-fsSL', '--proto', '=https', '--connect-timeout', '20', '--max-time', String(CURL_MAX_SECONDS), '--retry', '2', '-o', part, url], DOWNLOAD_TIMEOUT_MS)
    } catch (err) {
      // The asset's name, not its URL: the reason is told in a notice cut at 200 characters.
      throw new Error(`could not download ${asset} (${err instanceof Error ? err.message : String(err)})`)
    }
    const got = await sha256($, part, os)
    if (got !== sum) throw new Error(`the downloaded ${PROGRAM} failed its sha256 check (expected ${sum.slice(0, 12)}…, got ${got.slice(0, 12)}…) and was deleted`)
    await tool($, ['chmod', '755', part])
    await tool($, ['mv', '-f', part, file])
  } finally {
    await remove($, part) // gone already after the rename
  }
}

async function locate($: EngineInterface): Promise<string> {
  const local = `${$.plugin.root}/${LOCAL}`
  if ((await exists($, local)) && (await versionOf($, local)) !== undefined) return local

  const uname = (await tool($, ['uname', '-sm'], VERSION_TIMEOUT_MS)).trim()
  const target = targetOf(uname)
  if (target === undefined) throw new Error(`no renderer for ${uname || 'this platform'}`)
  const os = uname.split(/\s+/)[0] ?? ''
  let text: string
  try {
    text = await $.fs.read(`${$.plugin.root}/${CHECKSUMS}`)
  } catch {
    throw new Error(`${CHECKSUMS} is missing (from a clone: cargo build --release in renderer/)`)
  }
  const wanted = sumFor(text, target)
  if ('failure' in wanted) throw new Error(wanted.failure)
  const { version, sum } = wanted

  const dir = `${await cacheHome($)}/typeset/bin/${version}`
  const file = `${dir}/${PROGRAM}`
  const expected = `${PROGRAM} ${version}`
  if (await exists($, file)) {
    // Checked on every search: a file changed since it was put there is never run.
    if ((await sha256($, file, os)) === sum) {
      const says = await versionOf($, file)
      if (says === expected) return file
      throw new Error(`${file} does not run${says ? ` (it says ${says})` : ''}`)
    }
    await remove($, file)
  }
  const asset = `${PROGRAM}-${target}`
  await download($, dir, file, asset, `${REPO}/v${version}/${asset}`, sum, os)
  const says = await versionOf($, file)
  if (says !== expected) throw new Error(`the downloaded ${PROGRAM} does not run${says ? ` (it says ${says})` : ''}`)

  return file
}
