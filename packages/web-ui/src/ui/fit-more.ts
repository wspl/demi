import { nextTick, onBeforeUnmount, onMounted, ref, watch, type Ref } from 'vue'
import { useEventListener } from '@vueuse/core'

/**
 * "Fit N, count the rest as +N": items that stand on one line, or on a few,
 * of which as many show as leave room for the count of the others. The rule
 * is `fitMore`, pure arithmetic on widths; `useFitMore` is what measures a
 * live box for it. How the count looks is each caller's: the row's
 * pictures a faint number, a tag line a Tag with a tooltip, the changed
 * files a button.
 */

/** What a box holds, as widths in px. */
export interface FitMoreInput {
  /** The width of one line. */
  room: number
  /** The width of each item, in order. */
  widths: readonly number[]
  /** The space between neighbours, on a line and between lines. */
  gap: number
  /** How wide the count of `rest` hidden items is drawn. */
  countWidth: (rest: number) => number
  /** How many lines the items may take; one by default. */
  rows?: number
}

/** How many items show, and how many the count after them names (0 for none). */
export interface FitMore {
  shown: number
  more: number
}

/**
 * Which of the items show. Items flow left to right and a line breaks where
 * the next item no longer fits after its neighbours and the gap. When all
 * fit within `rows` lines, all show; otherwise as many show as leave room,
 * within the same lines, for the count of the rest after the last of them.
 * When not even the count fits, nothing shows and nothing is counted. An item
 * wider than a line fits nowhere.
 */
export function fitMore(input: FitMoreInput): FitMore {
  const { room, widths, gap, countWidth } = input
  const rows = input.rows ?? 1
  const all = widths.length

  /** Whether the first `shown` items and then the count of the rest fit within `rows` lines. */
  function fits(shown: number): boolean {
    const placed = widths.slice(0, shown)
    if (shown < all)
      placed.push(countWidth(all - shown))
    let line = 1
    let used = 0
    for (const width of placed) {
      if (width > room)
        return false
      if (used === 0) {
        used = width
      } else if (used + gap + width <= room) {
        used += gap + width
      } else {
        line += 1
        used = width
      }
    }
    return line <= rows
  }

  for (let shown = all; shown >= 0; shown -= 1) {
    if (fits(shown))
      return { shown, more: all - shown }
  }
  return { shown: 0, more: 0 }
}

/** What `useFitMore` reads from the live box, once every item is on the page. */
export type FitMoreMeasure = Omit<FitMoreInput, 'rows'> & { rows?: number }

export interface FitMoreOptions {
  /** The items the box holds; another list fits again. */
  items: () => readonly unknown[]
  /** The box whose width is the room; a change of its size fits again. */
  box: () => HTMLElement | null | undefined
  /** The widths of the items as they stand with all of them shown; null when the box cannot be measured. */
  measure: () => FitMoreMeasure | null
  /** False while the box shows everything on purpose, such as an expanded list. */
  enabled?: () => boolean
}

/**
 * How many of a box's items show. Every item shows first, so `measure` reads
 * them all, then the count settles, again whenever the box resizes, the
 * items change, a web font finishes loading (it changes the widths without
 * changing the box) or `enabled` turns on.
 */
export function useFitMore(options: FitMoreOptions): Ref<number> {
  const shown = ref(options.items().length)
  let observer: ResizeObserver | undefined
  let current = 0

  async function fit(): Promise<void> {
    if (options.enabled && !options.enabled())
      return
    const mine = ++current
    shown.value = options.items().length
    await nextTick()
    const measured = options.measure()
    if (mine !== current || !measured || options.items().length === 0)
      return
    shown.value = fitMore(measured).shown
  }

  onMounted(() => {
    observer = new ResizeObserver(() => { void fit() })
    const box = options.box()
    if (box)
      observer.observe(box)
    void fit()
  })
  onBeforeUnmount(() => observer?.disconnect())
  useEventListener(document.fonts, 'loadingdone', () => { void fit() })
  watch([options.items, () => options.enabled?.() ?? true], () => { void fit() })

  return shown
}
