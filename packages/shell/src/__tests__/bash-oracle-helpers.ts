import { tmpdir } from 'node:os'

/**
 * GNU bash on Linux is the oracle for `docs/bash-behavior.md`.
 * Tests spawn `bash --norc --noprofile` with `LC_ALL=C` and a PATH of
 * `/usr/bin:/bin` so nobody edits a table from memory.
 */
const BASH = ['bash', '--norc', '--noprofile'] as const

export const oracle = await probeOracle()

async function probeOracle(): Promise<{ ok: boolean; uname: string; bashVersion: string }> {
  const uname = await runCommand(['uname', '-s'])
  const bashVersion = await runCommand(['bash', '--version'])
  return {
    ok: uname.stdout.trim() === 'Linux' && bashVersion.stdout.includes('GNU bash'),
    uname: uname.stdout.trim(),
    bashVersion: bashVersion.stdout.split('\n')[0] ?? '',
  }
}

export async function runBash(
  script: string,
  options: { cwd?: string; env?: Record<string, string> } = {},
): Promise<{ stdout: string; stderr: string; exitCode: number }> {
  return runCommand([...BASH, '-c', script], options)
}

export async function runCommand(
  argv: string[],
  options: { cwd?: string; env?: Record<string, string> } = {},
): Promise<{ stdout: string; stderr: string; exitCode: number }> {
  const proc = Bun.spawn(argv, {
    cwd: options.cwd,
    env: {
      PATH: '/usr/bin:/bin',
      HOME: options.cwd ?? tmpdir(),
      LC_ALL: 'C',
      LANG: 'C',
      ...options.env,
    },
    stdout: 'pipe',
    stderr: 'pipe',
  })
  const stdout = await new Response(proc.stdout).text()
  const stderr = await new Response(proc.stderr).text()
  const exitCode = await proc.exited
  return { stdout, stderr, exitCode }
}
