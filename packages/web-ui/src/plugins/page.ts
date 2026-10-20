import { t } from './locales/text'
import {
  computed,
  defineComponent,
  h,
  inject,
  onScopeDispose,
  provide,
  shallowRef,
  watch,
  type Component,
  type ComputedRef,
  type InjectionKey,
  type PropType,
} from 'vue'
import { schemaRefusal } from '@demicodes/protocol'
import type { Pinia } from 'pinia'
import { z } from 'zod'
import type { PanelTabKind, TabCommand } from '../agent/panel-kinds/kind'
import type { HostArtifact } from '../devices/installed'
import type { ChangeSetSource, ReadCallChange } from '../files/changes'
import type { CommitHistorySource } from '../files/history'
import type { TranscriptRequest } from '../files/request-changes'
import type { FileBrowserSource } from '../files/types'
import { provideOnScreen } from '../files/on-screen'
import { reportError } from '../infra/errors'
import { showToast } from '../infra/toast'
import type { OverlayStore } from '../overlay/overlayStore'
import type { SettingsGroupId, SettingsNavGroup, SettingsNavItem } from '../settings/types'
import type { SidebarEntry } from '../sidebar/types'
import type { IntentName, IntentPayloads, IntentRequest } from './intents'
import { pluginCache, type PluginCache } from './cache'
import { pluginStore, type PluginStore } from './stores'
import type { OpenUserStream } from './streams'
import type { Operation } from '../ui/operations'
import type { HeadlineText, SentenceText } from '../ui/ui-text'

/**
 * A plugin's page: the object it declares with `definePage`, and the page
 * context every component of it reaches through `usePage()`.
 *
 * The shell knows a page only through this object, and a page knows the shell
 * only through its context, which `web` supplies over the backend and the gallery
 * over each specimen's fixtures. A component never names a plugin and cannot
 * reach another plugin's state or calls.
 *
 * Rules of a page:
 *
 * - Everything a page shows is pushed to it, or read when a pushed revision says
 *   it changed: no page polls, and none guesses when to read again.
 * - A state that does not read is reported, never thrown. A component that
 *   throws while it renders stops updating. In development Vue keeps the DOM of
 *   its last render, so a switch that showed its call pending stays pending; in
 *   a production build the component shows nothing. So the context keeps the
 *   last state that read and says why the latest did not. An answer that does
 *   not read fails its call.
 * - A control waits for its own call, and only for it, never for the state the
 *   call causes, and shows the plugin's state when the call succeeds. A call is
 *   an operation of the page, which goes on when the component that showed the
 *   control goes. A control that only sends input to what the user watches, such
 *   as a simulator's Home, waits for nothing: the picture shows the effect.
 * - Work panel kinds are data on the page, so the panel knows every kind of
 *   every page the user has on before it opens, and an intent can open a kind
 *   while the panel is closed.
 * - Intents are the only way a page opens a tab. A control that opens one asks
 *   first whether any page does, and shows plain text otherwise.
 * - What follows from a tab being created or closed, such as the browser closing
 *   its own tab, is the plugin's work on the backend, never the page's.
 * - What a page keeps live, such as a device's connection and last picture,
 *   lives in its stores (`./stores.ts`), from the plugin's first use while the
 *   user has it on until it is turned off; a component that shows it only
 *   follows it. The panel binds only the pages' kinds and holds no session,
 *   each page's apart, so turning one plugin on or off touches no other's.
 *
 * `web-ui` knows no plugin, no plugin's kind and no plugin's types, imports no
 * Node module, and ships no control-plane transport of its own: it consumes an
 * injected `ConversationClient` and injected plugin services.
 */

/**
 * What came back for a call that failed, which a page says for a reason it has no
 * words for. It is one of:
 *
 * - the backend's answer, by its status line such as `500 Internal Server Error`,
 * - none, as for a network failure or a timeout, or
 * - an answer the page could not read.
 */
export type PluginCallAnswer =
  | { type: 'status'; status: string }
  | { type: 'none' }
  | { type: 'unreadable' }

/**
 * A call the plugin, or the host in front of it, refused: its reason word
 * and message, with the request it came from; or an answer the page could
 * not read, with what of it did not read.
 */
export class PluginCallError extends Error {
  /** The request the refusal answered, which the page shows with it. */
  readonly requestId: string | null
  /** What of an answer did not read, which the page's report carries (`schemaRefusal`). */
  readonly detail: string | undefined

