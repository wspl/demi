import { mkdtemp, rm } from 'node:fs/promises'
import { join } from 'node:path'
import { tmpdir } from 'node:os'
import { expect, test } from 'bun:test'
import { LocalHost } from '@demicodes/host-local'
import { waitFor } from '@demicodes/utils'
import { BashEnvironment, CommandRegistry } from '../index'

function createGate() {
  let release!: () => void
  const promise = new Promise<void>((resolve) => { release = resolve })
  return { promise, release, hasEntered: false }
}

async function waitUntilSettled(env: BashEnvironment, commandId: string): Promise<void> {
  for (let attempt = 0; attempt < 200; attempt++) {
    const status = await env.status({ commandId, stdoutOffset: 0, stderrOffset: 0, outputOffset: 0 })
    if (status.status !== 'running') return
    await new Promise((resolve) => setTimeout(resolve, 5))
  }
  throw new Error('command did not settle')
}

for (const prefix of ["printf 'BEFORE\\n'", "/usr/bin/printf 'BEFORE\\n'", 'receipt']) {
  test(`script output remains cumulative across foreground jobs after ${prefix}`, async () => {
    const root = await mkdtemp(join(tmpdir(), 'demi-script-output-'))
    const host = new LocalHost(root)
    const commands = new CommandRegistry()
    const first = createGate()
    const second = createGate()
    commands.register({ name: 'receipt', summary: 'Receipt', run: async ({ io }) => { await io.stdout('BEFORE\n'); return { exitCode: 0 } } })
    for (const [name, gate, output] of [['first', first, 'FIRST-1234567890\n'], ['second', second, '二🙂\n']] as const) {
      commands.register({ name, summary: name, run: async ({ io }) => {
        await io.stdout(output)
        await io.stderr(`${name}-error\n`)
        gate.hasEntered = true
        await gate.promise
        return { exitCode: 1 }
      } })
    }
    const env = new BashEnvironment({ host, commands, initialEnv: { PATH: '/usr/bin:/bin' } })
    try {
      const initial = await env.exec({ script: `${prefix}; first; second; printf 'AFTER\\n'`, timeoutMs: 100 })
      expect(first.hasEntered).toBe(true)
      expect(initial.status).toBe('running')
      expect(initial.stdout.delta).toBe('BEFORE\nFIRST-1234567890\n')
      first.release()
      await waitFor(() => second.hasEntered)
      const next = await env.status({ commandId: initial.commandId })
      expect(next.stdout.delta).toBe('二🙂\n')
      expect(next.stderr.delta).toBe('second-error\n')
      expect(next.output.offset).toBeGreaterThan(initial.output.offset)
      second.release()
      await waitUntilSettled(env, initial.commandId)
      const final = await env.status({ commandId: initial.commandId })
      expect(final.status === 'exited' && final.exitCode).toBe(0)
      expect(final.stdout.delta).toBe('AFTER\n')
      expect(initial.output.text + next.output.text + final.output.text).toBe('BEFORE\nFIRST-1234567890\nfirst-error\n二🙂\nsecond-error\nAFTER\n')
      expect(await host.fs.readFile(final.stdout.path).then((bytes) => new TextDecoder().decode(bytes))).toBe('BEFORE\nFIRST-1234567890\n二🙂\nAFTER\n')
    } finally {
      first.release()
      second.release()
      await env.disposeAllShells()
      await rm(root, { recursive: true, force: true })
    }
  })
}

for (const script of ['secret | cat', 'value=$(secret)', 'secret > hidden.txt', '{ secret; } > hidden.txt']) {
  test(`captured output is not exposed while ${script} runs`, async () => {
    const root = await mkdtemp(join(tmpdir(), 'demi-captured-output-'))
    const gate = createGate()
    const commands = new CommandRegistry()
    commands.register({ name: 'secret', summary: 'Captured producer', run: async ({ io }) => {
      await io.stdout('hidden\n')
      gate.hasEntered = true
      await gate.promise
      return { exitCode: 0 }
    } })
    const env = new BashEnvironment({ host: new LocalHost(root), commands, initialEnv: { PATH: '/usr/bin:/bin' } })
    try {
      const initial = await env.exec({ script: `printf 'prefix\\n'; ${script}; printf 'done\\n'`, timeoutMs: 100 })
      expect(gate.hasEntered).toBe(true)
      expect(initial.stdout.delta).toBe('prefix\n')
      gate.release()
      await waitUntilSettled(env, initial.commandId)
      const final = await env.status({ commandId: initial.commandId, stdoutOffset: 0 })
      expect(final.stdout.delta).toBe(script.includes('|') ? 'prefix\nhidden\ndone\n' : 'prefix\ndone\n')
    } finally {
      gate.release()
      await env.disposeAllShells()
      await rm(root, { recursive: true, force: true })
    }
  })
}
