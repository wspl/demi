import { nextTick, onBeforeUnmount, onMounted, ref, watch, type Ref } from 'vue'
import { useEventListener } from '@vueuse/core'

/**
 * "Fit N, count the rest as +N": items that stand on one line, of which as many
 * show as leave room for the count of the others.
 *
 * The rule is `fitMore`, pure arithmetic on widths; `useFitMore` is what measures
 * a live box for it. How the count looks is each caller's: the row's pictures a
 * faint number, a tag line a Tag with a tooltip, the changed files a button.
 */

/** What a box holds, as widths in px. */
export interface FitMoreInput {
  /** The width of one line. */
  room: number
  /** The width of each item, in order. */
  widths: readonly number[]
  /** The space between neighbours. */
  gap: number
  /** How wide the count of `rest` hidden items is drawn. */
  countWidth: (rest: number) => number
}

/** How many items show, and how many the count after them names (0 for none). */
export interface FitMore {
  shown: number
  more: number
}

/**
 * Which of the items show, left to right with the gap between neighbours.
 *
 * - When all fit, all show.
 * - Otherwise as many show as leave room for the count of the rest after the
 *   last of them.
 * - When not even the count fits, nothing shows and nothing is counted.
 */
export function fitMore(input: FitMoreInput): FitMore {
  const { room, widths, gap, countWidth } = input
  const all = widths.length

  /** Whether the first `shown` items and then the count of the rest fit on the line. */
  function fits(shown: number): boolean {
    const placed = widths.slice(0, shown)
    if (shown < all)
      placed.push(countWidth(all - shown))
    const used = placed.reduce((sum, width) => sum + width, 0) + gap * Math.max(0, placed.length - 1)
    return used <= room
  }

  for (let shown = all; shown >= 0; shown -= 1) {
    if (fits(shown))
      return { shown, more: all - shown }
  }
  return { shown: 0, more: 0 }
}

export interface FitMoreOptions {
  /** The items the box holds; another list fits again. */
  items: () => readonly unknown[]
  /** The box whose width is the room; a change of its size fits again. */
  box: () => HTMLElement | null | undefined
  /** The widths of the items as they stand with all of them shown; null when the box cannot be measured. */
  measure: () => FitMoreInput | null
}

/**
 * How many of a box's items show. Every item shows first, so `measure` reads them
 * all.
 *
 * Then the count settles, again whenever the box's width changes, the items change
 * or a web font finishes loading (it changes the widths without changing the box).
 * Only the width decides what fits: a height the items themselves give the box, as
 * a caller that shows every item on request does, fits nothing again, so the count
 * and the control beside it stay as they are.
 */
export function useFitMore(options: FitMoreOptions): Ref<number> {
  const shown = ref(options.items().length)
  let observer: ResizeObserver | undefined
  let current = 0

  async function fit(): Promise<void> {
    const mine = ++current
    shown.value = options.items().length
    await nextTick()
    const measured = options.measure()
    if (mine !== current || !measured || options.items().length === 0)
      return
    shown.value = fitMore(measured).shown
  }

  onMounted(() => {
    let width: number | undefined
    observer = new ResizeObserver((entries) => {
      const now = entries.at(-1)?.contentRect.width
      if (now === width)
        return
      width = now
      void fit()
    })
    const box = options.box()
    if (box)
      observer.observe(box)
    void fit()
  })
  onBeforeUnmount(() => observer?.disconnect())
  useEventListener(document.fonts, 'loadingdone', () => { void fit() })
  watch(options.items, () => { void fit() })

  return shown
}
