/**
 * Regenerates the goldens by running every corpus case through real GNU bash.
 *
 *   bun run packages/tinybash/src/__tests__/corpus/generate.ts [case-name ...]
 *
 * The bash side is `TINYBASH_BASH` (default: `bash` on Linux, `limactl shell
 * fc -- bash` elsewhere), run with `-s` so the whole driver travels on stdin.
 * Each case gets a fresh temp directory holding the fixture as `home`, the
 * stub roots in `bin`, and runs `bash -c` with a fixed environment (C locale,
 * UTC); the home path is normalized to `@HOME@` in the recorded output.
 */
import { mkdirSync, writeFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { CASES, type CorpusCase } from './cases'
import { fixtureShell, stubRootShell } from './fixture'

export interface Golden {
  exit: number
  stdout: string
  stderr: string
  user: string
  group: string
}

export const GOLDENS_DIR = join(dirname(fileURLToPath(import.meta.url)), 'goldens')
export const ROOT_NAMES = ['demi', 'myagent']
export const HOME_TOKEN = '@HOME@'

function bashCommand(): string[] {
  const override = process.env.TINYBASH_BASH
  if (override) return override.split(' ')
  return process.platform === 'linux' ? ['bash'] : ['limactl', 'shell', 'fc', '--', 'bash']
}

/** The driver that builds one case's world and prints its results in a fixed frame. */
export function driver(testCase: CorpusCase): string {
  const marker = '__TINYBASH_CORPUS__'
  return [
    'set -u',
    'H=$(mktemp -d /tmp/tinybash.XXXXXX)',
    'mkdir -p "$H/home" "$H/bin"',
    ...ROOT_NAMES.map((name) => `cat > "$H/bin/${name}" <<'${marker}'\n${stubRootShell()}${marker}\nchmod 755 "$H/bin/${name}"`),
    'cd "$H/home"',
    fixtureShell(),
    `cat > "$H/script" <<'${marker}'\n${testCase.script}\n${marker}`,
    // `$(cat)` would strip trailing newlines; read the file as bash -c's argument via a variable.
    'script=$(cat "$H/script"; printf x); script=${script%x}; script=${script%$\'\\n\'}',
    'cd "$H/home"',
    'env -i HOME="$H/home" PATH="$H/bin:/usr/bin:/bin" LC_ALL=C TZ=UTC bash -c "$script" >"$H/out" 2>"$H/err" </dev/null',
    'code=$?',
    `printf '%s\\n' "${marker}"`,
    'printf "exit %s\\n" "$code"',
    'printf "home %s\\n" "$H/home"',
    'printf "user %s\\n" "$(id -un)"',
    'printf "group %s\\n" "$(id -gn)"',
    'printf "stdout %s\\n" "$(base64 -w0 < "$H/out")"',
    'printf "stderr %s\\n" "$(base64 -w0 < "$H/err")"',
    'rm -rf "$H"',
    '',
  ].join('\n')
}

export async function runOnBash(testCase: CorpusCase): Promise<Golden> {
  const proc = Bun.spawn([...bashCommand(), '-s'], { stdin: 'pipe', stdout: 'pipe', stderr: 'pipe' })
  proc.stdin.write(driver(testCase))
  await proc.stdin.end()
  const [stdout, stderr] = await Promise.all([new Response(proc.stdout).text(), new Response(proc.stderr).text()])
  const exit = await proc.exited
  const frame = stdout.split('__TINYBASH_CORPUS__\n')[1]
  if (exit !== 0 || frame === undefined) throw new Error(`driver failed for ${testCase.name}: exit ${exit}\n${stdout}\n${stderr}`)
  const fields = new Map<string, string>()
  for (const line of frame.split('\n')) {
    const space = line.indexOf(' ')
    if (space === -1) continue
    fields.set(line.slice(0, space), line.slice(space + 1))
  }
  const home = fields.get('home')!
  const decode = (b64: string) => normalizeHome(latin1(Buffer.from(b64, 'base64')), home)
  return {
    exit: Number(fields.get('exit')),
    stdout: decode(fields.get('stdout') ?? ''),
    stderr: decode(fields.get('stderr') ?? ''),
    user: fields.get('user')!,
    group: fields.get('group')!,
  }
}

export function normalizeHome(text: string, home: string): string {
  return text.split(home).join(HOME_TOKEN)
}

export function latin1(bytes: Uint8Array): string {
  let out = ''
  for (const b of bytes) out += String.fromCharCode(b)
  return out
}

export function goldenPath(name: string): string {
  return join(GOLDENS_DIR, `${name}.json`)
}

if (import.meta.main) {
  const only = new Set(process.argv.slice(2))
  mkdirSync(GOLDENS_DIR, { recursive: true })
  const selected = CASES.filter((c) => only.size === 0 || only.has(c.name))
  let failures = 0
  // Concurrent `limactl shell` sessions interleave their streams; raise only with a local bash.
  const limit = Number(process.env.TINYBASH_GENERATE_JOBS ?? 1)
  let index = 0
  await Promise.all(
    Array.from({ length: limit }, async () => {
      while (index < selected.length) {
        const testCase = selected[index++]!
        try {
          const golden = await runOnBash(testCase)
          writeFileSync(goldenPath(testCase.name), `${JSON.stringify(golden, null, 2)}\n`)
          console.log(`${testCase.name}: exit ${golden.exit}`)
        } catch (error) {
          failures++
          console.error(String(error))
        }
      }
    }),
  )
  if (failures > 0) process.exit(1)
}
