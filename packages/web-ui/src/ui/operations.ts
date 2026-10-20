import {
  callWithAsyncErrorHandling,
  computed,
  ErrorCodes,
  getCurrentInstance,
  inject,
  provide,
  shallowReactive,
  watch,
  type ComputedRef,
  type InjectionKey,
} from 'vue'

/**
 * What a control's press runs: it may return a promise, which is the work of
 * the control's operation until it settles. A template's inline handler such as
 * `open = true` returns its value, so the type admits any return.
 */
export type Action<A extends unknown[] = []> = (...args: A) => unknown

/**
 * An action on one target, such as removing project `app`: `{ action:
 * 'remove', target: 'project:app' }`. Operations that exclude each other on
 * one target, such as a dialog's Save and Delete, name the same `group`:
 * while one of them runs, the others start nothing. Without a group an
 * operation is a group of its own.
 */
export interface Operation {
  action: string
  target: string
  group?: string
}

function isPromise(value: unknown): value is Promise<unknown> {
  return typeof (value as { then?: unknown } | null)?.then === 'function'
}

/**
 * The running operations of a page, by their group and target, and how many
 * run on each target. The page has one (`operations`), so a control drawn
 * again, in a menu or a dialog opened again, finds its operation running;
 * nothing a control unmounts stops or forgets one. Every operation settles,
 * since a request carries its timeout (`apiRequest`).
 */
export function createOperations() {
  /** The action that runs, by its group and target. */
  const running = shallowReactive(new Map<string, string>())
  /** How many operations run, by target. */
  const onTargets = shallowReactive(new Map<string, number>())
  const slotOf = (operation: Operation) => `${operation.group ?? operation.action}\u0000${operation.target}`

  function count(target: string, step: 1 | -1): void {
    const next = (onTargets.get(target) ?? 0) + step
    if (next === 0) {
      onTargets.delete(target)
    } else {
      onTargets.set(target, next)
    }
  }

  /**
   * The operations whose `run` is being called, with the work of the starts
   * that joined them from inside, which is theirs too.
   */
  const starting = new Map<string, unknown[]>()
  /** Who waits for a group on a target to be free (`whenFree`), by its slot. */
  const waiting = new Map<string, () => void>()
  const freed = new Map<string, Promise<void>>()

  function free(slot: string): void {
    waiting.get(slot)?.()
    waiting.delete(slot)
    freed.delete(slot)
  }

  /** The work `result` stands for: the promise or promises in it, once they all settle. */
  function settled(result: unknown): Promise<void> | null {
    const promises = (Array.isArray(result) ? result : [result]).filter(isPromise)
    // A rejection is reported where the work was started (the control's
    // error handling, the store's toast); here it only ends the operation.
    return promises.length === 0 ? null : Promise.allSettled(promises).then(() => {})
  }

  /**
   * Starts `operation` with `run`, unless it is blocked; its work is the
   * promise or promises `run` returns, and it runs until they all settle.
   * Answers when it settled, or null when nothing started. A start of the
   * same operation from inside `run`, as when a button's handler calls the
   * store that starts it, joins it: its `run` is called and its work is
   * part of the one operation.
   */
  function start(operation: Operation, run: () => unknown): Promise<void> | null {
    const slot = slotOf(operation)
    const joining = starting.get(slot)
    if (joining) {
      const result = run()
      joining.push(result)
      return settled(result) ?? Promise.resolve()
    }
    if (running.has(slot)) {
      return null
    }
    const results: unknown[] = []
    starting.set(slot, results)
    try {
      results.push(run())
    } finally {
      starting.delete(slot)
    }
    const work = settled(results.flat())
    if (work === null) {
      free(slot)
      return Promise.resolve()
    }
    running.set(slot, operation.action)
    count(operation.target, 1)
    return work.then(() => {
      running.delete(slot)
      count(operation.target, -1)
      free(slot)
    })
  }

  return {
    /** Whether `operation` runs: a control bound to it shows the spinner. */
    runs: (operation: Operation): boolean => running.get(slotOf(operation)) === operation.action,
    /** Whether a press of `operation` starts nothing: it, or another of its group, runs on its target. */
    blocked: (operation: Operation): boolean => running.has(slotOf(operation)),
    /** Whether any operation runs on `target`: its row shows the spinner. */
    onTarget: (target: string): boolean => onTargets.has(target),
    start,
    /**
     * Starts `operation` with `work`, as `start`, and answers the work's own
     * promise, which rejects with its failure for the caller to report; null
     * when nothing started.
     */
    run<T>(operation: Operation, work: () => Promise<T>): Promise<T> | null {
      let started: Promise<T> | null = null
      start(operation, () => (started = work()))
      return started
    },
    /**
     * Resolves once no operation of `operation`'s group runs on its target,
     * for work that waits for it rather than being refused, such as an edit
     * made during a provider's test.
     */
    whenFree(operation: Operation): Promise<void> {
      const slot = slotOf(operation)
      if (!running.has(slot) && !starting.has(slot)) {
        return Promise.resolve()
      }
      let promise = freed.get(slot)
      if (!promise) {
        promise = new Promise<void>((resolve) => waiting.set(slot, resolve))
        freed.set(slot, promise)
      }
      return promise
    },
  }
}

