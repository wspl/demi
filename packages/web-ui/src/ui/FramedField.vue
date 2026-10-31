<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref, shallowRef, watch } from 'vue'
import { useElementSize, useWindowSize } from '@vueuse/core'
import { mirrorPage } from './frame-mirror'
import { passInput } from './user-input'

/**
 * A field in a document of its own, a same-origin frame, so that the focus
 * in it leaves the page's selection where it is: a document has one selection,
 * and a field in the page's own document would end it.
 *
 * ```text
 *   ┌ box: the place the field stands in ─┐
 *   │┌ frame ─────────────────────────────┼──────────────┐
 *   ││ the field, as wide as the box      │ hidden       │
 *   │└────────────────────────────────────┼──────────────┘
 *   └─────────────────────────────────────┘
 *    the frame is as wide as the window
 * ```
 *
 * The frame is as wide as the page's window, so that the page's style rules
 * for a window's width, as a phone's, hold in it as they do on the page; the
 * box shows only its field's part. It is as tall as the field. The default
 * slot is the field, which the frame's document holds; it gets that document,
 * which an editor is made in: ProseMirror listens to the document its editor
 * is made in, so one made on the page and moved into the frame would not hear
 * its selection.
 *
 * A key with ⌘ or Control that the field leaves goes on to the page, as it
 * would from a field of the page's own, so the app's shortcuts work in it,
 * and the page hears of every input to it (`user-input.ts`).
 */
const box = ref<HTMLElement>()
const frame = ref<HTMLIFrameElement>()
const body = shallowRef<HTMLElement | null>(null)
const field = ref<HTMLElement>()
const { width: windowWidth } = useWindowSize()
const { width: boxWidth } = useElementSize(box)

let release = (): void => {}
/** Dresses the frame's document and moves the field into it, again whenever the frame gets a new one, as when it is moved in the page. */
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
 * Hands the page a key with ⌘ or Control the field left, from the frame's
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

// The frame's own realm observes the field, which stands in its document.
// After the render, which moves the field into a new document the frame got.
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
  /** The frame's document, once the field stands in it. */
  get document(): Document | null {
    return body.value?.ownerDocument ?? null
  },
  /** Moves the focus into the frame, as its field takes it. */
  focusFrame(): void {
    frame.value?.contentWindow?.focus()
  },
  /** Whether the focus is in the frame. */
  get focused(): boolean {
    return document.activeElement === frame.value
  },
})
</script>

<template>
  <div ref="box" class="relative w-full min-w-0 overflow-hidden">
    <iframe
      ref="frame"
      data-framed-field
      class="block max-w-none border-0 bg-transparent"
      :style="{ width: `${windowWidth}px` }"
      @load="take"
    />
  </div>
  <Teleport v-if="body" :to="body">
    <div ref="field">
      <slot :document="body.ownerDocument" />
    </div>
  </Teleport>
</template>
