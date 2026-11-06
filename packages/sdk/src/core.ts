/**
 * A core a program opened: its product state, followed on the
 * synchronization lane; its conversations, providers and Hosts, through its
 * routes; its notifications, on the starter lane; the program's plugins, on
 * their lanes; and its routes and page connection for the program's own
 * server to forward a page's to.
 */
import { spawn, type ChildProcess } from 'node:child_process'
import { join } from 'node:path'
import type { Readable, Writable } from 'node:stream'
import { PageConnection, type ConnectionHost, type Lane } from '@demicodes/conversation-client'
import {
  accountsSchema,
  activeAccountSchema,
  codeTakenSchema,
  coreStateSchema,
  createdConversationSchema,
  deviceAnswerSchema,
  loginStartedSchema,
  providerAnswerSchema,
  providersSchema,
  removedAccountSchema,
  schemaRefusal,
  starterRequestSchema,
  syncEventSchema,
  withPart,
  type Accounts,
  type CoreState,
  type CreateConversation,
  type CreateProvider,
  type DeviceDto,
  type ProviderDto,
  type ProviderPatch,
  type PushMessage,
  type StarterAnswer,
  type SubscriptionLogin,
} from '@demicodes/protocol'
import { randomUUID } from 'node:crypto'
import { Conversation } from './conversation'
import { servePlugin, type Plugin } from './plugin'
import { PROCESS_ORIGIN, CoreRoutes } from './routes'
import type { Process } from './process'
import { StreamSocket } from './socket'

/** How long the connection may bring nothing: two and a half of the core's 30-second heartbeat. */
const SILENCE_MS = 75_000
const FIRST_WAIT_MS = 1_000
const LONGEST_WAIT_MS = 30_000

/** How a notification went, as the program answers the core: `id` aside. */
export type Notified = StarterAnswer extends infer Answer ? (Answer extends { id: number } ? Omit<Answer, 'id'> : never) : never

/** What a core is opened with. */
export interface CoreOpening {
  process: Process
  id: string
  dir: string
  plugins: readonly Plugin[]
  /** The package's own runner, which `host.spawn()` starts with no command. */
  runner: string | null
}

export class Core {
  readonly id: string
  /** The directory the core's data is in. */
  readonly dir: string
  readonly conversations: Conversations
  readonly providers: Providers
  readonly hosts: Hosts
  private readonly routes: CoreRoutes
  private readonly connection: PageConnection
  // Read by `follow`, which opening the core awaits before it hands the core over.
  private stateRead!: CoreState
  private readonly stateListeners = new Set<(state: CoreState) => void>()
  private notify: ((message: PushMessage) => Promise<Notified> | Notified) | null = null
  private starter: Lane | null = null
  private sync: Lane | null = null
  private closed = false

  constructor(opening: CoreOpening, private readonly closing: () => Promise<void>) {
    this.id = opening.id
    this.dir = opening.dir
    this.connection = new PageConnection(processConnection(opening.process, `/cores/${encodeURIComponent(opening.id)}/api/connect`))
    this.connection.start()
    this.routes = new CoreRoutes(opening.process, opening.id, this.connection)
    this.conversations = new Conversations(this.routes, () => this.stateRead)
    this.providers = new Providers(this.routes)
    this.hosts = new Hosts(this.routes, opening.process, opening.runner, opening.dir)
    for (const plugin of opening.plugins) {
      this.servePlugin(plugin)
    }
  }

  /** Reads the core's product state and follows it from then on. */
  async follow(): Promise<void> {
    this.stateRead = await this.routes.json('GET', '/state', coreStateSchema)
    this.openSync()
  }

  /** The core's product state, as the synchronization lane last left it. */
  get state(): CoreState {
    return this.stateRead
  }

  /**
   * Calls `listener` with the product state at each change of it, or with
   * each notification the core sends, whose answer says how it went;
   * answers what stops it. Notifications reach the program only while it
   * listens for them.
   */
  on(event: 'state', listener: (state: CoreState) => void): () => void
  on(event: 'notify', listener: (message: PushMessage) => Promise<Notified> | Notified): () => void
  on(event: 'state' | 'notify', listener: ((state: CoreState) => void) | ((message: PushMessage) => Promise<Notified> | Notified)): () => void {
    if (event === 'state') {
      const each = listener as (state: CoreState) => void
      this.stateListeners.add(each)
      return () => this.stateListeners.delete(each)
    }
    this.notify = listener as (message: PushMessage) => Promise<Notified> | Notified
    this.openStarter()
    return () => {
      this.notify = null
      this.starter?.close()
      this.starter = null
    }
  }

