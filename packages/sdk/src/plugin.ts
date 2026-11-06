/**
 * A program's plugin (`crates/plugin-interface/src/lib.rs`): its manifest,
 * which `definePlugin` builds from a plain object, and its handlers, which
 * answer the requests each core sends on the plugin's lane of the core's
 * connection, in the messages of `crates/plugin-interface/src/wire.rs`.
 *
 * ```text
 * core                                       program
 *   { request: 7, call: { type: "command", invocation } } ---->  run(input, call)
 *        <---- { request: 7, port: 1, message: { type: "rpc", request: stdout } }
 *   { request: 7, port: 1, answer: { type: "rpc", ... } } ---->
 *        <---- { request: 7, reply: { type: "exit", code: 0 } }
 * ```
 */
import { z } from 'zod'
import {
  hostMessageSchema,
  type Category,
  type Commands,
  type Group,
  type HostMessage,
  type Leaf,
  type Manifest,
  type MediumFacts,
  type Node,
  type PluginError,
  type PluginMessage,
  type PortAnswer,
  type PortMessage,
  type PortRequest,
  type PortResponse,
  type Reply,
  type Request as PluginRequest,
  type RpcInvocation,
  schemaRefusal,
} from '@demicodes/protocol'
import { decodeApart, encodeApart } from './apart'

/** A command group under `demi`, as `GroupBuilder` declares one. */
export interface GroupDefinition {
  summary: string
  details?: string
  /** Its sentence of the `demi` index, such as `Posts to the chat.` */
  index?: string
  /** The permission categories its leaves need. */
  permissions?: Category[]
  commands: Record<string, GroupDefinition | LeafDefinition<z.ZodType>>
}

/** A command the plugin runs itself, as `LeafBuilder` declares an `rpc` leaf. */
export interface LeafDefinition<Input extends z.ZodType> {
  summary: string
  /** Its arguments, whose JSON Schema is the leaf's input; each field's `describe` is its description. */
  input?: Input
  /** The fields given by their place, in order. */
  positionals?: string[]
  positionalOptions?: string[]
  /** The field the standard input fills, and when it is read. */
  stdin?: string | { field: string; unless: string[] } | { field: string; with: string[] }
  /** The field that takes every word after the others. */
  rest?: string
  /** What its standard output holds. */
  output?: string
  /** The JSON its `--json` writes, as a schema. */
  jsonOutput?: z.ZodType
  /** What a failure leaves. */
  failure?: string
  /** Whether it hands its job a medium. */
  media?: boolean
  runningHint?: string
  /** The permission category it needs. */
  permission?: string
  /** The permission category it needs when the field is set. */
  permissionWhen?: { category: string; field: string }
  /** The field that names the Host it brings. */
  bringsHost?: string
  /** Runs the command; its exit status is what it returns, 0 when it returns none. */
  run(input: z.output<Input>, call: CommandCall): Promise<number | void> | number | void
}

/** The leaf of `definition`, its input inferred. */
export function leaf<Input extends z.ZodType>(definition: LeafDefinition<Input>): LeafDefinition<Input> {
  return definition
}

/** A plugin as a program defines it. */
export interface PluginDefinition {
  id: string
  /** Its name in settings, in title style. */
  name: string
  /** What it does for the user, in one sentence that names its command. */
  description: string
  /** Its command groups under `demi`, by name. */
  commands?: Record<string, GroupDefinition>
  /** The context block it adds before each request of a node, if any. */
  context?(request: Extract<PluginRequest, { type: 'context' }>, port: PluginPort): Promise<string | undefined>
  /** Searches a conversation's project directory, what it found and what to watch. */
  projectSearch?(request: Extract<PluginRequest, { type: 'project_search' }>, port: PluginPort): Promise<Extract<Reply, { type: 'project' }>>
  onConversationDeleted?(request: Extract<PluginRequest, { type: 'conversation_deleted' }>, port: PluginPort): Promise<void>
  onConversationForked?(request: Extract<PluginRequest, { type: 'conversation_forked' }>, port: PluginPort): Promise<void>
}

