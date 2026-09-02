import { asError, collectBytes, concatByteStreams, concatBytes, decodeUtf8, emptyByteStream, encodeUtf8 } from '@demicodes/utils'
import type { z } from 'zod'
import { loadCommandModule, type CommandResult, type CommandWriter, type RuntimeModule } from './command-abi'
import type { Host } from './host'

export type CommandInputSpec = Record<string, z.ZodType>

export interface CommandOutputSpec {
  json?: z.ZodType
}

/**
 * The command tree (`docs/demi-next/commands.md`). Group nodes navigate,
 * leaf nodes execute: a group has subcommands and nothing else; a leaf has
 * a kind, its input and output specs, and either a backend handler
 * (`rpc`) or a module (`runtime`).
 */
export type Command = CommandGroup | CommandLeaf

export interface CommandGroup {
  name: string
  summary: string
  subcommands: Command[]
}

export type CommandKind = 'rpc' | 'runtime'

export type CommandLeaf = RpcCommand | RuntimeCommand

interface CommandLeafBase {
  name: string
  summary: string
  successOutput?: string
  failureOutput?: string
  input?: CommandInputSpec
  positionals?: string[]
  stdinField?: string
  /**
   * Field receiving every token after a literal `--`, unparsed — for
   * commands that forward a raw argv. Declare the field as
   * `z.array(z.string())` (optional when `--` may be omitted).
   */
  restField?: string
  output?: CommandOutputSpec
}

/** A leaf whose implementation runs in the backend, against conversation or platform state. */
export interface RpcCommand extends CommandLeafBase {
  kind: 'rpc'
  run: (ctx: CommandRunContext) => Promise<CommandResult> | CommandResult
}

/** A leaf whose implementation is a module run wherever the command is invoked. */
export interface RuntimeCommand extends CommandLeafBase {
  kind: 'runtime'
  module: RuntimeModule
}

export function isCommandGroup(command: Command): command is CommandGroup {
  return 'subcommands' in command
}

export interface ParsedCommandInput {
  /**
   * Path from root through the selected node, including the root name.
   * For help: path of the node help was requested for.
   */
  path: string[]
  /** True when the invocation requested `--help`, or named a group with nothing after it. */
  help: boolean
  values: Record<string, unknown>
  json: boolean
}

/** What an `rpc` handler receives. */
export interface CommandRunContext {
  argv: string[]
  parsed: ParsedCommandInput
  /** The command's stdin (the pipe), complete. */
  stdin: CommandStdin
  env: Record<string, string>
  cwd: string
  io: CommandIO
  storage: CommandStorage
  /** The Host the invoking shell runs against. */
  host: Host
  /** Aborted when the shell command is aborted (shell_abort, shell teardown). */
  signal: AbortSignal
  /**
   * Stdin written after the command started: each `shell_write` call arrives
   * as one chunk. Ends when the command's shell job is released.
   */
  stdinStream: AsyncIterable<Uint8Array>
}

export interface CommandStdin {
  /** Stdin decoded as UTF-8 text (lossy for non-text input). */
  text: string
  /** Raw stdin bytes, byte-identical to what the pipe delivered. */
  bytes: Uint8Array
}

export function emptyStdin(): CommandStdin {
  return { text: '', bytes: new Uint8Array(0) }
}

export function stdinOf(bytes: Uint8Array): CommandStdin {
  return { text: decodeUtf8(bytes), bytes }
}

export interface CommandIO {
  stdout: CommandWriter
  stderr: CommandWriter
}

export interface CommandStorage {
  readJson<T>(key: string): Promise<T | null>
  writeJson<T>(key: string, value: T): Promise<void>
  delete(key: string): Promise<void>
  list(prefix: string): Promise<string[]>
}

/** One invocation of a root command, as a shell hands it over. */
export interface CommandExecutionContext {
  argv: string[]
  /** The pipe: finite, complete once drained. Absent means empty. */
  stdin?: AsyncIterable<Uint8Array>
  env: Record<string, string>
  cwd: string
  io: CommandIO
  /** Session storage for `rpc` handlers run in this process; absent when every rpc leaf forwards elsewhere. */
  storage?: CommandStorage
  host: Host
  signal?: AbortSignal
  /** Stdin written after the command started, for `rpc` handlers that steer. */
  stdinStream?: AsyncIterable<Uint8Array>
}

const COMMAND_NAME_PATTERN = /^[A-Za-z0-9][A-Za-z0-9_-]*$/

export class CommandRegistry {
  private readonly commands = new Map<string, Command>()

  /** `reserved`: names the executable namespace already owns (a shell's builtins, the system tools); registering one is refused. */
  constructor(private readonly reserved: ReadonlySet<string> = new Set()) {}

