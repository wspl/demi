import { expect, test } from 'bun:test'
import { z } from 'zod'
import {
  COMMAND_PROMPT_DEFAULTS,
  CommandRegistry,
  parseCommandInput,
  renderCommandPrompt,
  runRegisteredCommand,
  type Command,
  type CommandAsset,
  type CommandIO,
  type CommandStorage,
} from '../index'

const editorSpec: Command = {
  name: 'editor',
  summary: 'Create, edit, and patch files.',
  subcommands: [
    {
      name: 'create',
      summary: 'Create a new file.',
      effects: 'modifies files',
      successOutput: 'writes Created <path> to stdout',
      failureOutput: 'writes the error reason to stderr and exits non-zero',
      input: {
        path: z.string().describe('Target file path'),
        content: z.string().describe('File content'),
      },
      positionals: ['path'],
      stdinField: 'content',
      examples: ["editor create src/foo.ts <<'EOF'\nexport const foo = 1\nEOF"],
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
      examples: ['editor edit src/foo.ts --old foo --new bar'],
      run: () => ({ exitCode: 0 }),
    },
    {
      name: 'list',
      summary: 'List files tracked by editor state.',
      input: {
        verbose: z.boolean().optional().describe('Include details'),
        tag: z.array(z.string()).optional().describe('Filter by repeated tag'),
      },
      output: {
        json: z.object({ files: z.array(z.string()) }),
      },
      examples: ['editor list --json --verbose --tag changed --tag staged'],
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
          examples: ["larkclaw watch create my-id <<'EOF'\n{}\nEOF"],
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
              examples: ['larkclaw watch state get my-id'],
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
      examples: ['larkclaw ping'],
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
  examples: ['kcenv HOME'],
  run: async ({ parsed, io }) => {
    await io.stdout(String(parsed.values.key))
    return { exitCode: 0 }
  },
}

const dualMode: Command = {
  name: 'tool',
  summary: 'Dual-mode parent with a child.',
  input: { x: z.number().optional().describe('Parent flag') },
  examples: ['tool --x 1', 'tool sub --y 2'],
  run: async ({ parsed, io }) => {
    await io.stdout(`parent x=${parsed.values.x ?? 'none'}`)
    return { exitCode: 0 }
  },
  subcommands: [
    {
      name: 'sub',
      summary: 'Child leaf.',
      input: { y: z.number().optional().describe('Child flag') },
      examples: ['tool sub --y 2'],
      run: async ({ parsed, io }) => {
        await io.stdout(`child y=${parsed.values.y ?? 'none'}`)
        return { exitCode: 0 }
      },
    },
  ],
}

test('parseCommandInput maps positionals, flags, and stdin fields', () => {
  const parsed = parseCommandInput(editorSpec, ['editor', 'create', 'src/foo.ts'], {
    text: 'export const foo = 1\n',
  })

  expect(parsed).toEqual({
    path: ['editor', 'create'],
    help: false,
    values: {
      path: 'src/foo.ts',
      content: 'export const foo = 1\n',
    },
    json: false,
  })
})

test('parseCommandInput validates long options and coerces numbers', () => {
  const parsed = parseCommandInput(editorSpec, [
    'editor',
    'edit',
    'src/foo.ts',
    '--old',
    'foo',
    '--new',
    'bar',
    '--occurrence',
    '2',
  ])

  expect(parsed.path).toEqual(['editor', 'edit'])
  expect(parsed.values).toEqual({
    path: 'src/foo.ts',
    old: 'foo',
    new: 'bar',
    occurrence: 2,
  })
})

test('parseCommandInput handles --json, booleans, and repeated array options', () => {
  const parsed = parseCommandInput(editorSpec, [
    'editor',
    'list',
    '--json',
    '--verbose',
    '--tag',
    'changed',
    '--tag',
    'staged',
  ])

  expect(parsed).toEqual({
    path: ['editor', 'list'],
    help: false,
    values: {
      verbose: true,
      tag: ['changed', 'staged'],
    },
    json: true,
  })
})

test('parseCommandInput rejects unknown options and invalid values', () => {
  expect(() => parseCommandInput(editorSpec, ['editor', 'edit', 'src/foo.ts', '--missing', 'x'])).toThrow(
    'Unknown option',
  )
  expect(() =>
    parseCommandInput(editorSpec, ['editor', 'edit', 'src/foo.ts', '--old', 'a', '--new', 'b', '--occurrence', 'NaN']),
  ).toThrow('Invalid value for "occurrence"')
})

test('parseCommandInput walks nested groups down to a leaf', () => {
  const parsed = parseCommandInput(nestedSpec, ['larkclaw', 'watch', 'create', 'my-id'], { text: '{"a":1}' })
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

test('parseCommandInput dual-mode: child name wins over parent args', () => {
  const parent = parseCommandInput(dualMode, ['tool', '--x', '1'])
  expect(parent).toEqual({ path: ['tool'], help: false, values: { x: 1 }, json: false })

  const child = parseCommandInput(dualMode, ['tool', 'sub', '--y', '2'])
  expect(child).toEqual({ path: ['tool', 'sub'], help: false, values: { y: 2 }, json: false })
})

test('parseCommandInput reports full paths for nested errors', () => {
  expect(() => parseCommandInput(nestedSpec, ['larkclaw', 'watch'])).toThrow('Command "larkclaw watch" requires a subcommand')
  expect(() => parseCommandInput(nestedSpec, ['larkclaw', 'watch', 'missing'])).toThrow(
    'Unknown subcommand "larkclaw watch missing"',
  )
  expect(() => parseCommandInput(nestedSpec, ['larkclaw', 'watch', 'create', 'my-id', '--missing', 'x'])).toThrow(
    'Unknown option "--missing" for "larkclaw watch create"',
  )
})

test('renderCommandPrompt documents the tree', () => {
  const prompt = renderCommandPrompt(editorSpec)

  expect(prompt).toContain('editor: Create, edit, and patch files.')
  expect(prompt).toContain('editor create')
  expect(prompt).toContain('Effects: modifies files')
  expect(prompt).toContain('Success output: writes Created <path> to stdout')
  expect(prompt).toContain('Failure output: writes the error reason to stderr and exits non-zero')
  expect(prompt).toContain('<path> - Target file path')
  expect(prompt).toContain('--old - Exact text to replace')
  expect(prompt).toContain('stdin/heredoc: content')
  expect(prompt).toContain('Success output: raw text by default; machine-readable JSON when --json is passed')
  expect(prompt).toContain('editor list --json --verbose --tag changed --tag staged')
})

test('CommandRegistry registers commands and renders all prompts', () => {
  const registry = new CommandRegistry()
  registry.register(editorSpec)

  expect(registry.get('editor')).toBe(editorSpec)
  expect(registry.list()).toEqual([editorSpec])
  expect(registry.renderPrompt()).toBe(`${COMMAND_PROMPT_DEFAULTS}\n\n${renderCommandPrompt(editorSpec)}`)
  expect(() => registry.register(editorSpec)).toThrow('already registered')
})

test('CommandRegistry rejects names reserved for shell and system commands', () => {
  const registry = new CommandRegistry()
  for (const name of reservedCommandNames) {
    expect(() => registry.register({ ...editorSpec, name })).toThrow('reserved for shell/system commands')
  }
})

test('CommandRegistry rejects empty nodes, dead fields, and reserved prompt children', () => {
  const registry = new CommandRegistry()
  expect(() => registry.register({ name: 'empty', summary: 'x' })).toThrow('must have run() and/or subcommands')
  expect(() =>
    registry.register({
      name: 'dead',
      summary: 'x',
      subcommands: [{ name: 'c', summary: 'c', examples: [], run: () => ({ exitCode: 0 }) }],
      effects: 'nope',
    }),
  ).toThrow('sets effects without run()')
  expect(() =>
    registry.register({
      name: 'badprompt',
      summary: 'x',
      subcommands: [{ name: 'prompt', summary: 'no', examples: [], run: () => ({ exitCode: 0 }) }],
    }),
  ).toThrow('reserved for the help pseudo-subcommand')
  expect(() =>
    registry.register({
      name: 'norunexamples',
      summary: 'x',
      run: () => ({ exitCode: 0 }),
    }),
  ).toThrow('missing examples[]')
})

test('runRegisteredCommand implements prompt from the same renderer', async () => {
  const io = new MemoryIO()

  const result = await runRegisteredCommand(editorSpec, {
    argv: ['editor', 'prompt'],
    env: {},
    cwd: '/workspace',
    io,
    storage: memoryStorage(),
  })

  expect(result.exitCode).toBe(0)
  expect(io.stdoutText()).toBe(`${renderCommandPrompt(editorSpec)}\n`)
})

test('runRegisteredCommand executes nested leaves and renders prompt at any group', async () => {
  const run = async (argv: string[], stdin = '') => {
    const io = new MemoryIO()
    const result = await runRegisteredCommand(nestedSpec, {
      argv,
      stdin: { text: stdin },
      env: {},
      cwd: '/workspace',
      io,
      storage: memoryStorage(),
    })
    return { result, io }
  }

  const created = await run(['larkclaw', 'watch', 'create', 'my-id'], '{"a":1}')
  expect(created.result.exitCode).toBe(0)
  expect(created.io.stdoutText()).toBe('created my-id body={"a":1}')

  const help = await run(['larkclaw', 'watch', 'prompt'])
  expect(help.result.exitCode).toBe(0)
  expect(help.io.stdoutText()).toContain('larkclaw watch: Background pollers.')
  expect(help.io.stdoutText()).toContain('larkclaw watch create')
})

test('runRegisteredCommand runs bare roots and dual-mode parents', async () => {
  const bareIO = new MemoryIO()
  const bare = await runRegisteredCommand(bareLeaf, {
    argv: ['kcenv', 'HOME'],
    env: {},
    cwd: '/',
    io: bareIO,
    storage: memoryStorage(),
  })
  expect(bare.exitCode).toBe(0)
  expect(bareIO.stdoutText()).toBe('HOME')

  const parentIO = new MemoryIO()
  await runRegisteredCommand(dualMode, {
    argv: ['tool', '--x', '3'],
    env: {},
    cwd: '/',
    io: parentIO,
    storage: memoryStorage(),
  })
  expect(parentIO.stdoutText()).toBe('parent x=3')

  const childIO = new MemoryIO()
  await runRegisteredCommand(dualMode, {
    argv: ['tool', 'sub', '--y', '4'],
    env: {},
    cwd: '/',
    io: childIO,
    storage: memoryStorage(),
  })
  expect(childIO.stdoutText()).toBe('child y=4')
})

test('runRegisteredCommand validates JSON output when --json is set', async () => {
  const io = new MemoryIO()

  const result = await runRegisteredCommand(editorSpec, {
    argv: ['editor', 'list', '--json'],
    env: {},
    cwd: '/workspace',
    io,
    storage: memoryStorage(),
  })

  expect(result.exitCode).toBe(0)
  expect(JSON.parse(io.stdoutText())).toEqual({ files: ['src/foo.ts'] })
})

test('runRegisteredCommand rejects invalid JSON mode output', async () => {
  const invalidJsonIO = new MemoryIO()
  await expect(
    runRegisteredCommand(editorSpecWithListOutput('not json'), {
      argv: ['editor', 'list', '--json'],
      env: {},
      cwd: '/workspace',
      io: invalidJsonIO,
      storage: memoryStorage(),
    }),
  ).rejects.toThrow('Invalid JSON output for "editor list"')
  expect(invalidJsonIO.stdoutText()).toBe('')

  const schemaMismatchIO = new MemoryIO()
  await expect(
    runRegisteredCommand(editorSpecWithListOutput(JSON.stringify({ files: [1] })), {
      argv: ['editor', 'list', '--json'],
      env: {},
      cwd: '/workspace',
      io: schemaMismatchIO,
      storage: memoryStorage(),
    }),
  ).rejects.toThrow('JSON output failed validation for "editor list"')
  expect(schemaMismatchIO.stdoutText()).toBe('')
})

test('runRegisteredCommand rejects JSON mode when the command has no JSON output schema', async () => {
  const io = new MemoryIO()

  await expect(
    runRegisteredCommand(editorSpec, {
      argv: ['editor', 'create', 'src/foo.ts', '--json'],
      stdin: { text: '' },
      env: {},
      cwd: '/workspace',
      io,
      storage: memoryStorage(),
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
  private readonly assetItems: CommandAsset[] = []

  stdout(data: string | Uint8Array): void {
    this.stdoutChunks.push(typeof data === 'string' ? Buffer.from(data) : data)
  }

  stderr(data: string | Uint8Array): void {
    this.stderrChunks.push(typeof data === 'string' ? Buffer.from(data) : data)
  }

  asset(asset: CommandAsset): void {
    this.assetItems.push(asset)
  }

  stdoutText(): string {
    return Buffer.concat(this.stdoutChunks).toString('utf8')
  }

  stderrText(): string {
    return Buffer.concat(this.stderrChunks).toString('utf8')
  }

  assets(): CommandAsset[] {
    return this.assetItems
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

function editorSpecWithListOutput(output: string): Command {
  return {
    ...editorSpec,
    subcommands: editorSpec.subcommands!.map((subcommand) =>
      subcommand.name === 'list'
        ? {
            ...subcommand,
            run: async ({ io }) => {
              await io.stdout(output)
              return { exitCode: 0 }
            },
          }
        : subcommand,
    ),
  }
}
