import { expect, test } from 'bun:test'
import { z } from 'zod'
import { bytesStream, encodeUtf8 } from '@demicodes/utils'
import {
  COMMAND_HELP_DEFAULTS,
  CommandRegistry,
  parseCommandInput,
  renderCommandHelp,
  runRegisteredCommand,
  type Command,
  type CommandGroup,
  type CommandIO,
  type CommandStdin,
  type CommandStorage,
  type Host,
  type RuntimeModule,
} from '../index'
import { RESERVED_COMMAND_NAMES } from '../portable-commands'

const testHost = {} as Host

function stdinOf(text: string): CommandStdin {
  return { text, bytes: encodeUtf8(text) }
}

const filerSpec: Command = {
  name: 'filer',
  summary: 'Create, edit, and patch files.',
  subcommands: [
    {
      name: 'create',
      summary: 'Create a new file.',
      successOutput: 'writes Created <path> to stdout',
      failureOutput: 'writes the error reason to stderr and exits non-zero',
      input: {
        path: z.string().describe('Target file path'),
        content: z.string().describe('File content'),
      },
      positionals: ['path'],
      stdinField: 'content',
      kind: 'rpc',
      run: () => ({ exitCode: 0 }),
    },
    {
      name: 'edit',
      summary: 'Replace exact text in a file.',
      input: {
        path: z.string().describe('Target file path'),
        old: z.string().describe('Exact text to replace'),
        new: z.string().describe('Replacement text'),
        occurrence: z.number().optional().describe('1-based occurrence to replace'),
      },
      positionals: ['path'],
      kind: 'rpc',
      run: () => ({ exitCode: 0 }),
    },
    {
      name: 'list',
      summary: 'List files tracked by filer state.',
      input: {
        verbose: z.boolean().optional().describe('Include details'),
        tag: z.array(z.string()).optional().describe('Filter by repeated tag'),
      },
      output: {
        json: z.object({ files: z.array(z.string()) }),
      },
      kind: 'rpc',
      run: async ({ io }) => {
        await io.stdout(JSON.stringify({ files: ['src/foo.ts'] }))
        return { exitCode: 0 }
      },
    },
  ],
}

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
          kind: 'rpc',
          run: async ({ parsed, io }) => {
            await io.stdout(`created ${parsed.values.id} body=${parsed.values.body}`)
            return { exitCode: 0 }
          },
        },
        {
          name: 'state',
          summary: 'Poller state.',
          subcommands: [
            {
              name: 'get',
              summary: 'Read poller state.',
              input: { id: z.string().describe('Poller id') },
              positionals: ['id'],
              kind: 'rpc',
              run: async ({ parsed, io }) => {
                await io.stdout(`state of ${parsed.values.id}`)
                return { exitCode: 0 }
              },
            },
          ],
        },
      ],
    },
    {
      name: 'ping',
      summary: 'Liveness check.',
      kind: 'rpc',
      run: async ({ io }) => {
        await io.stdout('pong')
        return { exitCode: 0 }
      },
    },
  ],
}

const bareLeaf: Command = {
  name: 'kcenv',
  summary: 'Read a key from the environment map.',
  input: { key: z.string().describe('Env key') },
  positionals: ['key'],
  kind: 'rpc',
  run: async ({ parsed, io }) => {
    await io.stdout(String(parsed.values.key))
    return { exitCode: 0 }
  },
}

test('parseCommandInput maps positionals, flags, and stdin fields', () => {
  const parsed = parseCommandInput(filerSpec, ['filer', 'create', 'src/foo.ts'], stdinOf('export const foo = 1\n'))

  expect(parsed).toEqual({
    path: ['filer', 'create'],
    help: false,
    values: {
      path: 'src/foo.ts',
      content: 'export const foo = 1\n',
    },
    json: false,
  })
})

test('parseCommandInput validates long options and coerces numbers', () => {
  const parsed = parseCommandInput(filerSpec, [
    'filer',
    'edit',
    'src/foo.ts',
    '--old',
    'foo',
    '--new',
    'bar',
    '--occurrence',
    '2',
  ])

  expect(parsed.path).toEqual(['filer', 'edit'])
  expect(parsed.values).toEqual({
    path: 'src/foo.ts',
    old: 'foo',
    new: 'bar',
    occurrence: 2,
  })
})

