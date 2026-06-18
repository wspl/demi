import type { z } from 'zod'
import { concatBytes, decodeUtf8, encodeUtf8 } from './bytes'

export type CommandInputSpec = Record<string, z.ZodType>

export interface CommandSpec {
  name: string
  summary: string
  subcommands: CommandSubcommandSpec[]
}

export interface CommandSubcommandSpec {
  name: string
  summary: string
  effects?: string
  successOutput?: string
  failureOutput?: string
  input?: CommandInputSpec
  positionals?: string[]
  stdinField?: string
  output?: CommandOutputSpec
  examples: string[]
  run(ctx: CommandRunContext): Promise<CommandRunResult> | CommandRunResult
}

export interface CommandOutputSpec {
  json?: z.ZodType
}

export interface ParsedCommandInput {
  subcommand: string
  values: Record<string, unknown>
  json: boolean
}

export interface CommandRunContext {
  argv: string[]
  parsed: ParsedCommandInput
  stdin: CommandStdin
  env: Record<string, string>
  cwd: string
  io: CommandIO
  storage: CommandStorage
}

export interface CommandRunResult {
  exitCode: number
  metadata?: unknown
}

export interface CommandStdin {
  text: string
}

export interface CommandIO {
  stdout(data: string | Uint8Array): Promise<void> | void
  stderr(data: string | Uint8Array): Promise<void> | void
}

export interface CommandStorage {
  readJson<T>(key: string): Promise<T | null>
  writeJson<T>(key: string, value: T): Promise<void>
  delete(key: string): Promise<void>
  list(prefix: string): Promise<string[]>
}

export interface CommandExecutionContext {
  argv: string[]
  stdin?: CommandStdin
  env: Record<string, string>
  cwd: string
  io: CommandIO
  storage: CommandStorage
}

const RESERVED_COMMAND_NAMES = new Set([
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
])

export class CommandRegistry {
  private readonly commands = new Map<string, CommandSpec>()

  register(spec: CommandSpec): void {
    if (RESERVED_COMMAND_NAMES.has(spec.name)) {
      throw new Error(`CommandRegistry: command "${spec.name}" is reserved for shell/system commands`)
    }
    if (this.commands.has(spec.name)) {
      throw new Error(`CommandRegistry: command "${spec.name}" is already registered`)
    }
    this.commands.set(spec.name, spec)
  }

  get(name: string): CommandSpec | null {
    return this.commands.get(name) ?? null
  }

  list(): CommandSpec[] {
    return [...this.commands.values()]
  }

  renderPrompt(): string {
    return this.list().map(renderCommandPrompt).join('\n\n')
  }
}

export function parseCommandInput(spec: CommandSpec, argv: string[], stdin: CommandStdin = { text: '' }): ParsedCommandInput {
  if (argv[0] !== spec.name) {
    throw new Error(`Expected command "${spec.name}", received "${argv[0] ?? ''}"`)
  }
  const subcommandName = argv[1]
  if (!subcommandName) throw new Error(`Command "${spec.name}" requires a subcommand`)

  if (subcommandName === 'prompt') {
    return { subcommand: 'prompt', values: {}, json: false }
  }

  const subcommand = spec.subcommands.find((candidate) => candidate.name === subcommandName)
  if (!subcommand) throw new Error(`Unknown subcommand "${spec.name} ${subcommandName}"`)

  const input = subcommand.input ?? {}
  const values: Record<string, unknown> = {}
  let json = false
  let positionalIndex = 0

  for (let i = 2; i < argv.length; i += 1) {
    const token = argv[i]
    if (token === '--json') {
      json = true
      continue
    }

    if (token.startsWith('--')) {
      const field = token.slice(2)
      const schema = input[field]
      if (!schema) throw new Error(`Unknown option "--${field}" for "${spec.name} ${subcommand.name}"`)

      const next = argv[i + 1]
      const takesImplicitBoolean = isBooleanSchema(schema) && (next === undefined || next.startsWith('--'))
      const rawValue = takesImplicitBoolean ? true : next
      if (!takesImplicitBoolean) i += 1
      if (rawValue === undefined) throw new Error(`Missing value for "--${field}"`)
      setParsedValue(values, field, rawValue)
      continue
    }

    const field = subcommand.positionals?.[positionalIndex]
    if (!field) throw new Error(`Unexpected positional argument "${token}"`)
    setParsedValue(values, field, token)
    positionalIndex += 1
  }

  if (subcommand.stdinField) {
    values[subcommand.stdinField] = stdin.text
  }

  return {
    subcommand: subcommand.name,
    values: validateInput(input, values),
    json,
  }
}