  constructor(
    /** The plugin's snake_case reason, or the host's code when the plugin did not answer. */
    readonly reason: string,
    message: SentenceText,
    readonly answer: PluginCallAnswer = { type: 'none' },
    from: { requestId?: string | null; detail?: string } = {},
  ) {
    super(message)
    this.requestId = from.requestId ?? null
    this.detail = from.detail
  }
}

export interface PluginCallOptions {
  /** How long the call may take; the host's default otherwise. */
  timeoutMs?: number
  signal?: AbortSignal
}

/**
 * A conversation's files: a product service, whichever page shows them.
 */
export interface ConversationFileService {
  /** The Host's file tree and file contents, while the conversation's Host is known. */
  readonly workspace: { source: FileBrowserSource; root: string; name?: string } | null
  /** Where the conversation's work runs, which a retained edit's paths are relative to. */
  readonly root: string | null
  /**
   * The uncommitted changes of the working tree under `directory`, on the
   * conversation's Host; the conversation's own without one.
   */
  changes(directory?: string): ChangeSetSource
  /**
   * The history of the repository the working tree under `directory` is
   * in, the conversation's without one (`packages/web-ui/src/files/history.ts`);
   * null while the conversation's Host is not known.
   */
  history(directory?: string): CommitHistorySource | null
  /** The two sides of one retained edit, by the blobs that hold them. */
  readonly edit: ReadCallChange
  /**
   * A request of the agent `node` (null for the conversation's own), with the files
   * its calls changed, as the agent's transcript holds it now. It is derived as it
   * is read, never stored (`crates/agent-store/src/changes.rs`).
   *
   * Null while the transcript does not hold it.
   */
  request(node: string | null, request: string): TranscriptRequest | null
  /**
   * The calling component shows the working tree under the directory `directory()`
   * names, read at each change, the conversation's when it names none.
   *
   * The service follows the Host's reports for it and keeps `changes` of it current
   * until the component's scope ends.
   */
  showChanges(directory?: () => string | undefined): void
}

/** Opening intents. */
export interface IntentService {
  /** Opens `request` in the conversation's panel; what the panel already shows stays as it is (`openIntent`). */
  open(conversation: string, request: IntentRequest): void
  /** Whether any page the user has on opens `intent`; a control that would open it shows only then. */
  canOpen(intent: IntentName): boolean
}

/** One plugin's state of one conversation, as the host follows it. */
export interface StateFeed {
  /** The state last read, read reactively; undefined before the first answer. */
  value(): unknown
  /** Why the last read failed, until one succeeds. */
  error(): PluginCallError | null
  /** Reads the state again now, as a Retry does. */
  read(): void
  stop(): void
}

/** What the page around the shell supplies for every page. */
export interface PageHost {
  /** The plugin's user state as the product state holds it, read reactively; undefined while there is none. */
  userState(plugin: string): unknown
  /** Follows the plugin's state of `conversation`, by its revision, until `stop`. */
  followState(plugin: string, conversation: string): StateFeed
  /**
   * Calls `method` of `plugin` for the user, or for `conversation`, and answers its
   * JSON result. A refusal rejects with a `PluginCallError`.
   *
   * - A call that cannot reach the backend, as while it restarts, waits until it can
   *   and goes then, so it never rejects for the connection.
   * - One whose `signal` aborts meanwhile rejects with the abort and is never sent.
   */
  call(
    plugin: string,
    method: string,
    params: object,
    conversation: string | null,
    options?: PluginCallOptions,
  ): Promise<unknown>
  /** The user stream `name` of `conversation`'s primary Host. */
  stream(name: string, conversation: string): OpenUserStream
  /** What `conversation`'s primary Host holds of `plugin`'s packages, read reactively. */
  installed(plugin: string, conversation: string): readonly HostArtifact[]
  /** Whether `conversation`'s primary Host is the user's Cloud while it does not run, read reactively. */
  hostStarting(conversation: string): boolean
  files(conversation: string): ConversationFileService
  intents: IntentService
  /** The panel tabs of one conversation. */
  panel: {
    /** The `data` of the panel's tabs of `kind`, as saved. */
    tabs(conversation: string, kind: string): unknown[]
    /**
     * Selects the tab `id` of `kind` and opens the panel, as a browser
     * selects the tab a click opened; a tab the panel does not have yet is
     * selected once it arrives.
     */
    select(conversation: string, kind: string, id: string): void
  }
  /** Opens a section of the settings dialog, such as `devices`. */
  openSettings(section: string): void
  /** The shell's Pinia, which holds the plugins' stores. */
  pinia: Pinia
  /**
   * Opens a web page in a new tab, such as a vendor's sign-in.
   *
   * Call it in the click's handler with the URL or a promise of the URL a call will
   * answer. The tab opens at once, so no popup blocker stops it, and goes to the URL
   * when it arrives, or closes when the call fails, whose failure the returned
   * promise carries.
   */
  openLink(url: string | Promise<string>): Promise<void>
  overlays: OverlayStore
}