export type Operations = ReturnType<typeof createOperations>

/** The page's running operations. */
export const operations = createOperations()

const rowTargetKey: InjectionKey<() => string | undefined> = Symbol('rowTarget')

/**
 * Makes the component a row of `target`: it shows the spinner while any
 * operation on its target runs (`operations.onTarget`), and the row actions
 * inside it of the same target, its icon buttons and the items of a menu
 * opened from it, leave the spinner to it.
 */
export function provideRowTarget(target: () => string | undefined): ComputedRef<boolean> {
  provide(rowTargetKey, target)
  return computed(() => {
    const shown = target()
    return shown !== undefined && operations.onTarget(shown)
  })
}

/** The listener an attribute carries: one function, or the array Vue merges several into. */
function listenersOf(value: unknown): Function | Function[] | null {
  if (typeof value === 'function') {
    return value
  }
  return Array.isArray(value) && value.every((each) => typeof each === 'function') ? value : null
}

/**
 * The pressing of one control bound to `operation`, as Button, IconButton
 * and MenuItem press. `press(listener, ...args)` starts the operation with
 * the control's listener, through Vue's error handling as any event
 * handler's, so a rejection is reported as one; it starts nothing while the
 * operation's group runs on its target. `running` is while the operation
 * runs, whenever the control is drawn; `blocked` while it or another of its
 * group does. `showsSpinner` is false for a row action (`rowAction`) in a
 * row of the same target, which shows it instead.
 *
 * A control given no operation is the target of its own: its press waits
 * while the control stays, for a control whose target lives as long as it
 * does, such as a specimen's.
 */
export function useOperationControl(operation: () => Operation | undefined, rowAction: boolean) {
  const instance = getCurrentInstance()
  const own: Operation = { action: 'press', target: `control:${instance?.uid ?? 0}` }
  const rowTarget = rowAction ? inject(rowTargetKey, null) : null
  const bound = computed(() => operation() ?? own)
  const running = computed(() => operations.runs(bound.value))
  const blocked = computed(() => operations.blocked(bound.value))
  const showsSpinner = computed(() => running.value && rowTarget?.() !== bound.value.target)

  function press(listener: unknown, ...args: unknown[]): void {
    const handlers = listenersOf(listener)
    if (handlers === null) {
      return
    }
    void operations.start(bound.value, () =>
      callWithAsyncErrorHandling(handlers, instance, ErrorCodes.NATIVE_EVENT_HANDLER, args),
    )
  }

  return { running, blocked, showsSpinner, press }
}

/**
 * The openings of a dialog, for work started in it to answer only there:
 * `shownNow()`, taken as the work starts, answers later whether that same
 * opening is still shown, not closed and not opened again since.
 */
export function useOpening(isOpen: () => boolean): { shownNow: () => () => boolean } {
  let opening = 0
  watch(isOpen, (open) => {
    if (open) {
      opening++
    }
  })
  return {
    shownNow() {
      const started = opening
      return () => isOpen() && opening === started
    },
  }
}