export async function runRegisteredCommand(spec: CommandSpec, ctx: CommandExecutionContext): Promise<CommandRunResult> {
  const stdin = ctx.stdin ?? { text: '' }
  const parsed = parseCommandInput(spec, ctx.argv, stdin)

  if (parsed.subcommand === 'prompt') {
    await ctx.io.stdout(`${renderCommandPrompt(spec)}\n`)
    return { exitCode: 0 }
  }

  const subcommand = spec.subcommands.find((candidate) => candidate.name === parsed.subcommand)
  if (!subcommand) throw new Error(`Unknown subcommand "${spec.name} ${parsed.subcommand}"`)
  if (parsed.json && !subcommand.output?.json) {
    throw new Error(`Subcommand "${spec.name} ${subcommand.name}" does not define JSON output`)
  }

  const capture = new CapturingIO(ctx.io)
  const result = await subcommand.run({
    argv: ctx.argv,
    parsed,
    stdin,
    env: ctx.env,
    cwd: ctx.cwd,
    io: parsed.json ? capture : ctx.io,
    storage: ctx.storage,
  })

  if (parsed.json && result.exitCode === 0) {
    const raw = capture.stdoutText()
    let json: unknown
    try {
      json = JSON.parse(raw)
    } catch (error) {
      throw new Error(`Invalid JSON output for "${spec.name} ${subcommand.name}": ${asError(error).message}`)
    }
    const validation = subcommand.output?.json?.safeParse(json)
    if (!validation?.success) {
      const issue = validation?.error.issues[0]
      throw new Error(`JSON output failed validation for "${spec.name} ${subcommand.name}": ${issue?.message}`)
    }
    await ctx.io.stdout(raw)
  }

  return result
}

export function renderCommandPrompt(spec: CommandSpec): string {
  const lines = [`${spec.name}: ${spec.summary}`, '', 'Subcommands:']

  for (const subcommand of spec.subcommands) {
    lines.push('', `  ${spec.name} ${subcommand.name}`)
    lines.push(`    ${subcommand.summary}`)
    lines.push(`    Effects: ${subcommand.effects ?? 'not specified'}`)
    lines.push(`    Success output: ${subcommand.successOutput ?? defaultSuccessOutput(subcommand)}`)
    lines.push(`    Failure output: ${subcommand.failureOutput ?? 'writes an error message to stderr and exits non-zero'}`)

    const fields = Object.entries(subcommand.input ?? {})
    if (fields.length > 0) {
      lines.push('    Parameters:')
      for (const [field, schema] of fields) {
        const positional = subcommand.positionals?.includes(field) ?? false
        const stdin = subcommand.stdinField === field
        lines.push(`      ${formatField(field, schema, positional, stdin)}`)
      }
    }

    if (subcommand.stdinField) {
      lines.push(`    stdin/heredoc: ${subcommand.stdinField}`)
    }
    if (subcommand.output?.json) {
      lines.push('    --json: emits machine-readable JSON for this subcommand')
    } else {
      lines.push('    --json: accepted only when this subcommand defines JSON output')
    }

    if (subcommand.examples.length > 0) {
      lines.push('    Examples:')
      for (const example of subcommand.examples) lines.push(indent(example, 6))
    }
  }

  return lines.join('\n')
}