  /**
   * Answers `request` with the core's route it names below `base`, such as
   * `/demi/api/conversations` below `/demi/api`, as the program's server
   * forwards a page's request.
   */
  fetch(request: Request, base = '/api'): Promise<Response> {
    const url = new URL(request.url)
    const path = url.pathname.startsWith(base) ? url.pathname.slice(base.length) : url.pathname
    const forwarded = new Request(PROCESS_ORIGIN + this.routes.path(path) + url.search, request)
    return this.routes.process.fetch(forwarded)
  }

  /**
   * Carries the page connection a page opened on the program's server,
   * `socket`, to the core, both ways, until either end closes; `url` is the
   * address the page opened, whose query goes with it.
   */
  accept(socket: AcceptedSocket, url?: string | URL): void {
    const query = url === undefined ? '' : new URL(url, PROCESS_ORIGIN).search
    const core = new StreamSocket(this.routes.process.connect(this.routes.path(`/connect${query}`), 'websocket', { 'sec-websocket-version': '13' }))
    const early: (string | ArrayBuffer | Uint8Array)[] = []
    let open = false
    let pageClosed = false
    socket.binaryType = 'arraybuffer'
    socket.addEventListener('message', (event) => {
      const data = event.data as string | ArrayBuffer | Uint8Array
      if (open) {
        core.send(data)
      } else {
        early.push(data)
      }
    })
    socket.addEventListener('close', () => {
      pageClosed = true
      core.close()
    })
    core.addEventListener('open', () => {
      open = true
      for (const data of early.splice(0)) {
        core.send(data)
      }
    })
    core.addEventListener('message', (event) => socket.send(event.data))
    core.addEventListener('close', (event) => {
      if (!pageClosed) {
        socket.close(event.code === 1006 || event.code === 1005 ? 1011 : event.code, event.reason)
      }
    })
  }

  /** Closes the core: its lanes end, and the process closes it. */
  async close(): Promise<void> {
    if (this.closed) {
      return
    }
    this.closed = true
    this.conversations.closeAll()
    this.connection.end()
    await this.closing()
  }

  private openSync(): void {
    if (this.closed) {
      return
    }
    const lane = this.connection.openLane({ kind: 'sync' }, 'synchronization channel')
    this.sync = lane
    lane.addEventListener('open', () => {
      const version = this.stateRead.version
      lane.send(JSON.stringify({ type: 'open', follows: version ? [{ value: 'state', holds: { kind: 'version', version } }] : [] }))
    })
    lane.addEventListener('message', (message) => {
      const parsed = syncEventSchema.safeParse(JSON.parse(message.data as string))
      if (!parsed.success) {
        process.emitWarning(schemaRefusal(parsed.error))
        return
      }
      const event = parsed.data
      switch (event.type) {
        case 'synced':
          this.setState({ ...this.stateRead, version: event.version })
          return
        case 'stale':
          void this.routes.json('GET', '/state', coreStateSchema).then((state) => this.setState(state))
          return
        case 'update':
          return
      }
      this.setState(withPart(this.stateRead, event))
    })
    lane.addEventListener('close', () => {
      if (this.sync === lane && !this.closed && lane.refusal === null) {
        this.openSync()
      }
    })
  }

  private setState(state: CoreState): void {
    this.stateRead = state
    for (const listener of [...this.stateListeners]) {
      listener(state)
    }
  }

  private openStarter(): void {
    if (this.closed || this.starter !== null) {
      return
    }
    const lane = this.connection.openLane({ kind: 'starter' }, 'starter lane')
    this.starter = lane
    lane.addEventListener('message', (message) => {
      const parsed = starterRequestSchema.safeParse(JSON.parse(message.data as string))
      if (!parsed.success) {
        process.emitWarning(schemaRefusal(parsed.error))
        return
      }
      const { id, message: notification } = parsed.data
      const notify = this.notify
      void Promise.resolve(notify === null ? ({ type: 'nowhere' } as Notified) : notify(notification))
        .catch((): Notified => ({ type: 'nowhere' }))
        .then((notified) => lane.send(JSON.stringify({ ...notified, id } satisfies StarterAnswer)))
    })
    lane.addEventListener('close', () => {
      if (this.starter === lane) {
        this.starter = null
        if (!this.closed && this.notify !== null && lane.refusal === null) {
          this.openStarter()
        }
      }
    })
  }

