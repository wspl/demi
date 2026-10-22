import {
  computed,
  inject,
  onMounted,
  onScopeDispose,
  provide,
  shallowReactive,
  shallowRef,
  watch,
  watchEffect,
  type ComputedRef,
  type InjectionKey,
  type Ref,
} from 'vue'

// How a pane of the work panel changes whole
// (`docs/decisions/0038-a-pane-changes-whole.md`). A pane is the content of
// one tab: it reads several things before its view is complete, such as a
// file's text, its folder's listing and the listings of the folders above it.
//
//   WorkPanel ── PaneScope ──┬── a view's part ── usePaneReady(settled)
//   (provides `Pane`)        ├── another part ─── usePaneReady(settled)
//                            └── the area that waits: TreeFrame, a browser's
//                                area, the media's stage (usePaneWaiting)
//
// Each part of a view says whether what it needs of the Host is in. While any
// part says it is not, the area under the view's address stays empty, at its
// size, and draws once when the last part is in. The click that switched the
// pane has already changed the tab and the address. After `PANE_WAIT_MS` of
// the pane being shown and waiting, the area says what it waits for.
//
// The first view a panel shows, as the page loads or the panel opens
// (`docs/decisions/0049-the-work-panel-shows-at-once.md`), says what it waits
// for from its start: the user has no view yet that the sentence could
// interrupt, and an empty area tells them nothing. Only that first wait; a
// later read in the same pane, as when another file is picked, keeps the rule
// above.
//
// A part says "in" for what it first needs only: an answer it holds that is
// being read again is in, so a view switched back to, or one whose reads are
// kept, shows at once.

/** How long a shown pane waits before its area says what it waits for. */
export const PANE_WAIT_MS = 300

/** What a pane's parts and its waiting areas share. */
export interface Pane {
  /** Whether some part of the view has not read all it needs. */
  readonly waiting: ComputedRef<boolean>
  /** Whether the pane has been shown and waiting for `PANE_WAIT_MS`. */
  readonly slow: Readonly<Ref<boolean>>
  /** What the pane's tab is called, as the area names what it waits for. */
  readonly name: ComputedRef<string>
  /** Part `part` says whether all it needs is in. */
  report(part: symbol, settled: boolean): void
  /** Part `part` is gone, and waits for nothing. */
  forget(part: symbol): void
}

const PANE: InjectionKey<Pane> = Symbol('work panel pane')

/**
 * Makes the pane of the calling component, for the parts and areas below it.
 * `shown` says whether its tab is the selected one: a pane that waits while
 * another tab is selected has nothing to say yet.
 */
export function providePane(options: {
  name: () => string
  shown: () => boolean
  /**
   * Whether this is the first view the panel shows: its first wait says what
   * it waits for at once. Read once, as the pane is made.
   */
  first?: boolean
}): Pane {
  const unsettled = shallowReactive(new Set<symbol>())
  const waiting = computed(() => unsettled.size > 0)
  const slow = shallowRef(false)
  /**
   * Whether the pane's wait that comes next is its first view's. It ends with the first wait
   * that is over once the render it ends in has drawn: a part that settles may be replaced in
   * that render by the view's own, whose parts then read on, as a view that waits for the
   * conversation's directory is replaced by the files it lists.
   */
  let fromStart = options.first ?? false
  watch(waiting, (now) => {
    if (!now)
      fromStart = false
  }, { flush: 'post' })
  // A view whose data is all in as it mounts has no first wait to say anything in: its later
  // reads are the pane's ordinary ones.
  onMounted(() => {
    if (!waiting.value)
      fromStart = false
  })
  watch(() => waiting.value && options.shown(), (now, _before, onCleanup) => {
    slow.value = false
    if (!now)
      return
    if (fromStart) {
      slow.value = true
      return
    }
    const timer = setTimeout(() => {
      slow.value = true
    }, PANE_WAIT_MS)
    onCleanup(() => clearTimeout(timer))
  }, { immediate: true })
  const pane: Pane = {
    waiting,
    slow,
    name: computed(options.name),
    report(part, settled) {
      if (settled)
        unsettled.delete(part)
      else
        unsettled.add(part)
    },
    forget(part) {
      unsettled.delete(part)
    },
  }
  provide(PANE, pane)
  return pane
}

/**
 * Says whether all the calling part of a view needs is in, for as long as the
 * component lives. Called in `setup`, so the part waits from the render that
 * mounts it. Without a pane around it, as in a specimen, it does nothing.
 */
export function usePaneReady(settled: () => boolean): void {
  const pane = inject(PANE, null)
  if (!pane)
    return
  const part = Symbol('pane part')
  // Synchronous: the first answer is given before the part first renders.
  watchEffect(() => pane.report(part, settled()), { flush: 'sync' })
  onScopeDispose(() => pane.forget(part))
}

const SETTLED_PANE: Pane = {
  waiting: computed(() => false),
  slow: shallowRef(false),
  name: computed(() => ''),
  report: () => {},
  forget: () => {},
}

/** The pane around the calling component; one that never waits outside any. */
export function usePaneWaiting(): Pane {
  return inject(PANE, SETTLED_PANE)
}
