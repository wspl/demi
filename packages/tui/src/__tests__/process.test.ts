import { expect, test } from 'bun:test'
import { spawn } from 'node:child_process'

test('TUI process entry prints help without opening a provider session', async () => {
  const result = await runTuiProcess(['--help'])

  expect(result.exitCode).toBe(0)
  expect(result.stderr).toBe('')
  expect(result.stdout).toContain('Usage: bun run tui -- [workspace] [options]')
  expect(result.stdout).toContain('/input <shellId> <text>')
  expect(result.stdout).not.toContain('claude runtime:')
  expect(result.stdout).not.toContain('Session opened')
})

async function runTuiProcess(args: string[]): Promise<{ stdout: string; stderr: string; exitCode: number | null }> {
  const child = spawn(process.execPath, ['run', 'packages/tui/src/index.ts', ...args], {
    cwd: process.cwd(),
    stdio: ['ignore', 'pipe', 'pipe'],
  })
  const [stdout, stderr, exitCode] = await Promise.all([
    collect(child.stdout),
    collect(child.stderr),
    new Promise<number | null>((resolve) => {
      child.once('close', (code) => resolve(code))
      child.once('error', () => resolve(null))
    }),
  ])
  return { stdout, stderr, exitCode }
}

async function collect(stream: NodeJS.ReadableStream): Promise<string> {
  const chunks: Uint8Array[] = []
  for await (const chunk of stream) chunks.push(chunk instanceof Uint8Array ? chunk : Buffer.from(String(chunk)))
  return Buffer.concat(chunks).toString('utf8')
}
