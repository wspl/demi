import { inject, onBeforeUnmount, provide, ref, shallowReactive, watch, type InjectionKey, type Ref } from 'vue'

interface RoomLabel {
  element: Ref<HTMLElement | null>
  text: () => string
  order: number
  compact: Ref<boolean>
}

const labelRoomKey: InjectionKey<{ add(label: RoomLabel): () => void }> = Symbol('label-room')

/**
 * Which labels give way, given the protected element's room as laid out now.
 * Labels return in ascending `order` while the room left still holds the next one.
 */
export function compactLabels(
  room: number,
  labels: ReadonlyArray<{ width: number, compact: boolean, order: number }>,
): boolean[] {
  // With every label hidden, the protected element has this much to spare.
  let spare = room
  for (const label of labels) {
    if (!label.compact)
      spare += label.width
  }
  const compact = labels.map(() => true)
  const byOrder = labels.map((label, index) => ({ ...label, index })).sort((a, b) => a.order - b.order)
  for (const label of byOrder) {
    // Strictly in order: a later label never shows while an earlier one cannot.
    if (spare < label.width)
      break
    compact[label.index] = false
    spare -= label.width
  }
  return compact
}

/**
 * Lets the controls under this component give up their labels for something
 * that needs the width more: a composer's editor, a header's title. `room` is
 * how much wider than its least the protected element is now, negative while
 * it is narrower. Labels go in descending `order` and return in ascending
 * order, each only when the protected element keeps its least width with it.
 */
export function provideLabelRoom(room: () => number | undefined): void {
  const labels = shallowReactive(new Set<RoomLabel>())
  provide(labelRoomKey, {
    add(label) {
      labels.add(label)
      return () => { labels.delete(label) }
    },
  })
  // A hidden label stays laid out, out of the flow, so its width is known either way.
  watch(
    () => [room(), ...[...labels].flatMap(label => [label.element.value, label.text()])],
    () => {
      const now = room()
      const measured = [...labels].filter(label => label.element.value)
      if (now === undefined) {
        for (const label of measured)
          label.compact.value = false
        return
      }
      const compact = compactLabels(now, measured.map(label => ({
        width: labelCost(label.element.value!),
        compact: label.compact.value,
        order: label.order,
      })))
      measured.forEach((label, index) => { label.compact.value = compact[index]! })
    },
    { flush: 'post' },
  )
}

/** What showing the label takes from the row: its own width and the gap its container puts before it. */
function labelCost(element: HTMLElement): number {
  const gap = element.parentElement ? Number.parseFloat(getComputedStyle(element.parentElement).columnGap) : 0
  // Layout is fractional and `offsetWidth` is not; a pixel of margin keeps a
  // label from returning into a row that rounding says has room for it.
  return Math.ceil(element.getBoundingClientRect().width) + (Number.isFinite(gap) ? gap : 0) + 1
}

/**
 * Whether this control is its icon alone for now, its label moved to a
 * tooltip. Bind the label element, and give a compact label the classes in
 * `COMPACT_LABEL_CLASS`. Without a provider above, the label always shows.
 */
export function useRoomLabel(
  element: Ref<HTMLElement | null>,
  text: () => string,
  order = 0,
): Ref<boolean> {
  const compact = ref(false)
  const remove = inject(labelRoomKey, null)?.add({ element, text, order, compact })
  if (remove)
    onBeforeUnmount(remove)
  return compact
}

/** Out of the flow and unseen, but still measurable. */
export const COMPACT_LABEL_CLASS = 'invisible absolute left-0 top-0'
