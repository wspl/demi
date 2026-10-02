import {
  computed,
  defineComponent,
  effectScope,
  h,
  inject,
  onScopeDispose,
  provide,
  type Component,
  type ComputedRef,
  type InjectionKey,
  type PropType,
} from 'vue'
import type { z } from 'zod'
import type { PanelTabKind } from '../agent/panel-kinds/kind'
import type { HostInstall } from '../devices/installs'
import type { ChangeSetSource, ReadCallChange } from '../files/changes'
import type { FileBrowserSource } from '../files/types'
import { reportError } from '../infra/errors'
import type { OverlayStore } from '../overlay/overlayStore'
import type { SettingsNavGroup, SettingsNavItem } from '../settings/types'
import type { IntentName, IntentPayloads, IntentRequest } from './intents'
import type { OpenUserStream } from './streams'

/**
 * A plugin's page (`plugin-pages.md`): the object it declares with
 * `definePage`, and the page context every component of it reaches through
 * `usePage()`. The shell knows a page only through this object, and a page
 * knows the shell only through its context, which `web` supplies over the
 * backend and the gallery over each specimen's fixtures.
 */

/** A call the plugin, or the host in front of it, refused: its reason word and message. */
export class PluginCallError extends Error {
  constructor(
    /** The plugin's snake_case reason, or the host's code when the plugin did not answer. */
    readonly reason: string,
    message: string,
  ) {
    super(message)
  }
}

export interface PluginCallOptions {
  /** How long the call may take; the host's default otherwise. */
  timeoutMs?: number
  signal?: AbortSignal
}

/**
 * A conversation's files (`plugin-pages.md` § The conversation files
 * service): a product service, whichever page shows them.
 */
export interface ConversationFileService {
  /** The Host's file tree and file contents, while the conversation's Host is known. */
  readonly workspace: { source: FileBrowserSource; root: string; name?: string } | null
  /** Where the conversation's work runs, which a retained edit's paths are relative to. */
  readonly root: string | null
  /** The working tree's uncommitted changes. */
  readonly changes: ChangeSetSource
  /** The two sides of one call's retained edit. */
  readonly edit: ReadCallChange
  /**
   * The calling component shows the working tree: the service lists it again
   * whenever it may have changed, until the component's scope ends.
   */
  showChanges(): void
}

/** Opening intents (`plugin-pages.md` § Intents). */
export interface IntentService {
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
   * Calls `method` of `plugin` for the user, or for `conversation`, and
   * answers its JSON result; a refusal rejects with a `PluginCallError`.
   */
  call(
    plugin: string,
    method: string,
    params: object,
    conversation: string | null,
    options?: PluginCallOptions,
  ): Promise<unknown>
  /** The user stream `name` of `conversation`'s main Host. */
  stream(name: string, conversation: string): OpenUserStream
  /** The installs of `plugin`'s packages on `conversation`'s main Host, read reactively. */
  installs(plugin: string, conversation: string): readonly HostInstall[]
  files(conversation: string): ConversationFileService
  intents: IntentService
  /** The panel tabs of one conversation. */
  panel: {
    /** The `data` of the panel's tabs of `kind`, as saved. */
    tabs(conversation: string, kind: string): unknown[]
    /** Adds a tab of `kind`; `select` selects it and opens the panel. */
    add(conversation: string, kind: string, data: unknown, options?: { select: boolean }): void
  }
  /** Opens a section of the settings dialog, such as `devices`. */
  openSettings(section: string): void
  overlays: OverlayStore
}

/** A plugin's state, validated with the page's schema: null until it arrives. */
export interface PluginState<T> {
  readonly value: ComputedRef<T | null>
  /** Why the last read failed, until one succeeds. */
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
  /** The installs of the plugin's packages on the conversation's main Host, which a first call may wait for. */
  readonly installs: ComputedRef<readonly HostInstall[]>
}

/** The page's own plugin. */
export interface PagePlugin {
  /** The user state, from the product state. */
  state<T>(schema: z.ZodType<T>): ComputedRef<T | null>
  /** Calls a method of the user scope. */
  call<T>(method: string, params: object, result: z.ZodType<T>, options?: PluginCallOptions): Promise<T>
  conversation(id: string): ConversationPlugin
}

