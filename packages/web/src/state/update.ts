import { readonly, ref } from 'vue'
import { reloadPage } from '@demicodes/web-ui/infra/build-reload'
import { pageLog } from '@demicodes/web-ui/infra/log'
import { errorFields } from '@demicodes/web-ui/infra/errors'
import { updateStateSchema, type UpdateState } from '../api/generated/web-api'

/**
 * The update the backend carries out before it serves a new release
 * (`crates/backend-http/src/update.rs`): once any answer says `updating`,
 * the page shows the update screen over everything, asks how far it is
 * until the backend serves, and then loads itself again, in the build the
 * backend now serves.
 */

/**
 * What says the backend is away for a moment: a proxy in front of it while
 * it does not run, Cloudflare's own failures, and the backend's 503 while it
 * starts or updates.
 */
const AWAY = new Set([502, 503, 504, 520, 521, 522, 523, 524])

/** How often the screen asks how far the update is. */
const ASK_EVERY_MS = 2000

/** What the update screen shows: how far the backend is, or null until it says. */
export interface UpdateShown {
  progress: Extract<UpdateState, { state: 'updating' }> | null
}

const shown = ref<UpdateShown | null>(null)

/** What the update screen shows; null while the backend is not updating. */
export const update = readonly(shown)

/**
 * An answer said `updating`: the page shows the screen and follows the
 * update. The app under the screen takes no focus, pointer or key: a draft's
 * Enter or a shortcut would act unseen.
 */
export function noteUpdating(): void {
  if (shown.value !== null)
    return
  shown.value = { progress: null }
  document.getElementById('app')?.setAttribute('inert', '')
  window.addEventListener('keydown', (event) => event.stopImmediatePropagation(), { capture: true })
  pageLog.info('update.shown', {})
  void follow()
}

/** Asks how far the update is until the backend serves, then loads the page again. */
async function follow(): Promise<void> {
  for (;;) {
    try {
      const response = await fetch('/api/update', { credentials: 'same-origin', cache: 'no-store' })
      if (response.ok) {
        const state = updateStateSchema.parse(await response.json())
        if (state.state === 'none') {
          reloadPage()
          return
        }
        shown.value = { progress: state }
      } else if (!AWAY.has(response.status)) {
        // Any other answer comes from a backend that serves without this
        // route, as after a return to an earlier release, whose session gate
        // answers it 401 or 404: the page loads itself again.
        reloadPage()
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