/** A plugin's state, validated with the page's schema: null until it arrives. */
export interface PluginState<T> {
  /** The last state that read with the page's schema. */
  readonly value: ComputedRef<T | null>
  /** Why the last read failed, or the last state did not read, until one succeeds. */
  readonly error: ComputedRef<PluginCallError | null>
  /** Reads the state again now. */
  read(): void
}

/** The page's plugin for one conversation. */
export interface ConversationPlugin {
  /** The conversation state, followed while the calling scope lives. */
  state<T>(schema: z.ZodType<T>): PluginState<T>
  /** Calls a method of the conversation scope. */
  call<T>(method: string, params: object, result: z.ZodType<T>, options?: PluginCallOptions): Promise<T>
  stream(name: string): OpenUserStream
  /** What the conversation's primary Host holds of the plugin's packages, kept while it is offline. */
  readonly installed: ComputedRef<readonly HostArtifact[]>
  /**
   * Whether the conversation's primary Host is the user's Cloud while it
   * does not run: work the user starts there waits for it to start first.
   */
  readonly hostStarting: ComputedRef<boolean>
}

/** The page's own plugin. */
export interface PagePlugin {
  /**
   * The user state, from the product state: the last one that read with
   * the page's schema. One that does not read is reported to the user.
   */
  state<T>(schema: z.ZodType<T>): ComputedRef<T | null>
  /** Calls a method of the user scope. */
  call<T>(method: string, params: object, result: z.ZodType<T>, options?: PluginCallOptions): Promise<T>
  conversation(id: string): ConversationPlugin
}

/** What every component of a page reaches. */
export interface PageContext {
  readonly plugin: PagePlugin
  readonly intents: IntentService
  /** The panel tabs of the page's own kinds. */
  readonly panel: PageHost['panel']
  readonly settings: { open(section: string): void }
  /** Opens a web page in a new tab, as {@link PageHost.openLink} does. */
  readonly links: { open(url: string | Promise<string>): Promise<void> }
  readonly errors: {
    /** An error the user sees; a call its caller dropped is none. */
    report(message: HeadlineText, error: unknown): void
    /** A defect of the page itself, which only the console shows; a call its caller dropped is none. */
    defect(message: string, error: unknown): void
  }
  readonly overlays: OverlayStore
  files(conversation: string): ConversationFileService
  /** The plugin's store `store`, made at its first use (`./stores.ts`). */
  store<T extends object>(store: PluginStore<T>): T
  /** The plugin's disk cache (`./cache.ts`). */
  readonly cache: PluginCache
  /**
   * The page's operation of the plugin's `action` on what `key` names, such as
   * `operation('set_source_enabled', source)` for a skill source's switch.
   *
   * A control given it shows the call working whenever it is drawn, its component
   * gone and drawn again included, and a press while it runs starts nothing.
   */
  operation(action: string, key: string): Operation
}

/** A kind's tab as the page's functions see it. */
export interface KindTab {
  conversation: string
  /** The page's context, whose stores hold what the page keeps live (`./stores.ts`). */
  page: PageContext
}

/**
 * A work panel kind, declared once on its page. Its functions are methods, so kinds of different
 * data fit one list.
 */