/** What every component of a page reaches (`plugin-pages.md` § The page context). */
export interface PageContext {
  readonly plugin: PagePlugin
  readonly intents: IntentService
  /** The panel tabs of the page's own kinds. */
  readonly panel: PageHost['panel']
  readonly settings: { open(section: string): void }
  readonly errors: {
    /** An error the user sees. */
    report(message: string, error: unknown): void
    /** A defect of the page itself, which only the console shows. */
    defect(message: string, error: unknown): void
  }
  readonly overlays: OverlayStore
  files(conversation: string): ConversationFileService
}

/** A kind's tab as the page's functions see it. */
export interface KindTab<Session> {
  conversation: string
  /** The page's panel session of the conversation. */
  session: Session
}

/**
 * A work panel kind, declared once on its page (`plugin-pages.md` § Work
 * panel kinds). Its functions are methods, so kinds of different data fit
 * one list.
 */
export interface PanelKind<Data, Session = undefined> {
  /** Its id, unique across plugins. */
  kind: string
  /** A tab's `data`, checked where saved state enters the page. */
  schema: z.ZodType<Data>
  title(data: Data, tab: KindTab<Session>): string
  /** The strip's mark. Props: `data`. */
  mark: Component
  /**
   * The tab's content. Props: `conversation`, `session`, `tabId`, `data`, and
   * `shown`, whether the tab is the panel's selection. Emits `update` with
   * the tab's next `data`, and `close` to have the panel close the tab as its
   * user would.
   */
  content: Component
  /** The kind is offered on the strip's new-tab control; `data` is a new tab's. */
  create?: { label: string; icon: Component; data(): Data }
  /**
   * The kind has one tab in every conversation's panel, ahead of the user's
   * tabs, which is never created, closed or saved; its data starts here and
   * lives in the page's memory. Its id is the kind's id.
   */
  pinned?: { data(): Data }
  /** What a tab shows next when its user picks it in the strip, even while it is selected. */
  picked?(data: Data): Data
  /** What the strip shows after a pinned tab's title, such as its counts. Props: `conversation`, `data`. */
  badge?: Component
  /** The intents it opens: for each, the data its tab shows next, from the payload and what it shows now. */
  intents?: {
    file?(payload: IntentPayloads['file'], current: Data | null): Data
    edit?(payload: IntentPayloads['edit'], current: Data | null): Data
  }
}

export interface PluginSettingsSection {
  /** The label of the rail's group it joins. */
  group: string
  item: SettingsNavItem
  component: Component
}

/** What a panel session may do when it ends, besides its effect scope stopping. */
export interface PanelSession {
  dispose?(): void
}

/** What a page contributes to the shell (`plugin-pages.md` § The page object). */
export interface PluginPage<Session extends PanelSession | undefined = undefined> {
  /** Its plugin's id, imported from its generated module. */
  plugin: string
  /** A section of the settings dialog. */
  settings?: PluginSettingsSection
  /** A component the conversation header shows. Props: `conversation`. */
  headerTool?: Component
  /** The kinds of tab it shows in the work panel. */
  kinds?: readonly PanelKind<unknown, Session>[]
  /**
   * What runs for a conversation while its panel is open with the plugin on
   * (`plugin-pages.md` § Panel sessions). It runs in its own effect scope,
   * which stops with it, and its `dispose`, if it has one, is called then.
   */
  panel?(conversation: string, page: PageContext): Session
}

/** A page, checked: its kinds' ids are its own once. */
export function definePage<Session extends PanelSession | undefined = undefined>(
  page: PluginPage<Session>,
): PluginPage<Session> {
  const ids = (page.kinds ?? []).map((kind) => kind.kind)
  const repeated = ids.find((id, index) => ids.indexOf(id) !== index)
  if (repeated !== undefined) {
    throw new Error(`the page of ${page.plugin} declares the kind ${repeated} twice`)
  }
  return page
}

/** Any page, as the registry lists them. */
export type AnyPluginPage = PluginPage<PanelSession | undefined>

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

/** Calls `method` of `plugin` over `host` and validates its result. */
async function validatedCall<T>(
  host: PageHost,
  plugin: string,
  method: string,
  params: object,
  result: z.ZodType<T>,
  conversation: string | null,
  options?: PluginCallOptions,
): Promise<T> {
  return result.parse(await host.call(plugin, method, params, conversation, options))
}

