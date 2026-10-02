import { computed, inject, provide, type ComputedRef, type InjectionKey } from 'vue'
import type { z } from 'zod'
import type { OpenUserStream } from './streams'
import type { HostInstall } from '../devices/installs'

/**
 * How a plugin's page reaches its plugin (`plugins.md` § The page): the
 * page around the slots supplies a `PluginHost`, `web` over HTTP and the
 * sync channel, the gallery over a specimen's state; a plugin's components
 * see only the `PluginClient` that `usePlugin()` gives them, which knows no
 * route and validates every state and result with the plugin's schemas.
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

/** What the page supplies for every plugin. */
export interface PluginHost {
  /** The plugin's state as the product state holds it, read reactively; undefined while there is none. */
  state(plugin: string): unknown
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
}

/** A plugin's calls for one conversation. */
export interface ConversationPluginClient {
  call<T>(method: string, params: object, result: z.ZodType<T>, options?: PluginCallOptions): Promise<T>
  stream(name: string): OpenUserStream
  /** The installs of the plugin's packages on the conversation's main Host, which a first call may wait for. */
  readonly installs: ComputedRef<readonly HostInstall[]>
}

/** One plugin, as its page's components reach it. */
export interface PluginClient<State> {
  /** The plugin's state; null while none arrived or the user has the plugin off. */
  readonly state: ComputedRef<State | null>
  /** Calls a method of the user scope. */
  call<T>(method: string, params: object, result: z.ZodType<T>, options?: PluginCallOptions): Promise<T>
  /** The plugin for `conversation`: its methods of the conversation scope and its user streams. */
  conversation(id: string): ConversationPluginClient
}

const PLUGIN_HOST: InjectionKey<PluginHost> = Symbol('plugin-host')

/** Gives the components below the host every plugin slot reaches. */
export function providePluginHost(host: PluginHost): void {
  provide(PLUGIN_HOST, host)
}

/** The host the page around the component supplied. */
export function usePluginHost(): PluginHost {
  const host = inject(PLUGIN_HOST, null)
  if (!host) {
    throw new Error('A plugin slot is shown without a plugin host')
  }
  return host
}

/** Calls `method` of `plugin` over `host` and validates its result. */
async function validatedCall<T>(
  host: PluginHost,
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
export function conversationClient(
  host: PluginHost,
  plugin: string,
  conversation: string,
): ConversationPluginClient {
  return {
    call: (method, params, result, options) =>
      validatedCall(host, plugin, method, params, result, conversation, options),
    stream: (name) => host.stream(name, conversation),
    installs: computed(() => host.installs(plugin, conversation)),
  }
}

/** The client of `plugin`, whose state `state` validates, over `host`. */
export function pluginClient<State>(
  host: PluginHost,
  plugin: string,
  state: z.ZodType<State>,
): PluginClient<State> {
  return {
    state: computed(() => {
      const raw = host.state(plugin)
      return raw === undefined ? null : state.parse(raw)
    }),
    call: (method, params, result, options) =>
      validatedCall(host, plugin, method, params, result, null, options),
    conversation: (id) => conversationClient(host, plugin, id),
  }
}

/** The client of `plugin` for a component of its page. */
export function usePlugin<State>(plugin: string, state: z.ZodType<State>): PluginClient<State> {
  return pluginClient(usePluginHost(), plugin, state)
}
