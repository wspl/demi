import { asError, concatBytes, decodeUtf8, encodeUtf8 } from '@demicodes/utils'
import type { z } from 'zod'
import { RESERVED_COMMAND_NAMES } from './portable-commands'

export type CommandInputSpec = Record<string, z.ZodType>

export interface CommandOutputSpec {
  json?: z.ZodType
}

/**
 * One CLI tree node. Routing (`subcommands`) and execution (`run`) are
 * independent optional capabilities — registration requires at least one.
 */
export interface Command {
  name: string
  summary: string
  /** Present when this node routes to named children. */
  subcommands?: Command[]
  /**
   * Present when this node is executable with its own args/flags.
   * Execution-only fields below are only meaningful when `run` is set.
   */
  run?: (ctx: CommandRunContext) => Promise<CommandRunResult> | CommandRunResult
  effects?: string
  successOutput?: string
  failureOutput?: string
  input?: CommandInputSpec
  positionals?: string[]
  stdinField?: string
  output?: CommandOutputSpec
  /** Required when `run` is set (may be empty for fixtures). */
  examples?: string[]
}

export interface ParsedCommandInput {
  /**
   * Path from root through the selected node, including the root name.
   * For help: path of the node help was requested for.
   */
  path: string[]
  /** True when the invocation was `<path…> prompt`. */
  help: boolean
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
  supportedAssetTypes: ReadonlySet<CommandAssetType>
}

export interface CommandRunResult {
  exitCode: number
  metadata?: unknown
}

export interface CommandStdin {
  text: string
}

/** A non-text content item a command emits to the model, peer to stdout text.
 *  `data` is base64. Video assets only reach models whose catalog marks video support. */
export type CommandAssetType = 'image' | 'video'
export type CommandAsset = { type: CommandAssetType; mediaType: string; data: string }

export interface CommandIO {
  stdout(data: string | Uint8Array): Promise<void> | void
  stderr(data: string | Uint8Array): Promise<void> | void
  asset(asset: CommandAsset): Promise<void> | void
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
  supportedAssetTypes?: readonly CommandAssetType[]
}

const EXECUTION_ONLY_FIELDS = [
  'effects',
  'successOutput',
  'failureOutput',
  'input',
  'positionals',
  'stdinField',
  'output',
  'examples',
] as const

const COMMAND_NAME_PATTERN = /^[A-Za-z0-9][A-Za-z0-9_-]*$/

export class CommandRegistry {
  private readonly commands = new Map<string, Command>()

  register(command: Command): void {
    if (RESERVED_COMMAND_NAMES.has(command.name)) {
      throw new Error(`CommandRegistry: command "${command.name}" is reserved for shell/system commands`)
    }
    if (this.commands.has(command.name)) {
      throw new Error(`CommandRegistry: command "${command.name}" is already registered`)
    }
    validateCommandTree(command, command.name)
    this.commands.set(command.name, command)
  }

  get(name: string): Command | null {
    return this.commands.get(name) ?? null
  }

  list(): Command[] {
    return [...this.commands.values()]
  }

  renderPrompt(): string {
    const rendered = this.list()
      .map((command) => renderCommandPrompt(command))
      .join('\n\n')
    if (!rendered) return rendered
    return `${COMMAND_PROMPT_DEFAULTS}\n\n${rendered}`
  }
}

// Stated once for the whole registry so per-command renders only carry deviations.
export const COMMAND_PROMPT_DEFAULTS =
  'Unless a command states otherwise: success prints raw text on stdout, failure writes an error message to stderr and exits non-zero.'

