import { nativeBindingSchema, type NativeBinding } from '@demicodes/command-protocol'
import {
  asError,
  collectBytes,
  concatByteStreams,
  concatBytes,
  decodeUtf8,
  emptyByteStream,
  encodeUtf8,
  throwIfAborted
} from '@demicodes/utils'
import { z } from 'zod'
import {
  type CommandResult,
  type CommandWriter,
  type DispatchIO,
  type NativeExecutor
} from './command-abi'
import type { Host } from './host'

/**
 * A leaf's argument declarations. Only the zod subset that survives the
 * manifest's JSON Schema round trip is allowed; `unsupportedInputSchema`
 * states it and registration enforces it.
 */
export type CommandInputSpec = Record<string, z.ZodType>

export interface CommandOutputSpec {
  json?: z.ZodType
}

/**
 * The command tree (`docs/demi-next/commands.md`). Group nodes navigate,
 * leaf nodes execute: a group has subcommands and nothing else; a leaf has
 * a kind, its input and output specs, and either a backend handler
 * (`rpc`) or a native package binding (`native`).
 */
export type Command = CommandGroup | CommandLeaf

export interface CommandGroup {
  name: string
  summary: string
  subcommands: Command[]
}

export type CommandKind = 'rpc' | 'native'

export type CommandLeaf<I extends CommandInputSpec = CommandInputSpec> =
  | RpcCommand<I>
  | NativeCommand<I>

interface CommandLeafBase<I extends CommandInputSpec> {
  name: string
  summary: string
  successOutput?: string
  failureOutput?: string
  input?: I
  positionals?: string[]
  /** Text read only from stdin (heredoc, pipe, or redirection), never argv. */
  stdinField?: string
  /**
   * Field receiving every token after a literal `--`, unparsed — for
   * commands that forward a raw argv. Declare the field as
   * `z.array(z.string())` (optional when `--` may be omitted).
   */
  restField?: string
  output?: CommandOutputSpec
  /**
   * Replaces the generic "check again with shell_status, or call yield" next
   * hint in model-facing shell results while this command is the running
   * foreground job. For long-running commands whose running state should not
   * be watched with status/yield polling (e.g. an attended child agent).
   */
  runningHint?: string
}

/**
 * A leaf whose implementation runs in the backend, against conversation or
 * platform state.
 *
 * `run` is a method so that a leaf declared with concrete input schemas still
 * belongs in a `Command[]` tree.
 */
export interface RpcCommand<I extends CommandInputSpec = CommandInputSpec>
  extends CommandLeafBase<I> {
  kind: 'rpc'
  run(ctx: CommandRunContext<I>): Promise<CommandResult> | CommandResult
}

/**
 * A leaf whose implementation runs in a native service on the execution target.
 */
export interface NativeCommand<I extends CommandInputSpec = CommandInputSpec>
  extends CommandLeafBase<I> {
  kind: 'native'
  binding: NativeBinding
}

export function isCommandGroup(command: Command): command is CommandGroup {
  return 'subcommands' in command
}

/**
 * Declares a leaf whose `parsed.values` follow its own input schemas. Without
 * it, a leaf written inside a `Command[]` literal is typed by the tree and its
 * values stay `unknown`.
 */
export function defineCommand<I extends CommandInputSpec>(
  leaf: CommandLeaf<I>
): CommandLeaf<I> {
  return leaf
}

export interface ParsedCommandInput<
  I extends CommandInputSpec = CommandInputSpec
> {
  /**
   * Path from root through the selected node, including the root name.
   * For help: path of the node help was requested for.
   */
  path: string[]
  /**
   * True when the invocation requested `--help`, or named a group with nothing
   * after it.
   */
  help: boolean
  /** The leaf's input, as its schemas validated it. */
  values: z.infer<z.ZodObject<I>>
  json: boolean
}

/** What an `rpc` handler receives. */
export interface CommandRunContext<
  I extends CommandInputSpec = CommandInputSpec
> {
  argv: string[]
  parsed: ParsedCommandInput<I>
  /**
   * The pipe: finite, read as it arrives (`runner.md` § Pipes). `null` when
   * the command was invoked without one — fd 0 is the shell's live stdin —
   * or when the leaf's `stdinField` consumed it into an argument.
   */
  stdin: AsyncIterable<Uint8Array> | null
  env: Record<string, string>
  cwd: string
  io: CommandIO
  storage: CommandStorage
  /** The Host the invoking shell runs against. */
  host: Host
  /**
   * Aborted when the shell command is aborted (shell_abort, shell teardown).
   */
  signal: AbortSignal
  /**
   * Stdin written after the command started: each `shell_write` call arrives
   * as one chunk. Ends when the command's shell job is released.
   */
  stdinStream: AsyncIterable<Uint8Array>
}


