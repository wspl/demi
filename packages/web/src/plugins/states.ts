import { effectScope, reactive, watch, type EffectScope } from 'vue'
import type { PluginCallError, StateFeed } from '@demicodes/web-ui/plugins/page'
import { apiRequest, readResponse } from '../api/client'
import { pluginStateAnswerSchema, type ProductState } from '../api/generated/web-api'
import { isNewer, type RunRevision } from '../state/revisions'
import { readFailure } from './errors'

/** What the page holds of one plugin's state of one conversation. */
interface Held {
  /** The revision of the state held; null while it holds none. */
  held: RunRevision | null
  /** The state last read; undefined before the first answer. */
  state: unknown
  error: PluginCallError | null
  followers: number
  reading: boolean
  /** Another read was asked for while one was in flight. */
  again: boolean
}

/**
 * The plugins' conversation states, as a page reads them by revision
 * (`web-api.md` § Conversation state of plugins): while anything follows a
 * plugin's state of a conversation, the page reads it whenever the
 * conversation's summary carries a newer revision than the one it holds, a
 * count of another run of the backend included (`web-api.md` § Revisions
 * counted in memory). A read that cannot reach the backend waits for it in
 * the HTTP client and shows no failure (`plugin-pages.md` § The page
 * context).
 * What it read stays for the page's lifetime, so following again shows it at
 * once.
 */
export function conversationStates(snapshot: () => ProductState | null) {
  const held = reactive(new Map<string, Held>())
  /** Each followed state's watch, which outlives the scope of whoever follows first. */
  const watching = new Map<string, EffectScope>()

  function summaryRevision(plugin: string, conversation: string): RunRevision | null {
    const state = snapshot()
    const summary = state?.conversations.find((entry) => entry.id === conversation)
    const revision = summary?.pluginRevisions.find((entry) => entry.plugin === plugin)?.revision
    return state && revision !== undefined ? { run: state.run, revision } : null
  }

  function entryOf(key: string): Held {
    if (!held.has(key)) {
      held.set(key, { held: null, state: undefined, error: null, followers: 0, reading: false, again: false })
    }
    // The map's own (reactive) view of the entry, never the plain object it was made from.
    return held.get(key)!
  }

  /** One read at a time; an answer, of the run the page holds now, is taken only when it is newer than what is held. */
  async function read(entry: Held, plugin: string, conversation: string): Promise<void> {
    if (entry.reading) {
      entry.again = true
      return
    }
    entry.reading = true
    try {
      const path = `/conversations/${encodeURIComponent(conversation)}/plugins/${encodeURIComponent(plugin)}/state`
      const answer = await readResponse(await apiRequest(path), pluginStateAnswerSchema)
      const taken = { run: snapshot()?.run ?? null, revision: answer.revision }
      if (isNewer(taken, entry.held)) {
        entry.held = taken
        entry.state = answer.state
      }
      entry.error = null
    } catch (error) {
      entry.error = readFailure(error)
    } finally {
      entry.reading = false
      if (entry.again) {
        entry.again = false
        void read(entry, plugin, conversation)
      }
    }
  }

  function follow(plugin: string, conversation: string): StateFeed {
    const key = `${conversation}\u0000${plugin}`
    const entry = entryOf(key)
    entry.followers += 1
    if (!watching.has(key)) {
      const scope = effectScope(true)
      scope.run(() =>
        watch(
          () => summaryRevision(plugin, conversation),
          (revision) => {
            if (revision !== null && isNewer(revision, entry.held)) {
              void read(entry, plugin, conversation)
            }
          },
          { immediate: true },
        ),
      )
      watching.set(key, scope)
    }
    let stopped = false
    return {
      value: () => entry.state,
      error: () => entry.error,
      read: () => void read(entry, plugin, conversation),
      stop() {
        if (stopped) {
          return
        }
        stopped = true
        entry.followers -= 1
        if (entry.followers === 0) {
          watching.get(key)?.stop()
          watching.delete(key)
        }
      },
    }
  }

  return { follow }
}