export function parseCommandInput(root: Command, argv: string[], stdin: CommandStdin = { text: '' }): ParsedCommandInput {
  if (argv[0] !== root.name) {
    throw new Error(`Expected command "${root.name}", received "${argv[0] ?? ''}"`)
  }

  let node: Command = root
  const path: string[] = [root.name]
  let index = 1

  while (true) {
    if (index >= argv.length) {
      if (node.run) return parseArgs(node, path, argv, index, stdin)
      throw new Error(`Command "${path.join(' ')}" requires a subcommand`)
    }

    const token = argv[index]

    // 'prompt' is the help pseudo-subcommand only where routing happens (like
    // any reserved child name); at a pure run node it stays an ordinary
    // argument, so e.g. a file literally named "prompt" remains addressable.
    if (token === 'prompt' && (node.subcommands?.length ?? 0) > 0) {
      return { path: [...path], help: true, values: {}, json: false }
    }

    const child = node.subcommands?.find((candidate) => candidate.name === token)
    if (child) {
      node = child
      path.push(child.name)
      index += 1
      continue
    }

    if (!node.run) {
      throw new Error(`Unknown subcommand "${[...path, token].join(' ')}"`)
    }
    return parseArgs(node, path, argv, index, stdin)
  }
}

function parseArgs(
  command: Command,
  path: string[],
  argv: string[],
  startIndex: number,
  stdin: CommandStdin,
): ParsedCommandInput {
  const displayPath = path.join(' ')
  const input = command.input ?? {}
  const values: Record<string, unknown> = {}
  let json = false
  let positionalIndex = 0

  for (let i = startIndex; i < argv.length; i += 1) {
    const token = argv[i]
    if (token === '--json') {
      json = true
      continue
    }

    if (token.startsWith('--')) {
      const field = token.slice(2)
      const schema = input[field]
      if (!schema) throw new Error(`Unknown option "--${field}" for "${displayPath}"`)

      const next = argv[i + 1]
      const takesImplicitBoolean = isBooleanSchema(schema) && (next === undefined || next.startsWith('--'))
      const rawValue = takesImplicitBoolean ? true : next
      if (!takesImplicitBoolean) i += 1
      if (rawValue === undefined) throw new Error(`Missing value for "--${field}"`)
      setParsedValue(values, field, rawValue)
      continue
    }

    const field = command.positionals?.[positionalIndex]
    if (!field) throw new Error(`Unexpected positional argument "${token}"`)
    setParsedValue(values, field, token)
    positionalIndex += 1
  }

  if (command.stdinField) {
    values[command.stdinField] = stdin.text
  }

  return {
    path: [...path],
    help: false,
    values: validateInput(input, values),
    json,
  }
}

function resolveCommand(root: Command, path: string[]): Command {
  if (path[0] !== root.name) {
    throw new Error(`Path root "${path[0] ?? ''}" does not match command "${root.name}"`)
  }
  let node: Command = root
  for (let i = 1; i < path.length; i += 1) {
    const segment = path[i]
    const child = node.subcommands?.find((candidate) => candidate.name === segment)
    if (!child) throw new Error(`Unknown subcommand "${path.slice(0, i + 1).join(' ')}"`)
    node = child
  }
  return node
}

export async function runRegisteredCommand(root: Command, ctx: CommandExecutionContext): Promise<CommandRunResult> {
  const stdin = ctx.stdin ?? { text: '' }
  const parsed = parseCommandInput(root, ctx.argv, stdin)
  const displayPath = parsed.path.join(' ')

  if (parsed.help) {
    const node = resolveCommand(root, parsed.path)
    const parentPath = parsed.path.length > 1 ? parsed.path.slice(0, -1).join(' ') : ''
    await ctx.io.stdout(`${renderCommandPrompt(node, parentPath)}\n`)
    return { exitCode: 0 }
  }

  const command = resolveCommand(root, parsed.path)
  if (!command.run) throw new Error(`"${displayPath}" is not runnable`)
  if (parsed.json && !command.output?.json) {
    throw new Error(`Command "${displayPath}" does not define JSON output`)
  }

  const capture = new CapturingIO(ctx.io)
  const result = await command.run({
    argv: ctx.argv,
    parsed,
    stdin,
    env: ctx.env,
    cwd: ctx.cwd,
    io: parsed.json ? capture : ctx.io,
    storage: ctx.storage,
    supportedAssetTypes: new Set(ctx.supportedAssetTypes),
  })

  if (parsed.json && result.exitCode === 0) {
    const raw = capture.stdoutText()
    let json: unknown
    try {
      json = JSON.parse(raw)
    } catch (error) {
      throw new Error(`Invalid JSON output for "${displayPath}": ${asError(error).message}`)
    }
    const validation = command.output?.json?.safeParse(json)
    if (!validation?.success) {
      const issue = validation?.error.issues[0]
      throw new Error(`JSON output failed validation for "${displayPath}": ${issue?.message}`)
    }
    await ctx.io.stdout(raw)
  }

  return result
}