/** A plugin: its manifest, and the definition that answers its requests. */
export interface Plugin {
  readonly manifest: Manifest
  readonly definition: PluginDefinition
}

/** The plugin `definition` defines. */
export function definePlugin(definition: PluginDefinition): Plugin {
  const commands: Commands[] = Object.entries(definition.commands ?? {}).map(([name, group]) => ({
    placement: 'demi',
    tree: groupNode(name, group),
    calls: [],
  }))
  const manifest: Manifest = {
    id: definition.id,
    name: definition.name,
    description: definition.description,
    commands,
    features: [],
    context: definition.context !== undefined,
    projectSearch: definition.projectSearch !== undefined,
    conversations: definition.onConversationDeleted !== undefined || definition.onConversationForked !== undefined,
    streams: [],
  }
  return { manifest, definition }
}

function isGroup(node: GroupDefinition | LeafDefinition<z.ZodType>): node is GroupDefinition {
  return 'commands' in node
}

function groupNode(name: string, group: GroupDefinition): Group {
  return {
    name,
    summary: group.summary,
    ...(group.index === undefined ? {} : { indexEntry: group.index }),
    ...(group.details === undefined ? {} : { details: group.details }),
    ...(group.permissions === undefined ? {} : { permissions: group.permissions }),
    subcommands: Object.entries(group.commands).map(([child, node]): Node =>
      isGroup(node) ? groupNode(child, node) : leafNode(child, node)),
  }
}

/**
 * A JSON Schema of `schema`, as a leaf declares its input or output. An
 * input's object without a catchall refuses unknown fields, as a command
 * refuses an unknown option, where zod's input schema would let them pass.
 */
function jsonSchema(schema: z.ZodType, io: 'input' | 'output'): Record<string, z.core.JSONSchema.JSONSchema> {
  const { $schema: _, ...document } = z.toJSONSchema(schema, {
    io,
    override: ({ zodSchema, jsonSchema }) => {
      const def = zodSchema._zod.def
      if (def.type === 'object' && def.catchall === undefined) {
        jsonSchema.additionalProperties = false
      }
    },
  }) as Record<string, z.core.JSONSchema.JSONSchema>
  return document
}

function leafNode(name: string, leaf: LeafDefinition<z.ZodType>): Leaf {
  const stdin = leaf.stdin
  const output = leaf.output !== undefined || leaf.jsonOutput !== undefined
    ? {
        ...(leaf.output === undefined ? {} : { description: leaf.output }),
        ...(leaf.jsonOutput === undefined ? {} : { json: jsonSchema(leaf.jsonOutput, 'output') }),
      }
    : undefined
  return {
    name,
    summary: leaf.summary,
    kind: 'rpc',
    ...(leaf.input === undefined ? {} : { input: jsonSchema(leaf.input, 'input') }),
    ...(leaf.positionals === undefined ? {} : { positionals: leaf.positionals }),
    ...(leaf.positionalOptions === undefined ? {} : { positionalOptions: leaf.positionalOptions }),
    ...(stdin === undefined ? {} : typeof stdin === 'string'
      ? { stdinField: stdin }
      : 'unless' in stdin
        ? { stdinField: stdin.field, stdinRead: { unless: stdin.unless } }
        : { stdinField: stdin.field, stdinRead: { with: stdin.with } }),
    ...(leaf.rest === undefined ? {} : { restField: leaf.rest }),
    ...(output === undefined ? {} : { output }),
    ...(leaf.failure === undefined ? {} : { failure: leaf.failure }),
    ...(leaf.media ? { media: true } : {}),
    ...(leaf.runningHint === undefined ? {} : { runningHint: leaf.runningHint }),
    ...(leaf.permissionWhen !== undefined
      ? { permission: leaf.permissionWhen.category, permissionField: leaf.permissionWhen.field }
      : leaf.permission === undefined ? {} : { permission: leaf.permission }),
    ...(leaf.bringsHost === undefined ? {} : { bringsHost: { field: leaf.bringsHost } }),
  } as Leaf
}