function defaultSuccessOutput(subcommand: CommandSubcommandSpec): string {
  if (subcommand.output?.json) return 'raw text by default; machine-readable JSON when --json is passed'
  return 'raw text on stdout'
}

function setParsedValue(values: Record<string, unknown>, field: string, value: unknown): void {
  if (values[field] === undefined) {
    values[field] = value
    return
  }
  if (Array.isArray(values[field])) {
    values[field].push(value)
    return
  }
  values[field] = [values[field], value]
}

function validateInput(input: CommandInputSpec, values: Record<string, unknown>): Record<string, unknown> {
  const parsed: Record<string, unknown> = {}
  for (const [field, schema] of Object.entries(input)) {
    const candidate = coerceValue(schema, values[field])
    const result = schema.safeParse(candidate)
    if (!result.success) {
      const issue = result.error.issues[0]
      throw new Error(`Invalid value for "${field}": ${issue?.message ?? 'validation failed'}`)
    }
    parsed[field] = result.data
  }
  return parsed
}

function coerceValue(schema: z.ZodType, value: unknown): unknown {
  if (value === undefined) return value
  if (isArraySchema(schema)) return Array.isArray(value) ? value : [value]
  if (isNumberSchema(schema) && typeof value === 'string' && value.trim() !== '') return Number(value)
  if (isBooleanSchema(schema) && typeof value === 'string') {
    if (value === 'true') return true
    if (value === 'false') return false
  }
  return value
}

function formatField(field: string, schema: z.ZodType, positional: boolean, stdin: boolean): string {
  const prefix = positional ? `<${field}>` : `--${field}`
  const source = stdin ? ' (from stdin/heredoc)' : ''
  const description = schema.description ? ` - ${schema.description}` : ''
  return `${prefix}${source}${description}`
}

function indent(text: string, spaces: number): string {
  const prefix = ' '.repeat(spaces)
  return text
    .split('\n')
    .map((line) => `${prefix}${line}`)
    .join('\n')
}

function isArraySchema(schema: z.ZodType): boolean {
  const unwrapped = unwrapSchema(schema)
  return zodTypeName(unwrapped) === 'array' || zodTypeName(unwrapped) === 'ZodArray'
}

function isBooleanSchema(schema: z.ZodType): boolean {
  const unwrapped = unwrapSchema(schema)
  return zodTypeName(unwrapped) === 'boolean' || zodTypeName(unwrapped) === 'ZodBoolean'
}

function isNumberSchema(schema: z.ZodType): boolean {
  const unwrapped = unwrapSchema(schema)
  return zodTypeName(unwrapped) === 'number' || zodTypeName(unwrapped) === 'ZodNumber'
}

function zodTypeName(schema: z.ZodType): string | undefined {
  const internals = schema as unknown as {
    def?: { type?: string; typeName?: string }
    _def?: { type?: string; typeName?: string }
  }
  return internals.def?.type ?? internals.def?.typeName ?? internals._def?.type ?? internals._def?.typeName
}

function unwrapSchema(schema: z.ZodType): z.ZodType {
  let current = schema
  while (true) {
    const internals = current as unknown as {
      def?: { innerType?: z.ZodType }
      _def?: { innerType?: z.ZodType }
    }
    const inner = internals.def?.innerType ?? internals._def?.innerType
    if (!inner) return current
    current = inner
  }
}

class CapturingIO implements CommandIO {
  private readonly chunks: Uint8Array[] = []

  constructor(private readonly target: CommandIO) {}

  async stdout(data: string | Uint8Array): Promise<void> {
    this.chunks.push(typeof data === 'string' ? encodeUtf8(data) : data)
  }

  async stderr(data: string | Uint8Array): Promise<void> {
    await this.target.stderr(data)
  }

  stdoutText(): string {
    return decodeUtf8(concatBytes(this.chunks))
  }
}

function asError(error: unknown): Error {
  return error instanceof Error ? error : new Error(String(error))
}