export interface CommandIO {
  stdout: CommandWriter
  stderr: CommandWriter
}

export interface CommandStorage {
  /** Preserve the original history binding while restricting an invocation's lifetime. */
  withSignal(signal: AbortSignal): CommandStorage
  /**
   * The stored value, or `null` when the key holds nothing. Stored values are
   * data from outside this process: the reader validates them with its own
   * schema before using them.
   */
  readJson(key: string): Promise<unknown>
  writeJson<T>(key: string, value: T): Promise<void>
  /**
   * Atomically replace a key from a detached value; the callback performs no
   * IO. The callback receives the stored value as `unknown` and decides its
   * type; the result is what the callback returned, detached.
   */
  updateJson<T>(key: string, update: (current: unknown) => T): Promise<T>
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
  /**
   * Session storage for `rpc` handlers run in this process; absent when every
   * rpc leaf forwards elsewhere.
   */
  storage?: CommandStorage
  host: Host
  signal?: AbortSignal
  /** Stdin written after the command started, for `rpc` handlers that steer. */
  stdinStream?: AsyncIterable<Uint8Array>
  /** The execution adapter for native package operations. */
  native?: NativeExecutor
  onRunningHint?: DispatchIO['onRunningHint']
}

const COMMAND_NAME_PATTERN = /^[A-Za-z0-9][A-Za-z0-9_-]*$/

export class CommandRegistry {
  private readonly commands = new Map<string, Command>()

  /**
   * `reserved`: names the executable namespace already owns (a shell's
   * builtins, the system tools); registering one is refused.
   */
  constructor(private readonly reserved: ReadonlySet<string> = new Set()) {}

