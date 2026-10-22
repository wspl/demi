import { onMounted, watch, type Ref } from 'vue'
import { useOverlayScrollbars } from 'overlayscrollbars-vue'
import { scrollbarsOptions, scrollbarsTarget } from '../ui/scrollbars'

/**
 * The app's scrollbar on a multi-line text field that scrolls past its height
 * (`ui/scrollbars.ts`): the bar floats inside `frame`, an element positioned
 * over the field's box and no larger, which holds nothing else the bar would
 * run under.
 *
 * A field's text is no change OverlayScrollbars observes, so it measures again
 * for each new `value`.
 */
export function useFieldScrollbars(
  field: Ref<HTMLElement | undefined>,
  frame: Ref<HTMLElement | undefined>,
  value: () => string,
): void {
  const [initialize, scrollbars] = useOverlayScrollbars({ options: scrollbarsOptions('y') })
  watch(value, () => scrollbars()?.update(true), { flush: 'post' })
  onMounted(() => {
    if (field.value && frame.value)
      initialize(scrollbarsTarget(field.value, frame.value))
  })
}