export function renderCommandPrompt(command: Command, parentPath = ''): string {
  const path = parentPath ? `${parentPath} ${command.name}` : command.name
  const blocks: string[] = []

  const lines = [`${path}: ${command.summary}`]

  if (command.run) {
    lines.push('', 'Usage:')
    lines.push('', `  ${path}`)
    if (command.effects) lines.push(`    Effects: ${command.effects}`)
    if (command.successOutput) lines.push(`    Success output: ${command.successOutput}`)
    else if (command.output?.json) {
      lines.push('    Success output: raw text by default; machine-readable JSON when --json is passed')
    }
    if (command.failureOutput) lines.push(`    Failure output: ${command.failureOutput}`)

    const fields = Object.entries(command.input ?? {})
    if (fields.length > 0) {
      lines.push('    Parameters:')
      for (const [field, schema] of fields) {
        const positional = command.positionals?.includes(field) ?? false
        const stdin = command.stdinField === field
        lines.push(`      ${formatField(field, schema, positional, stdin)}`)
      }
    }

    if (command.stdinField) {
      lines.push(`    stdin/heredoc: ${command.stdinField}`)
    }
    if (command.output?.json) {
      lines.push('    --json: emits machine-readable JSON for this command')
    } else {
      lines.push('    --json: accepted only when this command defines JSON output')
    }

    const examples = command.examples ?? []
    if (examples.length > 0) {
      lines.push('    Examples:')
      for (const example of examples) lines.push(indent(example, 6))
    }
  }

  const children = command.subcommands ?? []
  if (children.length > 0) {
    lines.push('', 'Subcommands:')
    for (const child of children) {
      lines.push(`  ${path} ${child.name} — ${child.summary}`)
    }
  }

  blocks.push(lines.join('\n'))

  for (const child of children) {
    blocks.push(renderCommandPrompt(child, path))
  }

  return blocks.join('\n\n')
}

function validateCommandTree(command: Command, path: string): void {
  if (!COMMAND_NAME_PATTERN.test(command.name)) {
    throw new Error(
      `CommandRegistry: "${path}" has invalid name "${command.name}"; use letters, numbers, underscores, and hyphens`,
    )
  }

  const hasRun = typeof command.run === 'function'
  const children = command.subcommands ?? []
  if (!hasRun && children.length === 0) {
    throw new Error(`CommandRegistry: "${path}" must have run() and/or subcommands`)
  }

  if (hasRun) {
    if (!Array.isArray(command.examples)) {
      throw new Error(`CommandRegistry: "${path}" defines run() but is missing examples[]`)
    }
  } else {
    for (const field of EXECUTION_ONLY_FIELDS) {
      if (command[field] !== undefined) {
        throw new Error(`CommandRegistry: "${path}" sets ${field} without run()`)
      }
    }
  }

  if (hasRun) {
    const input = command.input ?? {}
    if (command.stdinField && !(command.stdinField in input)) {
      throw new Error(`CommandRegistry: "${path}" stdinField "${command.stdinField}" is not in input`)
    }
    for (const positional of command.positionals ?? []) {
      if (!(positional in input)) {
        throw new Error(`CommandRegistry: "${path}" positional "${positional}" is not in input`)
      }
    }
  }

  const seen = new Set<string>()
  for (const child of children) {
    if (child.name === 'prompt') {
      throw new Error(`CommandRegistry: "${path} prompt" is reserved for the help pseudo-subcommand`)
    }
    if (seen.has(child.name)) {
      throw new Error(`CommandRegistry: duplicate subcommand "${path} ${child.name}"`)
    }
    seen.add(child.name)
    validateCommandTree(child, `${path} ${child.name}`)
  }
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

  async asset(asset: CommandAsset): Promise<void> {
    await this.target.asset(asset)
  }

  stdoutText(): string {
    return decodeUtf8(concatBytes(this.chunks))
  }
}