export interface PanelKind<Data> {
  /** Its id, unique across plugins. */
  kind: string
  /** A tab's `data`, checked where saved state enters the page. */
  schema: z.ZodType<Data>
  title(data: Data, tab: KindTab): string
  /**
   * What makes a tab of a kind that is not pinned the one it is, such as an
   * attachment's number: an intent whose new data has the key of a tab the strip
   * holds selects that tab instead of adding another.
   *
   * Without it, each intent adds a tab, as a browser opens each link in a tab of its
   * own.
   */
  key?(data: Data): string
  /** The strip's mark. Props: `data`. */
  mark: Component
  /**
   * A picture the strip shows in place of the mark, such as a web page's
   * icon, as an image URL; null while there is none. Read reactively.
   */
  icon?(data: Data, tab: KindTab, id: string): string | null
  /**
   * Whether the tab `id` is at work, such as a page that loads, read
   * reactively: the strip shows a spinner in place of its mark meanwhile.
   */
  busy?(data: Data, tab: KindTab, id: string): boolean
  /**
   * The tab's content.
   *
   * - Props: `conversation`, `tabId`, `data`, and `shown`, whether the tab is the
   *   panel's selection. What it keeps live beyond its mount is in its page's
   *   stores.
   * - Emits `update` with the tab's next `data`, and `close` to have the panel close
   *   the tab as its user would.
   */
  content: Component
  /**
   * The kind has one tab in every conversation's panel, ahead of the user's
   * tabs, which is never created, closed or saved; its data starts here and
   * lives in the page's memory. Its id is the kind's id.
   */
  pinned?: { data(): Data }
  /**
   * What the tab's menu offers before its Close commands, such as a
   * browser tab's Reload, read when the menu opens.
   */
  commands?(data: Data, tab: KindTab, id: string): readonly TabCommand[]
  /**
   * How many times something asked that the user see the tab, from its
   * data. When the count rises above the one the page last applied for the
   * tab, the panel opens and selects it, open or closed before.
   */
  shows?(data: Data): number
  /** What the strip shows after a pinned tab's title, such as its counts. Props: `conversation`, `data`. */
  badge?: Component
  /** The intents it opens: for each, the data its tab shows next, from the payload and what it shows now. */
  intents?: {
    file?(payload: IntentPayloads['file'], current: Data | null): Data
    edit?(payload: IntentPayloads['edit'], current: Data | null): Data
    commit?(payload: IntentPayloads['commit'], current: Data | null): Data
    attachment?(payload: IntentPayloads['attachment'], current: Data | null): Data
    media?(payload: IntentPayloads['media'], current: Data | null): Data
    web?(payload: IntentPayloads['web'], current: Data | null): Data
  }
}

export interface PluginSettingsSection {
  /** The rail's group it joins. */
  group: SettingsGroupId
  item: SettingsNavItem
  component: Component
  /**
   * The section also has an entry in the sidebar's top group, with its
   * item's icon and label, that opens it.
   */
  sidebar?: true
}

/** What a page contributes to the shell. */
export interface PluginPage {
  /** Its plugin's id, imported from its generated module. */
  plugin: string
  /** A section of the settings dialog. */
  settings?: PluginSettingsSection
  /** The kinds of tab it shows in the work panel. */
  kinds?: readonly PanelKind<unknown>[]
}

/** A page, checked: its kinds' ids are its own once. */
export function definePage(page: PluginPage): PluginPage {
  const ids = (page.kinds ?? []).map((kind) => kind.kind)
  const repeated = ids.find((id, index) => ids.indexOf(id) !== index)
  if (repeated !== undefined) {
    throw new Error(`the page of ${page.plugin} declares the kind ${repeated} twice`)
  }
  return page
}

/** Any page, as the registry lists them. */
export type AnyPluginPage = PluginPage

const PAGE_HOST: InjectionKey<PageHost> = Symbol('page-host')
const PAGE_CONTEXT: InjectionKey<PageContext> = Symbol('page-context')

/** Gives the components below the host every page's context is made over. */
export function providePageHost(host: PageHost): void {
  provide(PAGE_HOST, host)
}

/** The host the page around the component supplied. */
export function usePageHost(): PageHost {
  const host = inject(PAGE_HOST, null)
  if (!host) {
    throw new Error('A plugin page is shown without a page host')
  }
  return host
}

/** Whether `error` is the abort of a call whose caller went away. */
function dropped(error: unknown): boolean {
  return error instanceof DOMException && error.name === 'AbortError'
}

/** The context of the page whose component calls it. */
export function usePage(): PageContext {
  const context = inject(PAGE_CONTEXT, null)
  if (!context) {
    throw new Error('usePage() is called outside a plugin page')
  }
  return context
}

