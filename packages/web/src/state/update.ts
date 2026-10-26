import { readonly, ref } from 'vue'
import type { UpdateEnd } from '@demicodes/web-ui/ui/UpdatingScreen.vue'
import { pageLog } from '@demicodes/web-ui/infra/log'
import { errorFields } from '@demicodes/web-ui/infra/errors'
import { updateStateSchema, type UpdateState } from '../api/generated/web-api'

/**
 * The update the backend carries out before it serves a new release
 * (`crates/backend-http/src/update.rs`): once any answer says `updating`,
 * or the state channel says the backend stops for an update of the server,
 * the page shows the update screen over everything and asks how far it is
 * until the backend serves. Then the screen shows how the update ended, and
 * the page loads itself again only when the user opens Demi, so every step
 * is seen to end: nothing else loads it while the screen shows.
 */

/**
 * What says the backend is away for a moment: a proxy in front of it while
 * it does not run, Cloudflare's own failures, and the backend's 503 while it
 * starts or updates.
 */
const AWAY = new Set([502, 503, 504, 520, 521, 522, 523, 524])

/** How often the screen asks how far the update is. */
const ASK_EVERY_MS = 2000

/** What the update screen shows: every step of the update, or null until the server says, and how it ended. */
export interface UpdateShown {
  progress: Extract<UpdateState, { state: 'updating' }> | null
  ended: UpdateEnd | null
}

const shown = ref<UpdateShown | null>(null)

/** What the update screen shows; null while the backend is not updating. */
export const update = readonly(shown)

/** What the screen shows of `state`, which says the server updates. */
function shownOf(state: UpdateState | null): UpdateShown {
  return { progress: state?.state === 'updating' ? state : null, ended: null }
}

/**
 * The update ended, as the backend that serves now says, `state`, or, with
 * none, a backend of an earlier release that has no such answer. The server
 * updated when it runs the release the update moved to, with every step done
 * as that backend says or, when it started again since, as the screen shows
 * them; otherwise it returned to the release it left, whether or not the
 * screen heard it return.
 */
function end(state: Extract<UpdateState, { state: 'none' }> | null): void {
  const last = shown.value?.progress ?? null
  // A screen that never heard how far the update was has nothing to tell
  // a return by, and says only that Demi is updated.
  const updated = last === null || (state !== null && state.version === last.to)
  if (updated && last !== null && state?.updated) {
    shown.value = { progress: { ...last, returning: false, steps: state.updated }, ended: 'updated' }
  } else {
    shown.value = { progress: last, ended: updated ? 'updated' : 'returned' }
  }
  pageLog.info('update.ended', { ended: shown.value.ended, version: state?.version ?? null })
}

/**
 * An answer said `updating`, or the channel said the backend stops for an
 * update, `state`: the page shows the screen and follows the update. The app
 * under the screen takes no focus, pointer or key: a draft's Enter or a
 * shortcut would act unseen.
 */
export function noteUpdating(state: UpdateState | null = null): void {
  if (shown.value !== null)
    return
  shown.value = shownOf(state)
  document.getElementById('app')?.setAttribute('inert', '')
  window.addEventListener('keydown', (event) => event.stopImmediatePropagation(), { capture: true })
  pageLog.info('update.shown', {})
  void follow()
}

/** Asks how far the update is until the backend serves, then shows how it ended. */
async function follow(): Promise<void> {
  for (;;) {
    try {
      const response = await fetch('/api/update', { credentials: 'same-origin', cache: 'no-store' })
      if (response.ok) {
        const state = updateStateSchema.parse(await response.json())
        if (state.state === 'none') {
          end(state)
          return
        }
        shown.value = shownOf(state)
      } else if (!AWAY.has(response.status)) {
        // Any other answer comes from a backend that serves without this
        // route, as after a return to an earlier release, whose session gate
        // answers it 401 or 404.
        end(null)
        return
      }
      // Otherwise the backend, or a proxy in front of it, says it is away, as
      // between its stop and its start: the screen keeps what it shows.
    } catch (error) {
      pageLog.info('update.unreached', errorFields(error))
    }
    await new Promise((resolve) => setTimeout(resolve, ASK_EVERY_MS))
  }
}
