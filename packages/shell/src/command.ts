import { asError, concatBytes, decodeUtf8, encodeUtf8 } from '@demicodes/utils'
import type { z } from 'zod'
import { RESERVED_COMMAND_NAMES } from './portable-commands'

export type CommandInputSpec = Record<string, z.ZodType>

/**
 * A command is a tree of arbitrary depth, matching standard CLI semantics
 * (cobra/click): group nodes (`CommandSpec`) route and render help, leaf nodes
 * (`CommandSubcommandSpec`) carry the input schema and the executable. The
 * registry registers root groups; `<path...> prompt` renders help at any group.
 */
export interface CommandSpec {
  name: string
  summary: string
  subcommands: CommandNode[]
}

export type CommandNode = CommandSpec | CommandSubcommandSpec

export function isCommandGroup(node: CommandNode): node is CommandSpec {
  return 'subcommands' in node
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
  /** Leaf subcommand name, or 'prompt' for the help pseudo-subcommand. */
  subcommand: string
  /**
   * Segments after the root command name: group names down to the leaf. For
   * 'prompt' it is the path of the group the help applies to (empty = root).
   */
  path: string[]
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

/** A non-text content item a command emits to the model, peer to stdout text. */
export type CommandAsset = { type: 'image'; mediaType: string; data: string }

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
}

export class CommandRegistry {
  private readonly commands = new Map<string, CommandSpec>()

  register(spec: CommandSpec): void {
    if (RESERVED_COMMAND_NAMES.has(spec.name)) {
      throw new Error(`CommandRegistry: command "${spec.name}" is reserved for shell/system commands`)
    }
    if (this.commands.has(spec.name)) {
      throw new Error(`CommandRegistry: command "${spec.name}" is already registered`)
    }
    validateCommandTree(spec, spec.name)
    this.commands.set(spec.name, spec)
  }

  get(name: string): CommandSpec | null {
    return this.commands.get(name) ?? null
  }

  list(): CommandSpec[] {
    return [...this.commands.values()]
  }

  renderPrompt(): string {
    const rendered = this.list()
      .map((spec) => renderCommandPrompt(spec))
      .join('\n\n')
    if (!rendered) return rendered
    return `${COMMAND_PROMPT_DEFAULTS}\n\n${rendered}`
  }
}

// Stated once for the whole registry so per-subcommand renders only carry
// deviations — with dozens of leaves, repeating the defaults costs real prompt budget.
export const COMMAND_PROMPT_DEFAULTS =
  'Unless a subcommand states otherwise: success prints raw text on stdout, failure writes an error message to stderr and exits non-zero.'

export function parseCommandInput(spec: CommandSpec, argv: string[], stdin: CommandStdin = { text: '' }): ParsedCommandInput {
  if (argv[0] !== spec.name) {
    throw new Error(`Expected command "${spec.name}", received "${argv[0] ?? ''}"`)
  }

  let group: CommandSpec = spec
  const path: string[] = []
  let index = 1
  while (true) {
    const segment = argv[index]
    if (!segment) throw new Error(`Command "${[spec.name, ...path].join(' ')}" requires a subcommand`)
    if (segment === 'prompt') {
      return { subcommand: 'prompt', path, values: {}, json: false }
    }
    const child = group.subcommands.find((candidate) => candidate.name === segment)
    if (!child) throw new Error(`Unknown subcommand "${[spec.name, ...path, segment].join(' ')}"`)
    path.push(child.name)
    index += 1
    if (isCommandGroup(child)) {
      group = child
      continue
    }
    return parseLeafInput(child, [spec.name, ...path].join(' '), path, argv, index, stdin)
  }
}

function parseLeafInput(
  subcommand: CommandSubcommandSpec,
  displayPath: string,
  path: string[],
  argv: string[],
  startIndex: number,
  stdin: CommandStdin,
): ParsedCommandInput {
  const input = subcommand.input ?? {}
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
    path,
    values: validateInput(input, values),
    json,
  }
}