/** Gives the components below the page's context. */
export function providePage(context: PageContext): void {
  provide(PAGE_CONTEXT, context)
}

/** The failure of a value that does not read with its schema: `reason`, and what did not read. */
function unreadable(reason: string, error: z.ZodError): PluginCallError {
  const refusal = schemaRefusal(error)
  return new PluginCallError(reason, refusal, { type: 'unreadable' }, { detail: refusal })
}

/**
 * `raw` read with `schema` while the calling scope lives: the last state that read,
 * null while there is none, and why the latest did not read, until one does.
 *
 * A state that does not read never reaches the page's components, and the last one
 * that read stays shown.
 */
function readState<T>(raw: () => unknown, schema: z.ZodType<T>) {
  const value = shallowRef<T | null>(null)
  const refusal = shallowRef<PluginCallError | null>(null)
  watch(
    raw,
    (next) => {
      if (next === undefined) {
        value.value = null
        refusal.value = null
        return
      }
      const read = schema.safeParse(next)
      if (read.success) {
        value.value = read.data
        refusal.value = null
        return
      }
      refusal.value = unreadable('unreadable_state', read.error)
    },
    { immediate: true, flush: 'sync' },
  )
  return { value, refusal }
}

/**
 * Calls `method` of `plugin` over `host` and validates its result: an
 * answer the page cannot read fails the call, as a refusal does.
 */
async function validatedCall<T>(
  host: PageHost,
  plugin: string,
  method: string,
  params: object,
  result: z.ZodType<T>,
  conversation: string | null,
  options?: PluginCallOptions,
): Promise<T> {
  const read = result.safeParse(await host.call(plugin, method, params, conversation, options))
  if (!read.success) {
    throw unreadable('unreadable_answer', read.error)
  }
  return read.data
}

/** `plugin` for `conversation`, over `host`. */
function conversationPlugin(host: PageHost, plugin: string, conversation: string): ConversationPlugin {
  return {
    state<T>(schema: z.ZodType<T>): PluginState<T> {
      const feed = host.followState(plugin, conversation)
      onScopeDispose(() => feed.stop())
      const state = readState(() => feed.value(), schema)
      return {
        value: computed(() => state.value.value),
        error: computed(() => feed.error() ?? state.refusal.value),
        read: () => feed.read(),
      }
    },
    call: (method, params, result, options) =>
      validatedCall(host, plugin, method, params, result, conversation, options),
    stream: (name) => host.stream(name, conversation),
    installed: computed(() => host.installed(plugin, conversation)),
    hostStarting: computed(() => host.hostStarting(conversation)),
  }
}

/**
 * The readers of one plugin's user state over one host that cannot read it now.
 *
 * The refusal is one fact however many components read the state. It is told once,
 * when the first reader stops reading, and told again only after every reader read
 * again or went.
 */
class UserStateRefusals {
  private refusing = 0

  constructor(private readonly plugin: string) {}

  begin(refusal: PluginCallError | null): void {
    this.refusing += 1
    if (this.refusing === 1) {
      // What did not read is for a developer, in the console.
      reportError(t('dataUnreadable'), refusal, {
        userVisible: true,
        message: t('reloadMayHelp'),
        detail: `the user state of the ${this.plugin} plugin`,
      })
    }
  }

  end(): void {
    this.refusing -= 1
  }
}

const refusalsByHost = new WeakMap<PageHost, Map<string, UserStateRefusals>>()

/** The refusals of `plugin`'s user state over `host`, which all its readers share. */
function userStateRefusals(host: PageHost, plugin: string): UserStateRefusals {
  let byPlugin = refusalsByHost.get(host)
  if (!byPlugin) {
    byPlugin = new Map()
    refusalsByHost.set(host, byPlugin)
  }
  let refusals = byPlugin.get(plugin)
  if (!refusals) {
    refusals = new UserStateRefusals(plugin)
    byPlugin.set(plugin, refusals)
  }
  return refusals
}

/**
 * Each page host's number, which keeps its plugins' operations its own: the
 * product has one host, the gallery one for each specimen, which is a page
 * of its own.
 */
const hostNumbers = new WeakMap<PageHost, number>()
let hostsNumbered = 0

