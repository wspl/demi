import { readonly, ref, toValue, type MaybeRefOrGetter, type Ref } from 'vue'
import { useEventListener } from '@vueuse/core'

/** Whether a drag carries files: from the desktop, a folder too. */
export function transferHasFiles(transfer: DataTransfer | null | undefined): boolean {
  return transfer?.types.includes('Files') === true
}

export interface FileDropOptions {
  /** Whether drops are taken now; while false a drag passes by as it would over any element. */
  enabled?: MaybeRefOrGetter<boolean>
  /** Each move of the drag over the target, for a target that picks a spot under the pointer. */
  over?: (event: DragEvent) => void
  drop: (event: DragEvent) => void
}

/**
 * A drag carrying files over `target`: whether one is there, where it moves,
 * and its drop, taken as a copy. Every element under the pointer sends its
 * own enter and leave, so the drag is over while any element it entered is
 * still inside the target: an element removed under the pointer never sends
 * its leave. (VueUse's `useDropZone` counts enters against leaves instead,
 * and stays lit once a row it entered is gone, as a tree's rows go when a
 * folder under the drag opens.)
 */
export function useFileDrop(
  target: MaybeRefOrGetter<HTMLElement | null | undefined>,
  options: FileDropOptions,
): { over: Readonly<Ref<boolean>> } {
  const over = ref(false)
  const entered = new Set<Node>()

  function takes(event: DragEvent): boolean {
    return toValue(options.enabled ?? true) && transferHasFiles(event.dataTransfer)
  }

  useEventListener(target, 'dragenter', (event) => {
    if (!takes(event))
      return
    event.preventDefault()
    if (event.target instanceof Node)
      entered.add(event.target)
    over.value = true
  })
  useEventListener(target, 'dragover', (event) => {
    if (!takes(event))
      return
    event.preventDefault()
    if (event.dataTransfer)
      event.dataTransfer.dropEffect = 'copy'
    over.value = true
    options.over?.(event)
  })
  useEventListener(target, 'dragleave', (event) => {
    if (event.target instanceof Node)
      entered.delete(event.target)
    const root = toValue(target)
    for (const node of entered) {
      if (!root?.contains(node))
        entered.delete(node)
    }
    if (entered.size === 0)
      over.value = false
  })
  useEventListener(target, 'drop', (event) => {
    if (!takes(event))
      return
    event.preventDefault()
    entered.clear()
    over.value = false
    options.drop(event)
  })

  return { over: readonly(over) }
}
