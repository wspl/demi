import { mkdir, writeFile } from 'node:fs/promises'
import { nativePackageSchema } from '@demicodes/command-protocol'
import { parseCommandInput, renderCommandHelp, type Command } from '@demicodes/shell'
import { z } from 'zod'
import packageFixture from '../../command-protocol/tests/fixtures/package.json'
import { buildManifest } from '../src/manifest/build'

const descriptor = nativePackageSchema.parse(packageFixture.descriptor)
const root: Command = {
  name: 'fixture', summary: 'CLI fixture.', subcommands: [{
    name: 'read', kind: 'native', summary: 'Read a native file.',
    binding: { package: descriptor.id, operation: 'file.read' },
    input: {
      path: z.string().describe('File path'),
      count: z.number().int().positive().default(2).describe('Count'),
      upper: z.boolean().optional().describe('Uppercase'),
      tag: z.array(z.string()).optional().describe('Tags'),
      body: z.string().describe('Text body'),
      args: z.array(z.string()).optional().describe('Forwarded arguments'),
    },
    positionals: ['path'], stdinField: 'body', restField: 'args',
    output: { json: z.object({ ok: z.boolean() }) },
  }],
}
const inputs = [
  { argv: [], stdin: '' },
  { argv: ['read', '--help'], stdin: '' },
  { argv: ['read', 'some file', '--count', '3', '--tag', 'one', '--tag=two', '--upper', '--json', '--', '--help', '-x'], stdin: 'body\n' },
  { argv: ['read', '--', '--file'], stdin: 'literal' },
  { argv: ['read', 'file', '--count', '2.5'], stdin: '' },
  { argv: ['read', 'file', '--body=bad'], stdin: '' },
  { argv: ['read', 'file', '--count', '1', '--count', '2'], stdin: '' },
]
const cases = inputs.map(input => {
  try {
    return { ...input, parsed: parseCommandInput(root, [root.name, ...input.argv], input.stdin) }
  } catch {
    return { ...input, invalid: true }
  }
})
const directory = new URL('../rust/fixtures/', import.meta.url)
await mkdir(directory, { recursive: true })
await writeFile(new URL('cli.json', directory), `${JSON.stringify({ manifest: await buildManifest([root], { packages: [descriptor] }), help: renderCommandHelp(root), cases }, null, 2)}\n`)