function hostNumber(host: PageHost): number {
  let number = hostNumbers.get(host)
  if (number === undefined) {
    number = ++hostsNumbered
    hostNumbers.set(host, number)
  }
  return number
}

/** The context of `page` over `host`: its own plugin, and the panel tabs of its own kinds. */
export function pageContext(host: PageHost, page: AnyPluginPage): PageContext {
  const own = (kind: string) => {
    if (!(page.kinds ?? []).some((candidate) => candidate.kind === kind)) {
      throw new Error(`the page of ${page.plugin} has no kind ${kind}`)
    }
  }
  // A call its caller dropped, as a panel that closed, failed nothing anyone
  // waits for: neither a toast nor the console says anything of it.
  const errors: PageContext['errors'] = {
    report: (message, error) => {
      if (!dropped(error)) {
        reportError(message, error, { userVisible: true })
      }
    },
    defect: (message, error) => {
      if (!dropped(error)) {
        reportError(message, error)
      }
    },
  }
  const context: PageContext = {
    plugin: {
      state<T>(schema: z.ZodType<T>) {
        const state = readState(() => host.userState(page.plugin), schema)
        // The user state has no place of its own on a page, so the page
        // says once, in a toast, that it stopped reading it, however many
        // components read it (`userStateRefusals`).
        const refusals = userStateRefusals(host, page.plugin)
        let counted = false
        const settle = (refused: boolean) => {
          if (refused === counted) {
            return
          }
          counted = refused
          if (refused) {
            refusals.begin(state.refusal.value)
          } else {
            refusals.end()
          }
        }
        watch(() => state.refusal.value !== null, settle, { immediate: true, flush: 'sync' })
        onScopeDispose(() => settle(false))
        return computed(() => state.value.value)
      },
      call: (method, params, result, options) =>
        validatedCall(host, page.plugin, method, params, result, null, options),
      conversation: (id) => conversationPlugin(host, page.plugin, id),
    },
    intents: host.intents,
    panel: {
      tabs(conversation, kind) {
        own(kind)
        return host.panel.tabs(conversation, kind)
      },
      select(conversation, kind, id) {
        own(kind)
        host.panel.select(conversation, kind, id)
      },
    },
    settings: { open: (section) => host.openSettings(section) },
    links: { open: (url) => host.openLink(url) },
    errors,
    overlays: host.overlays,
    files: (conversation) => host.files(conversation),
    store: (store) => pluginStore(host.pinia, hostNumber(host), page.plugin, store, context),
    cache: pluginCache(page.plugin),
    operation: (action, key) => ({ action, target: `plugin:${hostNumber(host)}:${page.plugin}:${key}` }),
  }
  return context
}

/** The component of a page with the page's context below it, made once for the page it was given. */
const PageScopeBody = defineComponent({
  props: {
    page: { type: Object as PropType<AnyPluginPage>, required: true },
    component: { type: [Object, Function] as PropType<Component>, required: true },
    props: { type: Object as PropType<Record<string, unknown>>, default: () => ({}) },
  },
  setup(props) {
    providePage(pageContext(usePageHost(), props.page))
    return () => h(props.component, props.props)
  },
})

/**
 * Shows `component` of a page with the page's context below it. A scope
 * shown for another page is a new one, so a component never reads the
 * context, or keeps the effects, of the page that was shown before it.
 */
export const PageScope = defineComponent({
  props: {
    page: { type: Object as PropType<AnyPluginPage>, required: true },
    component: { type: [Object, Function] as PropType<Component>, required: true },
    props: { type: Object as PropType<Record<string, unknown>>, default: () => ({}) },
  },
  setup(props) {
    return () => h(PageScopeBody, { key: props.page.plugin, ...props })
  },
})

/**
 * The panel's kinds of `page` for `conversation`: each kind's functions and
 * components bound to the conversation and the page's context.
 */
export function bindPage(page: AnyPluginPage, host: PageHost, conversation: string): PanelTabKind[] {
  const context = pageContext(host, page)
  const tab: KindTab = { conversation, page: context }
  return (page.kinds ?? []).map((kind) => bindKind(kind, tab, context))
}

/**
 * The panel's kinds of the pages `pages()` names now, in their order, for
 * `conversation()`. Each page is bound apart (`bindPage`): a page that joins is
 * bound, and the others keep the kinds they had, so turning one plugin on or
 * off remounts no other's tab; a change of conversation binds every page anew.
 */
