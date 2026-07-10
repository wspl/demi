import { mkdtemp } from 'node:fs/promises'
import { join } from 'node:path'
import { tmpdir } from 'node:os'
import { expect, test } from 'bun:test'
import { z } from 'zod'
import { BashEnvironment, CommandRegistry, type Command } from '../index'
import { LocalHost } from '@demicodes/host-local'

const nestedSpec: Command = {
  name: 'larkclaw',
  summary: 'Unified entry for platform capabilities.',
  subcommands: [
    {
      name: 'watch',
      summary: 'Background pollers.',
      subcommands: [
        {
          name: 'create',
          summary: 'Create a poller.',
          input: {
            id: z.string().describe('Poller id'),
            body: z.string().describe('JSON body'),
          },
          positionals: ['id'],
          stdinField: 'body',
          examples: ["larkclaw watch create my-id <<'EOF'\n{}\nEOF"],
          run: async ({ parsed, io }) => {
            await io.stdout(`created ${parsed.values.id} body=${parsed.values.body}`)
            return { exitCode: 0 }
          },
        },
      ],
    },
  ],
}

function makeEnv(root: string): BashEnvironment {
  const commands = new CommandRegistry()
  commands.register(nestedSpec)
  return new BashEnvironment({
    host: new LocalHost(root),
    commands,
    initialEnv: { PATH: process.env.PATH ?? '' },
  })
}

test('nested command leaves execute through the interpreter with heredoc stdin', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-nested-exec-'))
  const env = makeEnv(root)
  const result = await env.exec({
    script: "larkclaw watch create my-id <<'EOF'\n{\"a\":1}\nEOF",
    agentSessionId: 'conv-nested',
  })
  expect(result.status).toBe('exited')
  if (result.status !== 'exited') throw new Error('expected exited')
  expect(result.exitCode).toBe(0)
  expect(result.stdout.delta).toBe('created my-id body={"a":1}\n')
})

test('group-level --help renders subtree help through the interpreter', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-nested-help-'))
  const env = makeEnv(root)
  const result = await env.exec({ script: 'larkclaw watch --help', agentSessionId: 'conv-nested-help' })
  expect(result.status).toBe('exited')
  if (result.status !== 'exited') throw new Error('expected exited')
  expect(result.exitCode).toBe(0)
  expect(result.stdout.delta).toContain('larkclaw watch: Background pollers.')
  expect(result.stdout.delta).toContain('  larkclaw watch create')
})