  private servePlugin(plugin: Plugin): void {
    if (this.closed) {
      return
    }
    const lane = this.connection.openLane({ kind: 'plugin', plugin: plugin.manifest.id }, `plugin lane of ${plugin.manifest.id}`)
    servePlugin(plugin, lane, (error) => process.emitWarning(error))
    lane.addEventListener('close', () => {
      if (lane.refusal !== null) {
        process.emitWarning(`The core refused the lane of the plugin ${plugin.manifest.id}: ${lane.refusal.error.message}`)
      } else if (!this.closed) {
        this.servePlugin(plugin)
      }
    })
  }
}

/** A socket a program's server accepted, as a WebSocket of the platform's. */
export interface AcceptedSocket {
  binaryType: string
  send(data: string | ArrayBuffer | Uint8Array): void
  close(code?: number, reason?: string): void
  addEventListener(type: 'message', listener: (event: { data: unknown }) => void): void
  addEventListener(type: 'close', listener: () => void): void
}

/** The page connection's host for a core of the process. */
function processConnection(process: Process, path: string): ConnectionHost {
  return {
    open: () => new StreamSocket(process.connect(path, 'websocket', { 'sec-websocket-version': '13' })),
    watch: (broken) => {
      let timer = setTimeout(broken, SILENCE_MS)
      return {
        heard: () => {
          clearTimeout(timer)
          timer = setTimeout(broken, SILENCE_MS)
        },
        ponged: () => {},
        stop: () => clearTimeout(timer),
      }
    },
    wait: (failures, _restarting, connect) => {
      const timer = setTimeout(connect, Math.min(FIRST_WAIT_MS * 2 ** (failures - 1), LONGEST_WAIT_MS))
      return { cancel: () => clearTimeout(timer) }
    },
    reached: () => {},
    failedToOpen: () => {},
    changed: () => {},
    log: { info: () => {}, warn: () => {} },
  }
}

/** What a new conversation is created with: the web API's; without an id, a new one, and without a workspace, the core's first. */
export type ConversationOptions = Partial<CreateConversation>

export class Conversations {
  /** Each conversation the program drives, one for each id. */
  private readonly opened = new Map<string, Conversation>()

  constructor(private readonly routes: CoreRoutes, private readonly state: () => CoreState) {}

  /** Creates a conversation; its workspace is the core's first unless named. */
  async create(options: ConversationOptions = {}): Promise<Conversation> {
    const workspaceId = options.workspaceId ?? this.state().workspaces?.[0]?.id
    if (workspaceId === undefined) {
      throw new Error('The core has no workspace to create the conversation in')
    }
    const body: CreateConversation = { ...options, id: options.id ?? randomUUID(), workspaceId }
    const created = await this.routes.json('POST', '/conversations', createdConversationSchema, body)
    return this.open(created.conversation.id)
  }

  /** The conversation `id`, whose lane opens at its first use. */
  open(id: string): Conversation {
    let conversation = this.opened.get(id)
    if (conversation === undefined) {
      conversation = new Conversation(id, this.routes)
      this.opened.set(id, conversation)
    }
    return conversation
  }

  /** Closes every conversation, as the core closes. */
  closeAll(): void {
    for (const conversation of this.opened.values()) {
      conversation.close()
    }
    this.opened.clear()
  }
}

/** A sign-in to a subscription, which finishes as its user confirms it; its stages are the product state's `logins`. */
export interface SignIn {
  readonly id: string
  /** Hands the sign-in the code its user pasted. */
  submitCode(code: string): Promise<void>
  cancel(): Promise<void>
}

/** The accounts of a provider entry. */
export interface ProviderAccounts {
  list(): Promise<Accounts>
  /** Makes `credentialId` the active account. */
  activate(credentialId: string): Promise<void>
  /** Orders the accounts, each once. */
  order(credentialIds: string[]): Promise<void>
  remove(credentialId: string): Promise<void>
  /** Signs in another account. */
  signIn(): Promise<SignIn>
}

export class Providers {
  constructor(private readonly routes: CoreRoutes) {}

  async list(): Promise<ProviderDto[]> {
    return (await this.routes.json('GET', '/providers', providersSchema)).providers
  }