/** The leaf `path` names under the plugin's groups. */
function leafAt(definition: PluginDefinition, path: readonly string[]): LeafDefinition<z.ZodType> | undefined {
  const [first, ...rest] = path
  let node: GroupDefinition | LeafDefinition<z.ZodType> | undefined = first === undefined ? undefined : definition.commands?.[first]
  for (const name of rest) {
    node = node !== undefined && isGroup(node) ? node.commands[name] : undefined
  }
  return node !== undefined && !isGroup(node) ? node : undefined
}

/** A port operation the core refused, in its words. */
export class PortFailure extends Error {
  constructor(message: string, readonly refusal?: unknown) {
    super(message)
    this.name = 'PortFailure'
  }
}

/** What a request asks of Demi through its port: any port message, answered. */
export interface PluginPort {
  /** Asks `message` of Demi, as `PluginPort` in Rust does, and answers its answer. */
  ask(message: PortMessage): Promise<PortAnswer>
  /** Aborted once the request is stopped. */
  readonly signal: AbortSignal
}

/** A command's call: its invocation, its standard streams, and its port. */
export interface CommandCall extends PluginPort {
  readonly invocation: RpcInvocation
  /** Writes to the command's standard output; resolves once the caller read enough. */
  stdout(data: string | Uint8Array): Promise<void>
  stderr(data: string | Uint8Array): Promise<void>
  /** The next chunk of the standard input; null once it ended. */
  readStdin(): Promise<Uint8Array | null>
  /** Says whether the call waits for the user's answer. */
  awaitsUser(awaits: boolean): Promise<void>
  /** Hands the command's job a medium, as a leaf that declares `media` may. */
  medium(facts: MediumFacts, bytes: Uint8Array): Promise<void>
}

/** A request in flight: its port messages' answers, and its cancellation. */
interface Pending {
  answers: Map<number, { resolve: (answer: PortAnswer) => void; reject: (error: Error) => void }>
  nextPort: number
  stop: AbortController
}

/** The part of a lane a plugin's lane takes: messages, sent and received. */
export interface PluginLane {
  binaryType: 'blob' | 'arraybuffer'
  send(data: string | ArrayBufferView): void
  addEventListener(type: 'message', listener: (event: MessageEvent) => void): void
  addEventListener(type: 'close', listener: () => void): void
}

/**
 * Answers the requests `lane` brings of `plugin`, until the lane closes;
 * `failed` hears a message that does not read, which ends nothing.
 */
export function servePlugin(plugin: Plugin, lane: PluginLane, failed: (error: Error) => void): void {
  lane.binaryType = 'arraybuffer'
  const pending = new Map<number, Pending>()
  const send = (message: PluginMessage) => {
    const encoded = encodeApart(message)
    lane.send(encoded)
  }
  lane.addEventListener('message', (event) => {
    let message: HostMessage
    try {
      const parsed = hostMessageSchema.safeParse(decodeApart(event.data as string | ArrayBuffer))
      if (!parsed.success) {
        throw new Error(schemaRefusal(parsed.error))
      }
      message = parsed.data
    } catch (error) {
      failed(error instanceof Error ? error : new Error(String(error)))
      return
    }
    if ('call' in message) {
      const request: Pending = { answers: new Map(), nextPort: 1, stop: new AbortController() }
      pending.set(message.request, request)
      const number = message.request
      void answer(plugin.definition, message.call, portOf(number, request, send)).then(
        (reply) => send({ request: number, reply }),
        (error: unknown) => send({ request: number, error: pluginError(error) }),
      ).finally(() => {
        pending.delete(number)
        for (const waiting of request.answers.values()) {
          waiting.reject(new PortFailure('The request is over'))
        }
      })
      return
    }
    const request = pending.get(message.request)
    if ('cancel' in message) {
      request?.stop.abort(new Error('The request was stopped'))
      return
    }
    const waiting = request?.answers.get(message.port)
    request?.answers.delete(message.port)
    if ('answer' in message) {
      waiting?.resolve(message.answer)
    } else {
      waiting?.reject(new PortFailure(message.error))
    }
  })
  lane.addEventListener('close', () => {
    for (const request of pending.values()) {
      request.stop.abort(new Error('The plugin\'s lane ended'))
      for (const waiting of request.answers.values()) {
        waiting.reject(new PortFailure('The plugin\'s lane ended'))
      }
    }
    pending.clear()
  })
}