test('parseCommandInput handles --json, booleans, and repeated array options', () => {
  const parsed = parseCommandInput(filerSpec, [
    'filer',
    'list',
    '--json',
    '--verbose',
    '--tag',
    'changed',
    '--tag',
    'staged',
  ])

  expect(parsed).toEqual({
    path: ['filer', 'list'],
    help: false,
    values: {
      verbose: true,
      tag: ['changed', 'staged'],
    },
    json: true,
  })
})

test('parseCommandInput rejects unknown options and invalid values', () => {
  expect(() => parseCommandInput(filerSpec, ['filer', 'edit', 'src/foo.ts', '--missing', 'x'])).toThrow(
    'Unknown option',
  )
  expect(() =>
    parseCommandInput(filerSpec, ['filer', 'edit', 'src/foo.ts', '--old', 'a', '--new', 'b', '--occurrence', 'NaN']),
  ).toThrow('Invalid value for "occurrence"')
})

test('parseCommandInput walks nested groups down to a leaf', () => {
  const parsed = parseCommandInput(nestedSpec, ['larkclaw', 'watch', 'create', 'my-id'], stdinOf('{"a":1}'))
  expect(parsed).toEqual({
    path: ['larkclaw', 'watch', 'create'],
    help: false,
    values: { id: 'my-id', body: '{"a":1}' },
    json: false,
  })

  const deep = parseCommandInput(nestedSpec, ['larkclaw', 'watch', 'state', 'get', 'my-id'])
  expect(deep.path).toEqual(['larkclaw', 'watch', 'state', 'get'])
  expect(deep.values).toEqual({ id: 'my-id' })

  const flat = parseCommandInput(nestedSpec, ['larkclaw', 'ping'])
  expect(flat).toEqual({ path: ['larkclaw', 'ping'], help: false, values: {}, json: false })
})

test('parseCommandInput supports bare root leaves', () => {
  const parsed = parseCommandInput(bareLeaf, ['kcenv', 'HOME'])
  expect(parsed).toEqual({
    path: ['kcenv'],
    help: false,
    values: { key: 'HOME' },
    json: false,
  })
})

test('parseCommandInput treats --help as help at every node', () => {
  // Groups, leaves, and bare leaf roots all render help.
  expect(parseCommandInput(filerSpec, ['filer', '--help']).help).toBe(true)
  expect(parseCommandInput(bareLeaf, ['kcenv', '--help'])).toEqual({
    path: ['kcenv'],
    help: true,
    values: {},
    json: false,
  })

  // --help wins wherever it appears among a run node's arguments.
  const leaf = parseCommandInput(filerSpec, ['filer', 'edit', 'src/foo.ts', '--old', 'a', '--help'])
  expect(leaf).toEqual({ path: ['filer', 'edit'], help: true, values: {}, json: false })

  // A positional named like the old pseudo-subcommand is just a value.
  const bare = parseCommandInput(bareLeaf, ['kcenv', 'prompt'])
  expect(bare).toEqual({ path: ['kcenv'], help: false, values: { key: 'prompt' }, json: false })
})

test('parseCommandInput reports full paths for nested errors', () => {
  // A group with nothing after it is a help request for that group.
  expect(parseCommandInput(nestedSpec, ['larkclaw', 'watch'])).toEqual({ path: ['larkclaw', 'watch'], help: true, values: {}, json: false })
  expect(() => parseCommandInput(nestedSpec, ['larkclaw', 'watch', 'missing'])).toThrow(
    'Unknown subcommand "larkclaw watch missing"',
  )
  expect(() => parseCommandInput(nestedSpec, ['larkclaw', 'watch', 'create', 'my-id', '--missing', 'x'])).toThrow(
    'Unknown option "--missing" for "larkclaw watch create"',
  )
})

test('renderCommandHelp documents the tree', () => {
  const prompt = renderCommandHelp(filerSpec)

  expect(prompt).toContain('filer: Create, edit, and patch files.')
  expect(prompt).toContain('filer create')
  expect(prompt).toContain('Success output: writes Created <path> to stdout')
  expect(prompt).toContain('Failure output: writes the error reason to stderr and exits non-zero')
  expect(prompt).toContain('<path> - Target file path')
  expect(prompt).toContain('--old - Exact text to replace')
  expect(prompt).toContain('stdin/heredoc: content')
  expect(prompt).toContain('Success output: raw text by default; machine-readable JSON when --json is passed')
  expect(prompt).toContain('--verbose - Include details')
  expect(prompt).toContain('--tag - Filter by repeated tag')
})

