import { memoryCommandStorage as memoryStorage } from '@demicodes/shell/testing'
import {
  type Command,
  type Host,
  type NativeExecutor
} from '@demicodes/shell'
import { z } from 'zod'

const notesSchema = z.array(z.string())

import { nativePackageSchema, NATIVE_TARGETS } from '@demicodes/command-protocol'

export const testPackage = nativePackageSchema.parse({
  id: 'fixture.commands', version: 'test', protocolVersion: 1,
  operations: ['copy', 'echo'],
  targets: Object.fromEntries(NATIVE_TARGETS.map(target => [target, { sha256: '0'.repeat(64), size: 1 }])),
})

/** Explicit fake executor checks the loader's adapter boundary; no code is loaded. */
export function testNative(host: Host): NativeExecutor {
  return async context => {
    if (context.binding.operation === 'copy') {
      let bytes = await host.fs.readFile(String(context.args.from), { cwd: context.cwd })
      if (context.args.upper)
        bytes = new TextEncoder().encode(new TextDecoder().decode(bytes).toUpperCase())
      await host.fs.writeFile(String(context.args.to), bytes, { cwd: context.cwd })
      await context.stdout(`copied ${context.args.from} -> ${context.args.to} in ${context.cwd}\n`)
      return { exitCode: 0 }
    }
    for await (const chunk of context.stdin)
      await context.stdout(chunk)
    await context.stderr(`env HOME=${context.env.HOME ?? ''}\n`)
    return { exitCode: Number(context.args.code ?? 0) }
  }
}

export function testRoots(): Command[] {
  return [
    {
      name: 'scout',
      summary: 'A second root beside demi.',
      subcommands: [
        {
          name: 'copy',
          kind: 'native',
          binding: { package: testPackage.id, operation: 'copy' },
          summary: 'Copy a file.',
          input: {
            from: z.string().describe('Source path'),
            to: z.string().describe('Destination path'),
            upper: z.boolean().optional().describe('Uppercase the content'),
          },
          positionals: ['from', 'to'],
        },
        {
          name: 'echo',
          kind: 'native',
          binding: { package: testPackage.id, operation: 'echo' },
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
              stdinField: 'text',
              output: { json: z.object({ count: z.number() }) },
              run: async ({ parsed, io, storage }) => {
                const notes = (notesSchema.nullable().parse(await storage.readJson('notes'))
                  ?? []).concat(String(parsed.values.text))
                await storage.writeJson('notes', notes)
                await io.stdout(parsed.json ? JSON.stringify({
                  count: notes.length
                }) : `${notes.length} notes\n`)
                return { exitCode: 0 }
              },
            },
          ],
        },
      ],
    },
  ]
}


export { memoryStorage }