function resolveCommandNode(spec: CommandSpec, path: string[]): CommandNode {
  let node: CommandNode = spec
  for (const segment of path) {
    if (!isCommandGroup(node)) throw new Error(`"${node.name}" has no subcommand "${segment}"`)
    const child: CommandNode | undefined = node.subcommands.find((candidate) => candidate.name === segment)
    if (!child) throw new Error(`Unknown subcommand "${segment}" under "${node.name}"`)
    node = child
  }
  return node
}

export async function runRegisteredCommand(spec: CommandSpec, ctx: CommandExecutionContext): Promise<CommandRunResult> {
  const stdin = ctx.stdin ?? { text: '' }
  const parsed = parseCommandInput(spec, ctx.argv, stdin)

  if (parsed.subcommand === 'prompt') {
    // The parser only accepts 'prompt' at group boundaries, so the node is a group.
    const node = resolveCommandNode(spec, parsed.path) as CommandSpec
    const parentPath = [spec.name, ...parsed.path.slice(0, -1)].join(' ')
    await ctx.io.stdout(`${renderCommandPrompt(node, parsed.path.length > 0 ? parentPath : '')}\n`)
    return { exitCode: 0 }
  }

  const subcommand = resolveCommandNode(spec, parsed.path)
  if (isCommandGroup(subcommand)) throw new Error(`"${[spec.name, ...parsed.path].join(' ')}" is not runnable`)
  if (parsed.json && !subcommand.output?.json) {
    throw new Error(`Subcommand "${[spec.name, ...parsed.path].join(' ')}" does not define JSON output`)
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
      throw new Error(`Invalid JSON output for "${[spec.name, ...parsed.path].join(' ')}": ${asError(error).message}`)
    }
    const validation = subcommand.output?.json?.safeParse(json)
    if (!validation?.success) {
      const issue = validation?.error.issues[0]
      throw new Error(`JSON output failed validation for "${[spec.name, ...parsed.path].join(' ')}": ${issue?.message}`)
    }
    await ctx.io.stdout(raw)
  }

  return result
}

export function renderCommandPrompt(spec: CommandSpec, parentPath = ''): string {
  const path = parentPath ? `${parentPath} ${spec.name}` : spec.name
  const leaves = spec.subcommands.filter((node): node is CommandSubcommandSpec => !isCommandGroup(node))
  const groups = spec.subcommands.filter(isCommandGroup)

  const lines = [`${path}: ${spec.summary}`]
  if (leaves.length > 0) {
    lines.push('', 'Subcommands:')
    for (const subcommand of leaves) {
      lines.push('', `  ${path} ${subcommand.name}`)
      lines.push(`    ${subcommand.summary}`)
      if (subcommand.effects) lines.push(`    Effects: ${subcommand.effects}`)
      if (subcommand.successOutput) lines.push(`    Success output: ${subcommand.successOutput}`)
      else if (subcommand.output?.json) lines.push('    Success output: raw text by default; machine-readable JSON when --json is passed')
      if (subcommand.failureOutput) lines.push(`    Failure output: ${subcommand.failureOutput}`)

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
  }

  const blocks = [lines.join('\n')]
  for (const group of groups) blocks.push(renderCommandPrompt(group, path))
  return blocks.join('\n\n')
}

function validateCommandTree(node: CommandSpec, path: string): void {
  const seen = new Set<string>()
  for (const child of node.subcommands) {
    if (child.name === 'prompt') {
      throw new Error(`CommandRegistry: "${path} prompt" is reserved for the help pseudo-subcommand`)
    }
    if (seen.has(child.name)) {
      throw new Error(`CommandRegistry: duplicate subcommand "${path} ${child.name}"`)
    }
    seen.add(child.name)
    if (isCommandGroup(child)) validateCommandTree(child, `${path} ${child.name}`)
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