/** `plugin` for `conversation`, over `host`. */
function conversationPlugin(host: PageHost, plugin: string, conversation: string): ConversationPlugin {
  return {
    state<T>(schema: z.ZodType<T>): PluginState<T> {
      const feed = host.followState(plugin, conversation)
      onScopeDispose(() => feed.stop())
      return {
        value: computed(() => {
          const raw = feed.value()
          return raw === undefined ? null : schema.parse(raw)
        }),
        error: computed(() => feed.error()),
        read: () => feed.read(),
      }
    },
    call: (method, params, result, options) =>
      validatedCall(host, plugin, method, params, result, conversation, options),
    stream: (name) => host.stream(name, conversation),
    installs: computed(() => host.installs(plugin, conversation)),
  }
}

/** The context of `page` over `host`: its own plugin, and the panel tabs of its own kinds. */
export function pageContext(host: PageHost, page: AnyPluginPage): PageContext {
  const own = (kind: string) => {
    if (!(page.kinds ?? []).some((candidate) => candidate.kind === kind)) {
      throw new Error(`the page of ${page.plugin} has no kind ${kind}`)
    }
  }
  return {
    plugin: {
      state<T>(schema: z.ZodType<T>) {
        return computed(() => {
          const raw = host.userState(page.plugin)
          return raw === undefined ? null : schema.parse(raw)
        })
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
      add(conversation, kind, data, options) {
        own(kind)
        host.panel.add(conversation, kind, data, options)
      },
    },
    settings: { open: (section) => host.openSettings(section) },
    errors: {
      report: (message, error) => reportError(message, error, { userVisible: true }),
      defect: (message, error) => reportError(message, error),
    },
    overlays: host.overlays,
    files: (conversation) => host.files(conversation),
  }
}

/** Shows `component` of a page with the page's context below it. */
export const PageScope = defineComponent({
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

/** A page's kinds for one conversation's panel, and the session behind them. */
export interface BoundKinds {
  kinds: PanelTabKind[]
  dispose(): void
}

/**
 * The panel's kinds of `pages` for `conversation`, with each page's panel
 * session made, until `dispose`: each kind's functions and components bound
 * to the conversation, its session and its page's context.
 */
export function bindPages(pages: readonly AnyPluginPage[], host: PageHost, conversation: string): BoundKinds {
  const scope = effectScope(true)
  const bound = scope.run(() =>
    pages.map((page) => {
      const context = pageContext(host, page)
      const session = page.panel?.(conversation, context)
      const tab: KindTab<PanelSession | undefined> = { conversation, session }
      const kinds = (page.kinds ?? []).map((kind) => bindKind(kind, tab, context))
      return { kinds, session }
    }),
  )!
  return {
    kinds: bound.flatMap((page) => page.kinds),
    dispose() {
      for (const page of bound) {
        page.session?.dispose?.()
      }
      scope.stop()
    },
  }
}

/** `kind` as the panel shows it for one conversation. */
function bindKind(
  kind: PanelKind<unknown, PanelSession | undefined>,
  tab: KindTab<PanelSession | undefined>,
  context: PageContext,
): PanelTabKind {
  const content = defineComponent({
    props: {
      tabId: { type: String, required: true },
      data: { required: true },
      shown: { type: Boolean, required: true },
    },
    emits: { update: (_data: unknown) => true, close: () => true },
    setup(props, { emit }) {
      providePage(context)
      return () =>
        h(kind.content, {
          conversation: tab.conversation,
          session: tab.session,
          tabId: props.tabId,
          data: props.data,
          shown: props.shown,
          onUpdate: (data: unknown) => emit('update', data),
          onClose: () => emit('close'),
        })
    },
  })
  const { badge, picked } = kind
  return {
    kind: kind.kind,
    schema: kind.schema,
    title: (data) => kind.title(data, tab),
    mark: kind.mark,
    content,
    create: kind.create,
    pinned: kind.pinned,
    picked: picked && ((data) => picked(data)),
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
): PanelKind<unknown, PanelSession | undefined> | null {
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
    const group = joined.find((candidate) => candidate.label === page.settings!.group)
    if (group) {
      group.items.push(page.settings.item)
    } else {
      joined.push({ label: page.settings.group, items: [page.settings.item] })
    }
  }
  return joined
}

/** The page whose settings section is `tab`, if a plugin fills it. */
export function settingsPage(pages: readonly AnyPluginPage[], tab: string): AnyPluginPage | null {
  return pages.find((page) => page.settings?.item.id === tab) ?? null
}
