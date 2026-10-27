<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref, shallowRef, watch } from 'vue'
import { useElementSize, useWindowSize } from '@vueuse/core'
import { mirrorPage } from './frame-mirror'
import { passInput } from './user-input'

/**
 * A text area in a document of its own, a same-origin frame, so that the focus
 * in it leaves the page's selection where it is: a document has one selection,
 * and a field in the page's own document would end it.
 *
 * ```text
 *   ┌ box: the place the field stands in ─┐
 *   │┌ frame ─────────────────────────────┼──────────────┐
 *   ││ text area, as wide as the box      │ hidden       │
 *   │└────────────────────────────────────┼──────────────┘
 *   └─────────────────────────────────────┘
 *    the frame is as wide as the window
 * ```
 *
 * The frame is as wide as the page's window, so that the page's style rules
 * for a window's width, as a phone's, hold in it as they do on the page; the
 * box shows only its text area's part. It is as tall as the text area, which
 * grows with its words as `field-sizing: content` lets it. The attributes and
 * listeners given to this component go to the text area, which `field`
 * exposes.
 *
 * A key with ⌘ or Control that the text area leaves goes on to the page, as
 * it would from a field of the page's own, so the app's shortcuts work in it,
 * and the page hears of every input to it (`user-input.ts`).
 */
defineOptions({ inheritAttrs: false })

const text = defineModel<string>({ default: '' })

const box = ref<HTMLElement>()
const frame = ref<HTMLIFrameElement>()
const body = shallowRef<HTMLElement | null>(null)
const field = ref<HTMLTextAreaElement>()
const { width: windowWidth } = useWindowSize()
const { width: boxWidth } = useElementSize(box)

let release = (): void => {}
/** Dresses the frame's document and moves the text area into it, again whenever the frame gets a new one, as when it is moved in the page. */
function take(): void {
  const inside = frame.value?.contentDocument
  const view = inside?.defaultView
  if (!inside?.body || !view || inside.body === body.value)
    return
  release()
  const unmirror = mirrorPage(inside)
  const unpass = passInput(view, window)
  view.addEventListener('keydown', passOn)
  view.addEventListener('keyup', passOn)
  release = () => {
    unmirror()
    unpass()
    view.removeEventListener('keydown', passOn)
    view.removeEventListener('keyup', passOn)
  }
  body.value = inside.body
}
onMounted(take)
onBeforeUnmount(() => release())

/**
 * Hands the page a key with ⌘ or Control the text area left, from the frame's
 * place in it, and the release of either, which holds neither any more.
 */
function passOn(event: KeyboardEvent): void {
  const command = event.metaKey || event.ctrlKey || event.key === 'Meta' || event.key === 'Control'
  if (event.defaultPrevented || !command || !frame.value)
    return
  const copy = new KeyboardEvent(event.type, event)
  frame.value.dispatchEvent(copy)
  if (copy.defaultPrevented)
    event.preventDefault()
}

// The frame's own realm observes the text area, which stands in its document.
// After the render, which moves the text area into a new document the frame got.
watch([field, body], ([element], _before, onCleanup) => {
  const view = element?.ownerDocument.defaultView
  if (!element || !view)
    return
  const sizes = new view.ResizeObserver(() => {
    if (frame.value)
      frame.value.style.height = `${element.offsetHeight}px`
  })
  sizes.observe(element)
  onCleanup(() => sizes.disconnect())
}, { flush: 'post' })

watch([field, boxWidth], ([element, width]) => {
  if (element)
    element.style.width = `${width}px`
}, { flush: 'post' })

defineExpose({
  field,
  /** Takes the focus into the frame and the text area. */
  focus(options?: FocusOptions): void {
    frame.value?.contentWindow?.focus()
    field.value?.focus(options)
  },
  /** Whether the text area has the focus. */
  get focused(): boolean {
    return !!field.value && document.activeElement === frame.value && field.value.ownerDocument.activeElement === field.value
  },
})
</script>

<template>
  <div ref="box" class="relative w-full min-w-0 overflow-hidden">
    <iframe
      ref="frame"
      data-framed-text-area
      class="block max-w-none border-0 bg-transparent"
      :style="{ width: `${windowWidth}px` }"
      @load="take"
    />
  </div>
  <Teleport v-if="body" :to="body">
    <textarea ref="field" v-model="text" v-bind="$attrs" />
  </Teleport>
</template>
