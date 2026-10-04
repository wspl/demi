import { computed, onScopeDispose, ref, type ComputedRef } from 'vue'
import type { PageContext } from './page'

/**
 * The calls a page's controls wait for, by the key of what each acts on,
 * such as a skill source's id (`plugin-pages.md` § Calls and states).
 */
export interface PendingCalls {
  /** The keys with a call in flight, whose controls show it and take no input. */
  readonly pending: ComputedRef<readonly string[]>
  /**
   * Runs `call` for `key`, unless one is in flight for it. The key is
   * pending from now until the call is answered, success or failure, and a
   * failure is reported as `couldNot`. `call` receives the signal that
   * aborts when the calling scope ends; an abort it causes reports nothing.
   */
  run(key: string, couldNot: string, call: (signal: AbortSignal) => Promise<unknown>): Promise<void>
}

/** The pending calls of the calling scope, whose failures `errors` reports. */
export function pendingCalls(errors: PageContext['errors']): PendingCalls {
  const pending = ref<string[]>([])
  const lifetime = new AbortController()
  onScopeDispose(() => lifetime.abort())
  return {
    pending: computed(() => pending.value),
    async run(key, couldNot, call) {
      if (pending.value.includes(key)) {
        return
      }
      pending.value = [...pending.value, key]
      try {
        await call(lifetime.signal)
      } catch (error) {
        // A call that fails once its scope ended failed because the scope
        // aborted it, and no control is left to say so.
        if (!lifetime.signal.aborted) {
          errors.report(couldNot, error)
        }
      } finally {
        pending.value = pending.value.filter((entry) => entry !== key)
      }
    },
  }
}