  register(command: Command): void {
    if (this.reserved.has(command.name)) {
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

  renderHelp(): string {
    const rendered = this.list()
      .map((command) => renderCommandHelp(command))
      .join('\n\n')
    if (!rendered) return rendered
    return `${COMMAND_HELP_DEFAULTS}\n\n${rendered}`
  }
}

// Stated once for the whole registry so per-command renders only carry deviations.
export const COMMAND_HELP_DEFAULTS =
  'Unless a command states otherwise: success prints raw text on stdout, failure writes an error message to stderr and exits non-zero. Pass --help at any level to print a command\'s documentation.'

/**
 * Resolves argv through the tree and parses the leaf's arguments. A group
 * with nothing after it is a help request for that group.
 */
export function parseCommandInput(root: Command, argv: string[], stdin: CommandStdin = emptyStdin()): ParsedCommandInput {
  const { node, path, index } = resolveArgv(root, argv)
  if (isCommandGroup(node)) return { path, help: true, values: {}, json: false }
  return parseArgs(node, path, argv, index, stdin)
}

/** The node argv names, with the index of the first token that is an argument. */
function resolveArgv(root: Command, argv: string[]): { node: Command; path: string[]; index: number } {
  if (argv[0] !== root.name) {
    throw new Error(`Expected command "${root.name}", received "${argv[0] ?? ''}"`)
  }
  let node: Command = root
  const path: string[] = [root.name]
  let index = 1
  while (isCommandGroup(node)) {
    const token = argv[index]
    // --help renders this node's documentation. A flag can never collide
    // with subcommand names (leading '-' is not a valid name), so no
    // reservation or routing precedence is needed.
    if (token === undefined || token === '--help') return { node, path, index }
    const child = node.subcommands.find((candidate) => candidate.name === token)
    if (!child) throw new Error(`Unknown subcommand "${[...path, token].join(' ')}"`)
    node = child
    path.push(child.name)
    index += 1
  }
  return { node, path, index }
}

function parseArgs(
  command: CommandLeaf,
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
    const token = argv[i]!
    if (command.restField && token === '--') {
      values[command.restField] = argv.slice(i + 1)
      break
    }
    if (token === '--help') {
      return { path: [...path], help: true, values: {}, json: false }
    }
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

  // A field can be both positional and stdin-fed ("positional, or stdin when
  // omitted"); an explicit positional wins over piped stdin.
  if (command.stdinField && values[command.stdinField] === undefined) {
    values[command.stdinField] = stdin.text
  }

  return {
    path: [...path],
    help: false,
    values: validateInput(input, values),
    json,
  }
}

export function resolveCommand(root: Command, path: string[]): Command {
  if (path[0] !== root.name) {
    throw new Error(`Path root "${path[0] ?? ''}" does not match command "${root.name}"`)
  }
  let node: Command = root
  for (let i = 1; i < path.length; i += 1) {
    const segment = path[i]
    const child = isCommandGroup(node) ? node.subcommands.find((candidate) => candidate.name === segment) : undefined
    if (!child) throw new Error(`Unknown subcommand "${path.slice(0, i + 1).join(' ')}"`)
    node = child
  }
  return node
}

/**
 * Runs one invocation of a root command: help for a group or `--help`,
 * otherwise the leaf — an `rpc` handler in this process, or a `runtime`
 * module loaded from its text against the Host's filesystem.
 */
export async function runRegisteredCommand(root: Command, ctx: CommandExecutionContext): Promise<CommandResult> {
  const { node, path, index } = resolveArgv(root, ctx.argv)
  const displayPath = path.join(' ')
  const stdin = ctx.stdin ?? emptyByteStream()
  const stdinStream = ctx.stdinStream ?? emptyByteStream()
  const signal = ctx.signal ?? new AbortController().signal

  const help = async () => {
    const parentPath = path.length > 1 ? path.slice(0, -1).join(' ') : ''
    await ctx.io.stdout(`${renderCommandHelp(node, parentPath)}\n`)
    return { exitCode: 0 }
  }
  if (isCommandGroup(node)) return help()

  // The pipe is drained before parsing when the leaf reads it into a field,
  // or when it is an rpc handler (rpc carries stdin as bytes).
  const needsBytes = node.stdinField !== undefined || node.kind === 'rpc'
  const pipe = needsBytes ? stdinOf(await collectBytes(stdin)) : emptyStdin()
  const parsed = parseArgs(node, path, ctx.argv, index, pipe)
  if (parsed.help) return help()
  if (parsed.json && !node.output?.json) {
    throw new Error(`Command "${displayPath}" does not define JSON output`)
  }

  const capture = new CapturingIO(ctx.io)
  const io = parsed.json ? capture : ctx.io
  let result: CommandResult
  if (node.kind === 'rpc') {
    result = await node.run({
      argv: ctx.argv,
      parsed,
      stdin: pipe,
      env: ctx.env,
      cwd: ctx.cwd,
      io,
      storage: ctx.storage ?? unavailableStorage(displayPath),
      host: ctx.host,
      signal,
      stdinStream,
    })
  } else {
    const run = await loadCommandModule(node.module)
    result = await run({
      args: parsed.values,
      fs: ctx.host.fs,
      cwd: ctx.cwd,
      env: ctx.env,
      stdin: needsBytes ? stdinStream : concatByteStreams(stdin, stdinStream),
      stdout: io.stdout,
      stderr: io.stderr,
      signal,
    })
  }

  if (parsed.json && result.exitCode === 0) {
    const raw = capture.stdoutText()
    let json: unknown
    try {
      json = JSON.parse(raw)
    } catch (error) {
      throw new Error(`Invalid JSON output for "${displayPath}": ${asError(error).message}`)
    }
    const validation = node.output?.json?.safeParse(json)
    if (!validation?.success) {
      const issue = validation?.error.issues[0]
      throw new Error(`JSON output failed validation for "${displayPath}": ${issue?.message}`)
    }
    await ctx.io.stdout(raw)
  }

  return { exitCode: result.exitCode }
}

export function renderCommandHelp(command: Command, parentPath = ''): string {
  const path = parentPath ? `${parentPath} ${command.name}` : command.name
  const blocks: string[] = []

  const lines = [`${path}: ${command.summary}`]

  if (!isCommandGroup(command)) {
    lines.push('', 'Usage:')
    lines.push('', `  ${path}`)
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
  }

  const children = isCommandGroup(command) ? command.subcommands : []
  if (children.length > 0) {
    lines.push('', 'Subcommands:')
    for (const child of children) {
      lines.push(`  ${path} ${child.name} — ${child.summary}`)
    }
  }

  blocks.push(lines.join('\n'))

  for (const child of children) {
    blocks.push(renderCommandHelp(child, path))
  }

  return blocks.join('\n\n')
}

export function validateCommandTree(command: Command, path: string): void {
  if (!COMMAND_NAME_PATTERN.test(command.name)) {
    throw new Error(
      `CommandRegistry: "${path}" has invalid name "${command.name}"; use letters, numbers, underscores, and hyphens`,
    )
  }

  if (isCommandGroup(command)) {
    if (command.subcommands.length === 0) {
      throw new Error(`CommandRegistry: group "${path}" has no subcommands`)
    }
    const seen = new Set<string>()
    for (const child of command.subcommands) {
      if (seen.has(child.name)) {
        throw new Error(`CommandRegistry: duplicate subcommand "${path} ${child.name}"`)
      }
      seen.add(child.name)
      validateCommandTree(child, `${path} ${child.name}`)
    }
    return
  }

  if (command.kind === 'rpc' && typeof command.run !== 'function') {
    throw new Error(`CommandRegistry: rpc leaf "${path}" has no run()`)
  }
  if (command.kind === 'runtime' && typeof command.module !== 'string') {
    throw new Error(`CommandRegistry: runtime leaf "${path}" has no module text`)
  }
  const input = command.input ?? {}
  if (command.stdinField && !(command.stdinField in input)) {
    throw new Error(`CommandRegistry: "${path}" stdinField "${command.stdinField}" is not in input`)
  }
  if (command.restField && !(command.restField in input)) {
    throw new Error(`CommandRegistry: "${path}" restField "${command.restField}" is not in input`)
  }
  for (const positional of command.positionals ?? []) {
    if (!(positional in input)) {
      throw new Error(`CommandRegistry: "${path}" positional "${positional}" is not in input`)
    }
  }
}

function unavailableStorage(displayPath: string): CommandStorage {
  const refuse = () => {
    throw new Error(`"${displayPath}" reads command storage, and this embedder runs rpc commands without one`)
  }
  return { readJson: refuse, writeJson: refuse, delete: refuse, list: refuse }
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
  // A reconstructed optional schema carries its description on the inner type.
  const text = schema.description ?? unwrapSchema(schema).description
  const description = text ? ` - ${text}` : ''
  return `${prefix}${source}${description}`
}

function isArraySchema(schema: z.ZodType): boolean {
  return zodTypeName(unwrapSchema(schema)) === 'array'
}

function isBooleanSchema(schema: z.ZodType): boolean {
  return zodTypeName(unwrapSchema(schema)) === 'boolean'
}

function isNumberSchema(schema: z.ZodType): boolean {
  return zodTypeName(unwrapSchema(schema)) === 'number'
}

function zodTypeName(schema: z.ZodType): string | undefined {
  return (schema as unknown as { def?: { type?: string } }).def?.type
}

function unwrapSchema(schema: z.ZodType): z.ZodType {
  let current = schema
  while (true) {
    const inner = (current as unknown as { def?: { innerType?: z.ZodType } }).def?.innerType
    if (!inner) return current
    current = inner
  }
}

class CapturingIO implements CommandIO {
  private readonly chunks: Uint8Array[] = []

  constructor(private readonly target: CommandIO) {}

  stdout = async (data: string | Uint8Array): Promise<void> => {
    this.chunks.push(typeof data === 'string' ? encodeUtf8(data) : data)
  }

  stderr = async (data: string | Uint8Array): Promise<void> => {
    await this.target.stderr(data)
  }

  stdoutText(): string {
    return decodeUtf8(concatBytes(this.chunks))
  }
}

