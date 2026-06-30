import { mkdtemp } from 'node:fs/promises'
import { join } from 'node:path'
import { tmpdir } from 'node:os'
import { expect, test } from 'bun:test'
import { BashEnvironment, CommandRegistry, type CommandSpec } from '../index'
import { LocalHost } from '@demicodes/host-local'

const screenshotSpec: CommandSpec = {
  name: 'screenshot',
  summary: 'Capture a fake screenshot.',
  subcommands: [
    {
      name: 'take',
      summary: 'Emit a status line on stdout and an image asset out-of-band.',
      examples: [],
      run: async (ctx) => {
        await ctx.io.stdout('captured viewport\n')
        await ctx.io.asset({ type: 'image', mediaType: 'image/png', data: 'AAAA' })
        return { exitCode: 0 }
      },
    },
  ],
}

function makeEnv(root: string, shellId: string): BashEnvironment {
  const commands = new CommandRegistry()
  commands.register(screenshotSpec)
  return new BashEnvironment({
    host: new LocalHost(root),
    commands,
    shellIdFactory: () => shellId,
    initialEnv: { PATH: process.env.PATH ?? '' },
  })
}

test('registered command assets reach ShellCommandSnapshot.assets, separate from stdout text', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-assets-'))
  const env = makeEnv(root, 'shell-assets')

  const result = await env.exec({ script: 'screenshot take' })
  expect(result.status).toBe('exited')
  if (result.status !== 'exited') throw new Error('expected exited result')

  // text rides stdout; the image rides the structured asset channel
  expect(result.stdout.delta).toContain('captured viewport')
  expect(result.stdout.delta).not.toContain('AAAA')
  expect(result.assets).toEqual([{ type: 'image', mediaType: 'image/png', data: 'AAAA' }])
})

test('a command that emits no asset leaves snapshot.assets undefined', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-assets-none-'))
  const env = makeEnv(root, 'shell-no-assets')

  const result = await env.exec({ script: 'echo hi' })
  expect(result.status).toBe('exited')
  if (result.status !== 'exited') throw new Error('expected exited result')
  expect(result.assets).toBeUndefined()
})
