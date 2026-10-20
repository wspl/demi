/**
 * A plugin's stores (`docs/decisions/0008-plugin-architecture.md`): what a page
 * keeps live for as long as the user has its plugin on, apart from any panel
 * or conversation the page shows, as Pinia stores the shell holds.
 *
 * - A page declares a store with `definePluginStore` and reaches it through
 *   its page context's `store`, which names it under the page's own plugin and
 *   page host. So a plugin reaches only its own stores, and a gallery
 *   specimen, a page host of its own, none of another specimen's.
 * - A store is made at its first use and lives until the shell disposes its
 *   plugin's stores, as it does when the user turns the plugin off. Its effect
 *   scope stops then, with every watcher its setup started and every cleanup
 *   it registered with `onScopeDispose`.
 *
 * What a store holds is the page's own object, kept raw: Pinia owns the
 * store's life and its effect scope, and the object its own reactivity.
 */
import { defineStore, type Pinia } from 'pinia'
import { markRaw } from 'vue'
import type { PageContext } from './page'

/** A store a page declares: its name, unique within its plugin, and what it holds, made from the page's context. */
export interface PluginStore<T extends object> {
  readonly name: string
  readonly setup: (page: PageContext) => T
  /** What it holds now under each id it has (`pluginStore`). */
  readonly made: Map<string, T>
}

/** Declares a store of the page's plugin, which its page context's `store` reaches. */
export function definePluginStore<T extends object>(name: string, setup: (page: PageContext) => T): PluginStore<T> {
  return { name, setup, made: new Map() }
}

/** The plugin of each store made in a Pinia, by its id, and what disposes it. */
const disposers = new WeakMap<Pinia, Map<string, { plugin: string; dispose(): void }>>()

/**
 * The store `store` of `plugin` in `pinia`, under `host`, the page host's
 * number, made from `page` at its first use.
 *
 * Pinia runs the setup in the store's effect scope, which its `$dispose`
 * stops. What the setup makes is kept on the definition rather than as the
 * store's state, since Pinia's types derive a store's members from its setup
 * and cannot carry a page's own type through a generic one.
 */
export function pluginStore<T extends object>(
  pinia: Pinia,
  host: number,
  plugin: string,
  store: PluginStore<T>,
  page: PageContext,
): T {
  const id = `plugin:${host}:${plugin}:${store.name}`
  const existing = store.made.get(id)
  if (existing) {
    return existing
  }
  let held: T | null = null
  const instance = defineStore(id, () => {
    held = markRaw(store.setup(page))
    return {}
  })(pinia)
  if (held === null) {
    throw new Error(`the store ${id} was made before, and disposed of elsewhere`)
  }
  const made: T = held
  store.made.set(id, made)
  let stores = disposers.get(pinia)
  if (!stores) {
    stores = new Map()
    disposers.set(pinia, stores)
  }
  stores.set(id, {
    plugin,
    dispose: () => {
      store.made.delete(id)
      instance.$dispose()
      // A store made again starts anew.
      delete pinia.state.value[id]
    },
  })
  return made
}

/** Disposes every store of `plugin` in `pinia`, as the shell does when the user turns the plugin off. */
export function disposePluginStores(pinia: Pinia, plugin: string): void {
  const stores = disposers.get(pinia)
  if (!stores) {
    return
  }
  for (const [id, store] of [...stores]) {
    if (store.plugin === plugin) {
      stores.delete(id)
      store.dispose()
    }
  }
}