export function useBoundKinds(
  pages: () => readonly AnyPluginPage[],
  host: PageHost,
  conversation: () => string | null,
): ComputedRef<PanelTabKind[]> {
  const bound = shallowRef(new Map<string, PanelTabKind[]>())
  let boundFor: string | null = null
  watch(
    [() => pages().map((page) => page.plugin).join(), conversation],
    () => {
      const now = conversation()
      const kept = now === boundFor ? bound.value : new Map<string, PanelTabKind[]>()
      const next = new Map<string, PanelTabKind[]>()
      if (now !== null) {
        for (const page of pages()) {
          next.set(page.plugin, kept.get(page.plugin) ?? bindPage(page, host, now))
        }
      }
      boundFor = now
      bound.value = next
    },
    { immediate: true },
  )
  return computed(() => [...bound.value.values()].flat())
}

/** `kind` as the panel shows it for one conversation. */
function bindKind(kind: PanelKind<unknown>, tab: KindTab, context: PageContext): PanelTabKind {
  const content = defineComponent({
    props: {
      tabId: { type: String, required: true },
      data: { required: true },
      shown: { type: Boolean, required: true },
    },
    emits: { update: (_data: unknown) => true, close: () => true },
    setup(props, { emit }) {
      providePage(context)
      // A tab that is not the selected one keeps its views but follows nothing.
      provideOnScreen(() => props.shown)
      return () =>
        h(kind.content, {
          conversation: tab.conversation,
          tabId: props.tabId,
          data: props.data,
          shown: props.shown,
          onUpdate: (data: unknown) => emit('update', data),
          onClose: () => emit('close'),
        })
    },
  })
  const { badge, icon, busy, commands } = kind
  return {
    kind: kind.kind,
    schema: kind.schema,
    title: (data) => kind.title(data, tab),
    mark: kind.mark,
    icon: icon && ((data, id) => icon(data, tab, id)),
    busy: busy && ((data, id) => busy(data, tab, id)),
    content,
    pinned: kind.pinned,
    commands: commands && ((data, id) => commands(data, tab, id)),
    badge: badge
      ? defineComponent({
          props: { data: { required: true } },
          setup(props) {
            providePage(context)
            return () => h(badge, { conversation: tab.conversation, data: props.data })
          },
        })
      : undefined,
  }
}

/** The kind of the first page the user has on that opens `intent`, or null. */
export function intentKind(
  pages: readonly AnyPluginPage[],
  enabled: (plugin: string) => boolean,
  intent: IntentName,
): PanelKind<unknown> | null {
  for (const page of pages) {
    if (!enabled(page.plugin)) {
      continue
    }
    const kind = (page.kinds ?? []).find((candidate) => candidate.intents?.[intent])
    if (kind) {
      return kind
    }
  }
  return null
}

/**
 * The settings rail with each enabled page's section joined to its group,
 * at the group's end; a group the rail lacks is added at the end.
 */
export function withPluginSections(
  groups: readonly SettingsNavGroup[],
  pages: readonly AnyPluginPage[],
  enabled: (plugin: string) => boolean,
): SettingsNavGroup[] {
  const joined = groups.map((group) => ({ ...group, items: [...group.items] }))
  for (const page of pages) {
    if (!page.settings || !enabled(page.plugin)) {
      continue
    }
    const group = joined.find((candidate) => candidate.id === page.settings!.group)
    if (group) {
      group.items.push(page.settings.item)
    } else {
      joined.push({ id: page.settings.group, items: [page.settings.item] })
    }
  }
  return joined
}

/** The sidebar entries of the enabled pages whose settings section asks for one, in page order. */
export function sidebarEntries(
  pages: readonly AnyPluginPage[],
  enabled: (plugin: string) => boolean,
): SidebarEntry[] {
  const entries: SidebarEntry[] = []
  for (const page of pages) {
    if (!page.settings?.sidebar || !enabled(page.plugin)) {
      continue
    }
    const { id, label, icon } = page.settings.item
    entries.push({ section: id, label, icon })
  }
  return entries
}

/** The page whose settings section is `tab`, if a plugin fills it. */
export function settingsPage(pages: readonly AnyPluginPage[], tab: string): AnyPluginPage | null {
  return pages.find((page) => page.settings?.item.id === tab) ?? null
}