test('CommandRegistry registers commands and renders all prompts', () => {
  const registry = new CommandRegistry()
  registry.register(filerSpec)

  expect(registry.get('filer')).toBe(filerSpec)
  expect(registry.list()).toEqual([filerSpec])
  expect(registry.renderHelp()).toBe(`${COMMAND_HELP_DEFAULTS}\n\n${renderCommandHelp(filerSpec)}`)
  expect(() => registry.register(filerSpec)).toThrow('already registered')
})

test('CommandRegistry rejects names reserved for shell and system commands', () => {
  const registry = new CommandRegistry(RESERVED_COMMAND_NAMES)
  for (const name of reservedCommandNames) {
    expect(() => registry.register({ ...filerSpec, name })).toThrow('reserved for shell/system commands')
  }
})

test('CommandRegistry rejects command names that are unsafe as CLI path segments', () => {
  const registry = new CommandRegistry()
  for (const name of ['../escape', '/absolute', '..', 'package.json', 'has space']) {
    expect(() => registry.register({ ...filerSpec, name })).toThrow('has invalid name')
  }
  expect(() =>
    registry.register({
      name: 'safe-root',
      summary: 'x',
      subcommands: [{ name: '../escape', summary: 'x', kind: 'rpc', run: () => ({ exitCode: 0 }) }],
    }),
  ).toThrow('has invalid name')
})

test('CommandRegistry rejects empty groups, handlerless leaves, and dangling field references', () => {
  const registry = new CommandRegistry()
  expect(() => registry.register({ name: 'empty', summary: 'x', subcommands: [] })).toThrow('has no subcommands')
  expect(() =>
    registry.register({ name: 'noRun', summary: 'x', kind: 'rpc', run: undefined as unknown as () => { exitCode: number } }),
  ).toThrow('has no run()')
  expect(() =>
    registry.register({ name: 'noModule', summary: 'x', kind: 'runtime', module: undefined as unknown as RuntimeModule }),
  ).toThrow('has no module text')
  expect(() =>
    registry.register({ name: 'dangling', summary: 'x', kind: 'rpc', positionals: ['nope'], run: () => ({ exitCode: 0 }) }),
  ).toThrow('positional "nope" is not in input')
  // With help moved to --help, 'prompt' is an ordinary (legal) child name.
  registry.register({
    name: 'okprompt',
    summary: 'x',
    subcommands: [{ name: 'prompt', summary: 'fine', kind: 'rpc', run: () => ({ exitCode: 0 }) }],
  })
  expect(registry.get('okprompt')).not.toBeNull()
})

test('runRegisteredCommand implements --help from the same renderer', async () => {
  const io = new MemoryIO()

  const result = await runRegisteredCommand(filerSpec, {
    argv: ['filer', '--help'],
    env: {},
    cwd: '/workspace',
    io,
    storage: memoryStorage(),
    host: testHost,
  })

  expect(result.exitCode).toBe(0)
  expect(io.stdoutText()).toBe(`${renderCommandHelp(filerSpec)}\n`)
})

test('runRegisteredCommand executes nested leaves and renders help at any group', async () => {
  const run = async (argv: string[], stdin = '') => {
    const io = new MemoryIO()
    const result = await runRegisteredCommand(nestedSpec, {
      argv,
      stdin: bytesStream(encodeUtf8(stdin)),
      env: {},
      cwd: '/workspace',
      io,
      storage: memoryStorage(),
      host: testHost,
    })
    return { result, io }
  }

  const created = await run(['larkclaw', 'watch', 'create', 'my-id'], '{"a":1}')
  expect(created.result.exitCode).toBe(0)
  expect(created.io.stdoutText()).toBe('created my-id body={"a":1}')

  const help = await run(['larkclaw', 'watch', '--help'])
  expect(help.result.exitCode).toBe(0)
  expect(help.io.stdoutText()).toContain('larkclaw watch: Background pollers.')
  expect(help.io.stdoutText()).toContain('larkclaw watch create')
})

test('runRegisteredCommand runs bare leaf roots', async () => {
  const bareIO = new MemoryIO()
  const bare = await runRegisteredCommand(bareLeaf, {
    argv: ['kcenv', 'HOME'],
    env: {},
    cwd: '/',
    io: bareIO,
    storage: memoryStorage(),
    host: testHost,
  })
  expect(bare.exitCode).toBe(0)
  expect(bareIO.stdoutText()).toBe('HOME')

})