/** The port of the request `number`. */
function portOf(number: number, request: Pending, send: (message: PluginMessage) => void): PluginPort {
  return {
    signal: request.stop.signal,
    ask: (message) => new Promise<PortAnswer>((resolve, reject) => {
      const port = request.nextPort
      request.nextPort += 1
      request.answers.set(port, { resolve, reject })
      send({ request: number, port, message })
    }),
  }
}

/** The command call of `invocation` on `port`. */
function commandCall(invocation: RpcInvocation, port: PluginPort): CommandCall {
  const rpc = async (request: PortRequest): Promise<PortResponse> => {
    const answer = await port.ask({ type: 'rpc', request })
    if (answer.type !== 'rpc') {
      throw new PortFailure(`The port answered ${answer.type} to an rpc operation`)
    }
    return answer.response
  }
  const bytes = (data: string | Uint8Array): Uint8Array<ArrayBuffer> => typeof data === 'string'
    ? new TextEncoder().encode(data)
    : data.buffer instanceof ArrayBuffer
      ? (data as Uint8Array<ArrayBuffer>)
      : new Uint8Array(data)
  return {
    ...port,
    invocation,
    stdout: async (data) => void (await rpc({ type: 'stdout', bytes: bytes(data) })),
    stderr: async (data) => void (await rpc({ type: 'stderr', bytes: bytes(data) })),
    readStdin: async () => {
      const response = await rpc({ type: 'read_stdin' })
      return response.type === 'input' ? (response.bytes ?? null) : null
    },
    awaitsUser: async (awaits) => void (await rpc({ type: 'awaits_user', awaits })),
    medium: async (facts, data) => void (await rpc({ type: 'medium', facts, bytes: bytes(data) })),
  }
}

/** The reply to `call`, by the definition's handler of it. */
async function answer(definition: PluginDefinition, call: PluginRequest, port: PluginPort): Promise<Reply> {
  switch (call.type) {
    case 'command': {
      const leaf = leafAt(definition, call.invocation.path)
      if (leaf === undefined) {
        throw usage(`No command ${call.invocation.path.join(' ')}`)
      }
      const parsed = (leaf.input ?? z.object({})).safeParse(call.invocation.args)
      if (!parsed.success) {
        throw usage(schemaRefusal(parsed.error))
      }
      const commandPort = commandCall(call.invocation, port)
      try {
        const code = await leaf.run(parsed.data, commandPort)
        return { type: 'exit', code: code ?? 0 }
      } catch (error) {
        if (error instanceof PortFailure) {
          throw error
        }
        await commandPort.stderr(`${error instanceof Error ? error.message : String(error)}\n`)
        return { type: 'exit', code: 1 }
      }
    }
    case 'context': {
      const text = await definition.context?.(call, port)
      return text === undefined ? { type: 'context' } : { type: 'context', text }
    }
    case 'project_search':
      if (definition.projectSearch === undefined) {
        throw undeclared('a project search')
      }
      return await definition.projectSearch(call, port)
    case 'conversation_deleted':
      await definition.onConversationDeleted?.(call, port)
      return { type: 'done' }
    case 'conversation_forked':
      await definition.onConversationForked?.(call, port)
      return { type: 'done' }
    default:
      throw undeclared(call.type)
  }
}

class RequestFailure extends Error {
  constructor(readonly error: PluginError) {
    super('message' in error ? error.message : error.type)
  }
}

function usage(message: string): RequestFailure {
  return new RequestFailure({ type: 'usage', message })
}

function undeclared(what: string): RequestFailure {
  return new RequestFailure({ type: 'failed', message: `The plugin declares no ${what}` })
}

function pluginError(error: unknown): PluginError {
  if (error instanceof RequestFailure) {
    return error.error
  }
  return { type: 'failed', message: error instanceof Error ? error.message : String(error) }
}
