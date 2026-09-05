import { pathArg, runtimeModule, type Command, type CommandStorage } from '@demicodes/shell'
import { z } from 'zod'

/** Test roots: a `runtime` leaf, an `rpc` leaf that reads storage, and a group. */

export const COPY_MODULE = `import type { CommandContext, CommandResult } from '@demicodes/shell'

interface Args { from: string; to: string; upper?: boolean }

export default async function copy(ctx: CommandContext<Args>): Promise<CommandResult> {
  let bytes = await ctx.fs.readFile(ctx.args.from, { cwd: ctx.cwd })
  if (ctx.args.upper) bytes = new TextEncoder().encode(new TextDecoder().decode(bytes).toUpperCase())
  await ctx.fs.writeFile(ctx.args.to, bytes, { cwd: ctx.cwd })
  await ctx.stdout(\`copied \${ctx.args.from} -> \${ctx.args.to} in \${ctx.cwd}\\n\`)
  return { exitCode: 0 }
}
`

export const ECHO_STDIN_MODULE = `import type { CommandContext, CommandResult } from '@demicodes/shell'

export default async function echo(ctx: CommandContext): Promise<CommandResult> {
  for await (const chunk of ctx.stdin) await ctx.stdout(chunk)
  await ctx.stderr(\`env HOME=\${ctx.env.HOME ?? ''}\\n\`)
  return { exitCode: Number(ctx.args.code ?? 0) }
}
`

export function testRoots(): Command[] {
  return [
    {
      name: 'scout',
      summary: 'A second root beside demi.',
      subcommands: [
        {
          name: 'copy',
          kind: 'runtime',
          module: runtimeModule(COPY_MODULE),
          summary: 'Copy a file.',
          input: {
            from: pathArg(z.string().describe('Source path')),
            to: pathArg(z.string().describe('Destination path')),
            upper: z.boolean().optional().describe('Uppercase the content'),
          },
          positionals: ['from', 'to'],
        },
        {
          name: 'echo',
          kind: 'runtime',
          module: runtimeModule(ECHO_STDIN_MODULE),
          summary: 'Echo stdin.',
          input: { code: z.number().int().optional().describe('Exit code') },
        },
        {
          name: 'note',
          summary: 'Notes kept in session storage.',
          subcommands: [
            {
              name: 'add',
              kind: 'rpc',
              summary: 'Add a note.',
              input: { text: z.string().describe('Note text') },
              positionals: ['text'],
              stdinField: 'text',
              output: { json: z.object({ count: z.number() }) },
              run: async ({ parsed, io, storage }) => {
                const notes = ((await storage.readJson<string[]>('notes')) ?? []).concat(String(parsed.values.text))
                await storage.writeJson('notes', notes)
                await io.stdout(parsed.json ? JSON.stringify({ count: notes.length }) : `${notes.length} notes\n`)
                return { exitCode: 0 }
              },
            },
          ],
        },
      ],
    },
  ]
}

export function memoryStorage(): CommandStorage {
  const data = new Map<string, unknown>()
  return {
    readJson: async <T>(key: string) => (data.get(key) as T | undefined) ?? null,
    writeJson: async (key, value) => void data.set(key, value),
    delete: async (key) => void data.delete(key),
    list: async (prefix) => [...data.keys()].filter((key) => key.startsWith(prefix)),
  }
}

export function transpile(source: string): string {
  return new Bun.Transpiler({ loader: 'ts', target: 'browser' }).transformSync(source)
}
