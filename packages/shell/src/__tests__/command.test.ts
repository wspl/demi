import { expect, test } from 'bun:test'
import { z } from 'zod'
import {
  CommandRegistry,
  parseCommandInput,
  renderCommandPrompt,
  runRegisteredCommand,
  type CommandAsset,
  type CommandIO,
  type CommandSpec,
  type CommandStorage,
} from '../index'

const editorSpec: CommandSpec = {
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

test('parseCommandInput maps positionals, flags, and stdin fields', () => {
  const parsed = parseCommandInput(editorSpec, ['editor', 'create', 'src/foo.ts'], {
    text: 'export const foo = 1\n',
  })

  expect(parsed).toEqual({
    subcommand: 'create',
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
    subcommand: 'list',
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

test('renderCommandPrompt uses CommandSpec as the single source of command help', () => {
  const prompt = renderCommandPrompt(editorSpec)

  expect(prompt).toContain('editor: Create, edit, and patch files.')
  expect(prompt).toContain('editor create')
  expect(prompt).toContain('Effects: modifies files')
  expect(prompt).toContain('Success output: writes Created <path> to stdout')
  expect(prompt).toContain('Failure output: writes the error reason to stderr and exits non-zero')
  expect(prompt).toContain('<path> - Target file path')
  expect(prompt).toContain('--old - Exact text to replace')
  expect(prompt).toContain('stdin/heredoc: content')
  expect(prompt).toContain('Effects: not specified')
  expect(prompt).toContain('Success output: raw text by default; machine-readable JSON when --json is passed')
  expect(prompt).toContain('editor list --json --verbose --tag changed --tag staged')
})

test('CommandRegistry registers commands and renders all prompts', () => {
  const registry = new CommandRegistry()
  registry.register(editorSpec)

  expect(registry.get('editor')).toBe(editorSpec)
  expect(registry.list()).toEqual([editorSpec])
  expect(registry.renderPrompt()).toBe(renderCommandPrompt(editorSpec))
  expect(() => registry.register(editorSpec)).toThrow('already registered')
})

test('CommandRegistry rejects names reserved for shell and system commands', () => {
  const registry = new CommandRegistry()
  for (const name of reservedCommandNames) {
    expect(() => registry.register({ ...editorSpec, name })).toThrow('reserved for shell/system commands')
  }
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

test('runRegisteredCommand rejects JSON mode when the subcommand has no JSON output schema', async () => {
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

function editorSpecWithListOutput(output: string): CommandSpec {
  return {
    ...editorSpec,
    subcommands: editorSpec.subcommands.map((subcommand) =>
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