  register(command: Command): void {
    if (this.reserved.has(command.name)) {
      throw new Error(
        `CommandRegistry: command "${command.name}" is reserved for shell/system commands`
      )
    }
    if (this.commands.has(command.name)) {
      throw new Error(
        `CommandRegistry: command "${command.name}" is already registered`
      )
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
    if (!rendered)
      return rendered
    return `${COMMAND_HELP_DEFAULTS}\n\n${rendered}`
  }
}

// Stated once for the whole registry so per-command renders only carry deviations.
export const COMMAND_HELP_DEFAULTS =
  'Unless a command states otherwise: success prints raw text on stdout, failure writes an error message to stderr and exits non-zero. Pass --help at any level to print a command\'s documentation. Usage uses <placeholders> for values and [brackets] for optional arguments. Quote values containing spaces. Stdin bodies use a quoted heredoc, pipe, or input redirection; they have no command-line option. Use --name=value for option values beginning with --, and -- before positional values beginning with --.'

/**
 * Resolves argv through the tree and parses the leaf's arguments. A group
 * with nothing after it is a help request for that group.
 */
export function parseCommandInput(
  root: Command,
  argv: string[],
  stdinText = ''
): ParsedCommandInput {
  const { node, path, index } = resolveArgv(root, argv)
  if (isCommandGroup(node))
    return { path, help: true, values: {}, json: false }
  return parseArgs(node, path, argv, index, stdinText)
}

/**
 * The node argv names, with the index of the first token that is an argument.
 */
function resolveArgv(
  root: Command,
  argv: string[]
): {
  node: Command;
  path: string[];
  index: number
} {
  if (argv[0] !== root.name) {
    throw new Error(
      `Expected command "${root.name}", received "${argv[0] ?? ''}"`
    )
  }
  let node: Command = root
  const path: string[] = [root.name]
  let index = 1
  while (isCommandGroup(node)) {
    const token = argv[index]
    // --help renders this node's documentation. A flag can never collide
    // with subcommand names (leading '-' is not a valid name), so no
    // reservation or routing precedence is needed.
    if (token === undefined || token === '--help')
      return { node, path, index }
    const child = node.subcommands.find((candidate) => candidate.name === token)
    if (!child)
      throw new Error(`Unknown subcommand "${[...path, token].join(' ')}"`)
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
  /** The pipe as text, for a leaf whose `stdinField` reads it. */
  stdinText: string,
): ParsedCommandInput {
  const displayPath = path.join(' ')
  const input = command.input ?? {}
  const tokens: Record<string, ArgvToken> = {}
  let json = false
  let positionalIndex = 0
  let optionsEnded = false

  for (let i = startIndex; i < argv.length; i += 1) {
    const token = argv[i]!
    if (!optionsEnded && token === '--') {
      if (command.restField) {
        tokens[command.restField] = argv.slice(i + 1)
        break
      }
      optionsEnded = true
      continue
    }
    if (!optionsEnded && token === '--help') {
      return { path: [...path], help: true, values: {}, json: false }
    }
    if (!optionsEnded && token === '--json') {
      if (!command.output?.json)
        throw new Error(`Command "${displayPath}" does not define JSON output`)
      json = true
      continue
    }

    if (!optionsEnded && token.startsWith('--')) {
      const equals = token.indexOf('=')
      const field = token.slice(2, equals === -1 ? undefined : equals)
      const schema = input[field]
      if (!schema)
        throw new Error(`Unknown option "--${field}" for "${displayPath}"`)
      const source = fieldSource(command, field)
      if (source === 'stdin') {
        throw new Error(
          `"${displayPath}" reads ${field} only from stdin. Remove --${field} and use a quoted heredoc, pipe, or input redirection.`,
        )
      }
      if (source !== 'option') {
        throw new Error(
          `"${field}" is ${source === 'rest' ? 'passed after --' : 'a positional argument'} for "${displayPath}"; --${field} is not an option`,
        )
      }

      const next = argv[i + 1]
      const takesImplicitBoolean = equals === -1 && isBooleanSchema(schema)
        && (next === undefined || next.startsWith('--'))
      const rawValue = equals !== -1
        ? token.slice(equals + 1)
        : takesImplicitBoolean ? true : next
      if (equals === -1 && !takesImplicitBoolean)
        i += 1
      if (rawValue === undefined ||
        (equals === -1 && typeof rawValue === 'string' && rawValue.startsWith('--')))
        throw new Error(`Missing value for "--${field}"`)
      setArgvToken(tokens, field, rawValue, schema)
      continue
    }

    const field = command.positionals?.[positionalIndex]
    if (!field)
      throw new Error(`Unexpected positional argument "${token}"`)
    setArgvToken(tokens, field, token, input[field])
    positionalIndex += 1
  }

  if (command.stdinField) {
    tokens[command.stdinField] = stdinText
  }

  // argv text becomes the value each field's schema expects. A token for a
  // field the leaf does not declare travels unchanged, for the validation
  // below to report as an unrecognized key.
  const values: Record<string, unknown> = {}
  for (const [field, token] of Object.entries(tokens)) {
    const schema = input[field]
    values[field] = schema === undefined ? token : argvValue(schema, token)
  }

  return {
    path: [...path],
    help: false,
    values: validateCommandValues(input, values),
    json,
  }
}

export function resolveCommand(root: Command, path: string[]): Command {
  if (path[0] !== root.name) {
    throw new Error(
      `Path root "${path[0] ?? ''}" does not match command "${root.name}"`
    )
  }
  let node: Command = root
  for (let i = 1; i < path.length; i += 1) {
    const segment = path[i]
    const child = isCommandGroup(node)
      ? node.subcommands.find((candidate) => candidate.name === segment)
      : undefined
    if (!child)
      throw new Error(`Unknown subcommand "${path.slice(0, i + 1).join(' ')}"`)
    node = child
  }
  return node
}

/**
 * Runs one invocation of a root command: help for a group or `--help`,
 * otherwise the leaf through an application RPC handler or an injected native executor.
 */
export async function runRegisteredCommand(
  root: Command,
  ctx: CommandExecutionContext
): Promise<CommandResult> {
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
  if (isCommandGroup(node))
    return help()

  // The pipe is drained before parsing only when the leaf reads it into a
  // field; otherwise it streams into the handler or module as it arrives.
  const consumed = node.stdinField !== undefined
  const parsed = parseArgs(
    node,
    path,
    ctx.argv,
    index,
    consumed ? decodeUtf8(await collectBytes(stdin)) : ''
  )
  if (parsed.help)
    return help()
  const capture = new CapturingIO(ctx.io)
  const io = parsed.json ? capture : ctx.io
  try {
    if (node.runningHint !== undefined)
      await ctx.onRunningHint?.(node.runningHint)
    throwIfAborted(signal)
    let result: CommandResult
    if (node.kind === 'rpc') {
      result = await node.run({
        argv: ctx.argv,
        parsed,
        stdin: consumed ? null : (ctx.stdin ?? null),
        env: ctx.env,
        cwd: ctx.cwd,
        io,
        storage: ctx.storage ?? unavailableStorage(displayPath),
        host: ctx.host,
        signal,
        stdinStream,
      })
    } else {
      if (!ctx.native)
        throw new Error(`"${displayPath}" requires a native command executor`)
      result = await ctx.native({
        binding: node.binding,
        args: parsed.values,
        cwd: ctx.cwd,
        env: ctx.env,
        stdin: consumed ? stdinStream : concatByteStreams(stdin, stdinStream),
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
        throw new Error(
          `Invalid JSON output for "${displayPath}": ${asError(error).message}`
        )
      }
      const validation = node.output?.json?.safeParse(json)
      if (!validation?.success) {
        const issue = validation?.error.issues[0]
        throw new Error(
          `JSON output failed validation for "${displayPath}": ${issue?.message}`
        )
      }
      await ctx.io.stdout(raw)
    }

    return { exitCode: result.exitCode }
  } finally {
    if (node.runningHint !== undefined)
      await ctx.onRunningHint?.(undefined)
  }
}

export function renderCommandHelp(command: Command, parentPath = ''): string {
  const path = parentPath ? `${parentPath} ${command.name}` : command.name
  const blocks: string[] = []

  const lines = [`${path}: ${command.summary}`]

  if (!isCommandGroup(command)) {
    lines.push('', 'Usage:')
    lines.push('', ...renderUsage(command, path).map(line => `  ${line}`))
    if (command.successOutput) lines.push(`    Success output: ${command.successOutput}`)
    else if (command.output?.json) {
      lines.push(
        '    Success output: raw text by default; machine-readable JSON when --json is passed'
      )
    }
    if (command.failureOutput)
      lines.push(`    Failure output: ${command.failureOutput}`)

    const fields = Object.entries(command.input ?? {})
      .filter(([field]) => field !== command.stdinField)
    if (fields.length > 0) {
      lines.push('    Parameters:')
      for (const [field, schema] of fields) {
        const source = fieldSource(command, field)
        const required = schema.isOptional() ? 'optional' : 'required'
        const repeatable = source === 'option' && isArraySchema(schema)
          ? ', repeatable'
          : ''
        lines.push(
          `      ${fieldSyntax(field, schema, source)} (${required}${repeatable})${fieldDescription(schema)}`,
        )
      }
    }

    if (command.stdinField) {
      const schema = command.input![command.stdinField]!
      lines.push(`    Stdin body: ${command.stdinField}${fieldDescription(schema)}`)
    }
    if (command.output?.json) {
      lines.push('    --json: emits machine-readable JSON for this command')
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
        throw new Error(
          `CommandRegistry: duplicate subcommand "${path} ${child.name}"`
        )
      }
      seen.add(child.name)
      validateCommandTree(child, `${path} ${child.name}`)
    }
    return
  }

  if (command.kind === 'rpc' && typeof command.run !== 'function') {
    throw new Error(`CommandRegistry: rpc leaf "${path}" has no run()`)
  }
  if (command.kind === 'native')
    nativeBindingSchema.parse(command.binding)
  const input = command.input ?? {}
  for (const [field, schema] of Object.entries(input)) {
    if (!COMMAND_NAME_PATTERN.test(field))
      throw new Error(`CommandRegistry: "${path}" has invalid input name "${field}"`)
    if (fieldSource(command, field) === 'option' && ['help', 'json'].includes(field))
      throw new Error(`CommandRegistry: "${path}" option "${field}" is reserved`)
    const unsupported = unsupportedInputSchema(schema)
    if (unsupported)
      throw new Error(`CommandRegistry: "${path}" input "${field}" ${unsupported}`)
  }
  if (command.stdinField && !(command.stdinField in input)) {
    throw new Error(
      `CommandRegistry: "${path}" stdinField "${command.stdinField}" is not in input`
    )
  }
  if (command.restField && !(command.restField in input)) {
    throw new Error(
      `CommandRegistry: "${path}" restField "${command.restField}" is not in input`
    )
  }
  if (command.stdinField && command.stdinField === command.restField) {
    throw new Error(`CommandRegistry: "${path}" field "${command.stdinField}" has multiple input sources`)
  }
  if (command.stdinField
    && !(unwrapSchema(input[command.stdinField]!) instanceof z.ZodString)) {
    throw new Error(`CommandRegistry: "${path}" stdinField must be a string`)
  }
  const seenPositionals = new Set<string>()
  let optionalPositional = false
  for (const positional of command.positionals ?? []) {
    if (!(positional in input)) {
      throw new Error(
        `CommandRegistry: "${path}" positional "${positional}" is not in input`
      )
    }
    if (positional === command.stdinField || positional === command.restField ||
      seenPositionals.has(positional)) {
      throw new Error(`CommandRegistry: "${path}" field "${positional}" has multiple input sources`)
    }
    seenPositionals.add(positional)
    if (input[positional]!.isOptional()) {
      optionalPositional = true
    } else if (optionalPositional) {
      throw new Error(`CommandRegistry: "${path}" required positional "${positional}" follows an optional positional`)
    }
  }
}

const INPUT_SUBSET = 'Command input allows string, number, boolean, enum, and '
  + 'arrays of those, with .optional() and .describe().'

const REFINEMENT_REASON =
  'carries a .refine() or .check() predicate, which JSON Schema drops, so a '
  + 'remote runner would accept what this process rejects'

const DEFAULT_REASON =
  'uses .default(), whose value the manifest round trip loses: a remote runner '
  + 'rebuilds the field as a plain optional and fills in nothing'

/**
 * Why a field schema falls outside the subset a command input may use, or
 * null when it is inside it.
 *
 * A command tree travels to a remote runner as JSON Schema
 * (`docs/demi-next/commands.md`): the subset is what survives that round trip
 * unchanged, so both ends accept the same values, and it is what the argv
 * parser and help renderer know how to spell.
 */
export function unsupportedInputSchema(schema: z.ZodType): string | null {
  const reason = fieldSchemaReason(schema)
  return reason === null ? null : `${reason}. ${INPUT_SUBSET}`
}

function fieldSchemaReason(schema: z.ZodType): string | null {
  if (hasCustomCheck(schema))
    return REFINEMENT_REASON
  if (schema instanceof z.ZodDefault)
    return DEFAULT_REASON
  if (schema instanceof z.ZodOptional)
    return fieldSchemaReason(classicSchema(schema.unwrap()))
  if (schema instanceof z.ZodArray) {
    const element = elementSchemaReason(classicSchema(schema.element))
    return element === null ? null : `has an array element that ${element}`
  }
  return scalarSchemaReason(schema)
}

/** An array element takes no wrapper of its own, and no array of its own. */
function elementSchemaReason(element: z.ZodType): string | null {
  return hasCustomCheck(element) ? REFINEMENT_REASON : scalarSchemaReason(element)
}

function scalarSchemaReason(schema: z.ZodType): string | null {
  const supported = schema instanceof z.ZodString
    || schema instanceof z.ZodNumber
    || schema instanceof z.ZodBoolean
    || schema instanceof z.ZodEnum
  if (supported)
    return null
  return `uses an unsupported "${schema.type}" schema`
}

/**
 * True when a refinement rides along with the schema's own constraints.
 *
 * A check's kind is readable only through `_zod`, zod's internals namespace:
 * the classic API exposes the checks array on `def` but no accessor for what
 * each check is. Both members are declared in zod's types (`$ZodTypeDef.checks`
 * and `$ZodCheckInternals.def.check`), so this reads them without a cast.
 */
function hasCustomCheck(schema: z.ZodType): boolean {
  const checks = schema.def.checks ?? []
  return checks.some((check) => check._zod.def.check === 'custom')
}

function unavailableStorage(displayPath: string): CommandStorage {
  const refuse = () => {
    throw new Error(
      `"${displayPath}" reads command storage, and this embedder runs rpc commands without one`
    )
  }
  return { withSignal: refuse, readJson: refuse, writeJson: refuse, updateJson: refuse, delete: refuse, list: refuse }
}

/** One field as argv spelled it: a value, a bare boolean flag, or repetitions. */
export type ArgvToken = string | true | (string | true)[]

function setArgvToken(
  tokens: Record<string, ArgvToken>,
  field: string,
  token: string | true,
  schema: z.ZodType | undefined,
): void {
  const existing = tokens[field]
  if (existing === undefined) {
    tokens[field] = token
    return
  }
  if (schema === undefined || !isArraySchema(schema))
    throw new Error(`Duplicate value for "${field}"`)
  tokens[field] = Array.isArray(existing)
    ? [...existing, token]
    : [existing, token]
}

/**
 * The value an argv token stands for under a field's schema: argv carries only
 * text, while the schemas describe numbers, booleans and arrays. This is the
 * CLI path alone — an RPC invocation carries decoded JSON and is validated as
 * it arrives, without conversion.
 */
export function argvValue(schema: z.ZodType, token: ArgvToken): unknown {
  const inner = unwrapSchema(schema)
  if (inner instanceof z.ZodArray) {
    const element = classicSchema(inner.element)
    const items = Array.isArray(token) ? token : [token]
    return items.map((item) => argvValue(element, item))
  }
  if (inner instanceof z.ZodNumber && typeof token === 'string'
    && token.trim() !== '')
    return Number(token)
  if (inner instanceof z.ZodBoolean) {
    if (token === 'true')
      return true
    if (token === 'false')
      return false
  }
  return token
}

/**
 * Validates one invocation's values against the leaf's whole input: unknown
 * fields are rejected, and the error carries every issue with its field.
 */
export function validateCommandValues<I extends CommandInputSpec>(
  input: I,
  values: unknown
): z.infer<z.ZodObject<I>> {
  const result = z.strictObject(input).safeParse(values)
  if (!result.success)
    throw new Error(result.error.issues.map(issueMessage).join('; '))
  return result.data
}

function issueMessage(issue: z.core.$ZodIssue): string {
  const field = issue.path.join('.')
  return field === ''
    ? issue.message
    : `Invalid value for "${field}": ${issue.message}`
}

type FieldSource = 'stdin' | 'positional' | 'rest' | 'option'

function fieldSource(command: CommandLeaf, field: string): FieldSource {
  if (command.stdinField === field)
    return 'stdin'
  if (command.restField === field)
    return 'rest'
  return command.positionals?.includes(field) ? 'positional' : 'option'
}

function renderUsage(command: CommandLeaf, path: string): string[] {
  const input = command.input ?? {}
  const fields = [
    ...(command.positionals ?? []),
    ...Object.keys(input).filter(field => fieldSource(command, field) === 'option'),
    ...(command.restField ? [command.restField] : []),
  ]
  const arguments_ = fields.map(field => {
    const schema = input[field]!
    const syntax = fieldSyntax(field, schema, fieldSource(command, field))
    return schema.isOptional() ? `[${syntax}]` : syntax
  })
  if (command.output?.json)
    arguments_.splice(command.restField ? arguments_.length - 1 : arguments_.length, 0, '[--json]')
  const invocation = [path, ...arguments_].join(' ')
  return command.stdinField
    ? [`${invocation} <<'EOF'`, `<${command.stdinField}>`, 'EOF']
    : [invocation]
}

function fieldSyntax(
  field: string,
  schema: z.ZodType,
  source: FieldSource,
): string {
  if (source === 'positional')
    return `<${field}>`
  if (source === 'rest')
    return `-- <${field}>...`
  const unwrapped = unwrapSchema(schema)
  if (unwrapped instanceof z.ZodBoolean)
    return `--${field} [true|false]`
  const label = unwrapped instanceof z.ZodEnum
    ? unwrapped.options.join('|')
    : field
  return `--${field} <${label}>`
}

function fieldDescription(schema: z.ZodType): string {
  // A reconstructed optional schema carries its description on the inner type.
  const text = schema.description ?? unwrapSchema(schema).description
  return text ? ` - ${text}` : ''
}

function isArraySchema(schema: z.ZodType): boolean {
  return unwrapSchema(schema) instanceof z.ZodArray
}

function isBooleanSchema(schema: z.ZodType): boolean {
  return unwrapSchema(schema) instanceof z.ZodBoolean
}

/** The value schema under `.optional()`, the one wrapper a command input allows. */
export function unwrapSchema(schema: z.ZodType): z.ZodType {
  let current = schema
  while (current instanceof z.ZodOptional) {
    current = classicSchema(current.unwrap())
  }
  return current
}

/**
 * zod declares `unwrap()` and `element` as the core schema interface, which
 * carries no `type` or `description`. Every schema a command input holds is a
 * classic one, and this restores that for the members the CLI reads.
 */
function classicSchema(schema: z.core.$ZodType): z.ZodType {
  if (!(schema instanceof z.ZodType))
    throw new Error('Command input schemas must come from the zod entrypoint')
  return schema
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
