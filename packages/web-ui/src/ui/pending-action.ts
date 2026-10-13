import {
  callWithAsyncErrorHandling,
  computed,
  ErrorCodes,
  getCurrentInstance,
  inject,
  provide,
  shallowReactive,
  ref,
  type InjectionKey,
  type Ref,
} from 'vue'

/**
 * What a control's press runs: it may return a promise, and the control shows
 * that it works until the promise settles (`web-application.md` § Responding
 * to the user, "No click goes unanswered"). A handler written `async` or
 * returning its request's promise is all it takes; no handler keeps a
 * `loading`, `saving` or `busy` flag for this. A template's inline handler
 * such as `open = true` returns its value, so the type admits any return.
 */
export type Action<A extends unknown[] = []> = (...args: A) => unknown

/** The row whose controls start work: it takes the work to show on itself. */
export interface RowWork {
  /** `settled` never rejects; the row works until every promise tracked has settled. */
  track: (settled: Promise<unknown>) => void
}

const rowWorkKey: InjectionKey<RowWork | null> = Symbol('rowWork')

/** One row's work: whether it works, and the way its controls hand it their promises. */
export function createRowWork(): RowWork & { working: Readonly<Ref<boolean>> } {
  const count = ref(0)
  return {
    working: computed(() => count.value > 0),
    track(settled) {
      count.value++
      void settled.then(() => {
        count.value--
      })
    },
  }
}

/**
 * The work of many rows that one menu serves, such as the sidebar's one
 * project menu for every project: `scope(key)` is the row `key`'s
 * (`RowWorkScope` gives it to the menu while it acts on that row), and
 * `isWorking(key)` tells the row to show the spinner.
 */
export function createKeyedRowWork() {
  const rows = shallowReactive(new Map<string, ReturnType<typeof createRowWork>>())
  function row(key: string) {
    let found = rows.get(key)
    if (!found) {
      found = createRowWork()
      rows.set(key, found)
    }
    return found
  }
  return {
    scope: (key: string): RowWork => row(key),
    isWorking: (key: string): boolean => rows.get(key)?.working.value ?? false,
  }
}

/**
 * Makes the component a row that shows the work of the row actions inside it
 * (its icon buttons and menu items, a menu opened from it included): the
 * menu closes at once and the row works until the action settles. Its text
 * buttons show their work on themselves. Returns whether the row works, for its template to show a spinner
 * (`RowSpinner`). A dialog or sheet inside the row starts its own scope
 * (`endRowWork`): its controls show their work on themselves.
 */
export function provideRowWork(): Readonly<Ref<boolean>> {
  const row = createRowWork()
  provide(rowWorkKey, row)
  return row.working
}

/** Provides `scope` to the controls below, as a row component does for itself; for a menu that serves many rows (`RowWorkScope`). */
export function provideRowWorkScope(scope: () => RowWork): void {
  provide(rowWorkKey, { track: (settled) => scope().track(settled) })
}

/** Ends the row scope for what is inside this component, as a dialog's controls are not the row's. */
export function endRowWork(): void {
  provide(rowWorkKey, null)
}

function isFunction(value: unknown): value is Function {
  return typeof value === 'function'
}

/** The listener an attribute carries: one function, or the array Vue merges several into. */
function listenersOf(value: unknown): Function | Function[] | null {
  if (isFunction(value)) {
    return value
  }
  return Array.isArray(value) && value.every(isFunction) ? value : null
}

function isPromise(value: unknown): value is Promise<unknown> {
  return typeof (value as { then?: unknown } | null)?.then === 'function'
}

/**
 * The pressing of one control. `press(listener, ...args)` runs the control's
 * listener (a function or the array Vue merges several into) and, when it
 * returns a promise, holds the control `pending` until it settles; a press
 * while pending does nothing. A row action (`reportsToRow`: an icon button
 * or a menu item) inside a row (`provideRowWork`) leaves the showing to the
 * row and `showsWork` is false; any other control shows its work itself, as
 * a button does with its spinner.
 * A promise that rejects is reported through Vue's error handling, as any
 * event handler's is, and the control is free again.
 */
export function usePendingAction(reportsToRow: boolean) {
  const instance = getCurrentInstance()
  const row = reportsToRow ? inject(rowWorkKey, null) : null
  const count = ref(0)
  const pending = computed(() => count.value > 0)

  function press(listener: unknown, ...args: unknown[]): void {
    if (pending.value) {
      return
    }
    const handlers = listenersOf(listener)
    if (handlers === null) {
      return
    }
    const result: unknown = callWithAsyncErrorHandling(
      handlers,
      instance,
      ErrorCodes.NATIVE_EVENT_HANDLER,
      args,
    )
    const promises = (Array.isArray(result) ? result : [result]).filter(isPromise)
    if (promises.length === 0) {
      return
    }
    // Vue already reports a rejection; here it only ends the wait.
    const settled = Promise.allSettled(promises)
    count.value++
    row?.track(settled)
    void settled.then(() => {
      count.value--
    })
  }

  return { pending, showsWork: row === null, press }
}