  async create(provider: CreateProvider): Promise<ProviderDto> {
    return (await this.routes.json('POST', '/providers', providerAnswerSchema, provider)).provider
  }

  async update(id: string, patch: ProviderPatch): Promise<ProviderDto> {
    return (await this.routes.json('PATCH', `/providers/${encodeURIComponent(id)}`, providerAnswerSchema, patch)).provider
  }

  async remove(id: string): Promise<void> {
    await this.routes.send('DELETE', `/providers/${encodeURIComponent(id)}`)
  }

  accounts(id: string): ProviderAccounts {
    const path = `/providers/${encodeURIComponent(id)}/accounts`
    return {
      list: () => this.routes.json('GET', path, accountsSchema),
      activate: async (credentialId) => void (await this.routes.json('PUT', `${path}/active`, activeAccountSchema, { credentialId })),
      order: (credentialIds) => this.routes.send('PUT', `${path}/order`, { credentialIds }),
      remove: async (credentialId) => void (await this.routes.json('DELETE', `${path}/${encodeURIComponent(credentialId)}`, removedAccountSchema)),
      signIn: async () => this.signInOf((await this.routes.json('POST', `${path}/login`, loginStartedSchema)).login.id),
    }
  }

  /** Signs in to a subscription, which makes its entry. */
  async signIn(login: SubscriptionLogin): Promise<SignIn> {
    return this.signInOf((await this.routes.json('POST', '/providers/subscription-login', loginStartedSchema, login)).login.id)
  }

  private signInOf(id: string): SignIn {
    const path = `/providers/subscription-login/${encodeURIComponent(id)}`
    return {
      id,
      submitCode: async (code) => void (await this.routes.json('POST', `${path}/code`, codeTakenSchema, { code })),
      cancel: () => this.routes.send('DELETE', path),
    }
  }
}

/** A Host of the core, which a runner serves over a channel the program gives it. */
export class Host {
  constructor(
    readonly device: DeviceDto,
    private readonly routes: CoreRoutes,
    private readonly process: Process,
    private readonly runner: string | null,
    private readonly dir: string,
  ) {}

  get id(): string {
    return this.device.id
  }

  /**
   * Carries the Host's channel on `stream`, a runner's standard output and
   * input, both ways; resolves once the channel opened, and `ended` once
   * either end closed.
   */
  async connect(stream: { readable: Readable; writable: Writable }): Promise<{ ended: Promise<void> }> {
    const channel = await this.process.connect(this.routes.path(`/devices/${encodeURIComponent(this.id)}/runner`), 'h2c')
    stream.readable.pipe(channel)
    channel.pipe(stream.writable)
    const ended = new Promise<void>((resolve) => {
      channel.once('close', resolve)
      stream.readable.once('close', () => {
        channel.close()
        resolve()
      })
    })
    return { ended }
  }

  /**
   * Starts `command`, a `demi-runner run --stdio` and whatever reaches it,
   * such as `docker exec -i box demi-runner run --stdio`, and connects its
   * standard output and input. With no command, the package's own runner
   * runs on the program's machine, its state in the core's directory.
   */
  async spawn(command?: readonly string[]): Promise<ChildProcess> {
    const [executable, ...args] = command ?? this.ownRunner()
    if (executable === undefined) {
      throw new Error('The command is empty')
    }
    const child = spawn(executable, args, { stdio: ['pipe', 'pipe', 'inherit'] })
    const { ended } = await this.connect({ readable: child.stdout!, writable: child.stdin! })
    // The runner ends with its channel.
    void ended.then(() => child.stdin?.end())
    return child
  }

  private ownRunner(): string[] {
    if (this.runner === null) {
      throw new Error('This platform has no runner of the package\'s; name a command')
    }
    return [this.runner, 'run', '--stdio', '--home', join(this.dir, 'hosts', this.id)]
  }
}

export class Hosts {
  constructor(
    private readonly routes: CoreRoutes,
    private readonly process: Process,
    private readonly runner: string | null,
    private readonly dir: string,
  ) {}

  /** Creates a Host, which serves the core once a runner connects to it. */
  async create(): Promise<Host> {
    const { device } = await this.routes.json('POST', '/devices', deviceAnswerSchema)
    return this.host(device)
  }

  /** The Host `device` is, as the product state lists it. */
  host(device: DeviceDto): Host {
    return new Host(device, this.routes, this.process, this.runner, this.dir)
  }
}