test('runRegisteredCommand validates JSON output when --json is set', async () => {
  const io = new MemoryIO()

  const result = await runRegisteredCommand(filerSpec, {
    argv: ['filer', 'list', '--json'],
    env: {},
    cwd: '/workspace',
    io,
    storage: memoryStorage(),
    host: testHost,
  })

  expect(result.exitCode).toBe(0)
  expect(JSON.parse(io.stdoutText())).toEqual({ files: ['src/foo.ts'] })
})

test('runRegisteredCommand rejects invalid JSON mode output', async () => {
  const invalidJsonIO = new MemoryIO()
  await expect(
    runRegisteredCommand(filerSpecWithListOutput('not json'), {
      argv: ['filer', 'list', '--json'],
      env: {},
      cwd: '/workspace',
      io: invalidJsonIO,
      storage: memoryStorage(),
      host: testHost,
    }),
  ).rejects.toThrow('Invalid JSON output for "filer list"')
  expect(invalidJsonIO.stdoutText()).toBe('')

  const schemaMismatchIO = new MemoryIO()
  await expect(
    runRegisteredCommand(filerSpecWithListOutput(JSON.stringify({ files: [1] })), {
      argv: ['filer', 'list', '--json'],
      env: {},
      cwd: '/workspace',
      io: schemaMismatchIO,
      storage: memoryStorage(),
      host: testHost,
    }),
  ).rejects.toThrow('JSON output failed validation for "filer list"')
  expect(schemaMismatchIO.stdoutText()).toBe('')
})

test('runRegisteredCommand rejects JSON mode when the command has no JSON output schema', async () => {
  const io = new MemoryIO()

  await expect(
    runRegisteredCommand(filerSpec, {
      argv: ['filer', 'create', 'src/foo.ts', '--json'],
      env: {},
      cwd: '/workspace',
      io,
      storage: memoryStorage(),
      host: testHost,
    }),
  ).rejects.toThrow('does not define JSON output')
})

const reservedCommandNames = [
  '.',
  'awk',
  'bash',
  'break',
  'bun',
  'cargo',
  'cat',
  'cd',
  'chmod',
  'command',
  'continue',
  'cp',
  'cut',
  'docker',
  'du',
  'echo',
  'exit',
  'export',
  'file',
  'find',
  'git',
  'grep',
  'head',
  'jobs',
  'jq',
  'local',
  'ls',
  'mkdir',
  'mv',
  'nl',
  'node',
  'npm',
  'pnpm',
  'popd',
  'printf',
  'pushd',
  'python',
  'read',
  'return',
  'rg',
  'rm',
  'sed',
  'set',
  'sh',
  'shift',
  'sort',
  'source',
  'stat',
  'tail',
  'tee',
  'test',
  'touch',
  'tree',
  'tr',
  'uniq',
  'unset',
  'wait',
  'wc',
  'xargs',
  'yarn',
  'yq',
]

class MemoryIO implements CommandIO {
  private readonly stdoutChunks: Uint8Array[] = []
  private readonly stderrChunks: Uint8Array[] = []

  stdout(data: string | Uint8Array): void {
    this.stdoutChunks.push(typeof data === 'string' ? Buffer.from(data) : data)
  }

  stderr(data: string | Uint8Array): void {
    this.stderrChunks.push(typeof data === 'string' ? Buffer.from(data) : data)
  }

  stdoutText(): string {
    return Buffer.concat(this.stdoutChunks).toString('utf8')
  }

  stderrText(): string {
    return Buffer.concat(this.stderrChunks).toString('utf8')
  }
}

function memoryStorage(): CommandStorage {
  const values = new Map<string, unknown>()
  return {
    readJson: async (key) => (values.has(key) ? (values.get(key) as never) : null),
    writeJson: async (key, value) => {
      values.set(key, value)
    },
    delete: async (key) => {
      values.delete(key)
    },
    list: async (prefix) => [...values.keys()].filter((key) => key.startsWith(prefix)),
  }
}

function filerSpecWithListOutput(output: string): Command {
  return {
    ...filerSpec,
    subcommands: (filerSpec as CommandGroup).subcommands.map((subcommand): Command =>
      subcommand.name === 'list'
        ? {
            ...subcommand,
            kind: 'rpc',
            run: async ({ io }) => {
              await io.stdout(output)
              return { exitCode: 0 }
            },
          }
        : subcommand,
    ),
  }
}
