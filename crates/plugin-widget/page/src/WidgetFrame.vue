<script setup lang="ts">
/**
 * A widget's page, filling the box it is in: a frame sandboxed with
 * `HTML_SANDBOX`, an opaque origin, which loads the widget's document as a
 * page of its own, served by the backend (`crates/backend-http/src/blocks.rs`),
 * so the policy of the page around it never reaches its scripts. The reply's
 * block shows a widget through it, as does a widget's share.
 *
 * - The page draws in the reply's appearance (`./look.ts`); a change of the
 *   appearance draws it again.
 * - Until its page has loaded, the frame is hidden, since the page's styles
 *   are made by a script it loads and its markup would show first without
 *   them, and the box shows a spinner once loading takes longer than
 *   `SPINNER_AFTER_MS`.
 * - It emits `height` with each height the page reports for its content, and
 *   `unavailable` when the route could not serve the document, as the page it
 *   answered then says (`{ replyBlock: "unavailable" }`).
 * - `picture()` asks the page for a PNG of what the frame shows now, which
 *   the page draws itself (`crates/plugin-widget/plugin/src/bootstrap.js`):
 *   the frame's parent cannot read an opaque origin.
 */
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { HTML_SANDBOX, IndeterminateSpinner, usePage } from '@demicodes/plugin-sdk'
import { useLook } from './look'
import { t } from './locales/text'

const props = defineProps<{ id: string, title: string }>()
const emit = defineEmits<{ height: [height: number], unavailable: [] }>()
const page = usePage()

/**
 * How long a widget loads before its box shows a spinner: a page that loads
 * sooner shows none, so nothing flashes.
 */
const SPINNER_AFTER_MS = 300

const source = computed(() => page.plugin.blockDocument('widget', props.id))
const look = useLook()
const frame = ref<HTMLIFrameElement>()
/** Whether the frame's page has loaded, for the page the frame shows now. */
const loaded = ref(false)
/**
 * How long the page may take to draw its picture. A page that never answers
 * would leave the copy waiting with no word: Chrome pauses the animation
 * frames of a cross-origin frame scrolled out of view, which html-to-image
 * waits on to copy a canvas, and its library may not load at all.
 */
const PICTURE_TIMEOUT_MS = 10_000

/** The pictures asked for and not answered yet, by their request's number. */
const asked = new Map<number, { resolve: (picture: Blob) => void, reject: (error: Error) => void }>()
let lastRequest = 0
function dropAsked(): void {
  for (const waiting of asked.values())
    waiting.reject(new Error(t('picture.notLoaded')))
  asked.clear()
}
/** Whether loading has taken long enough to show the spinner. */
const slow = ref(false)
let slowTimer: ReturnType<typeof setTimeout> | undefined
watch(() => [props.id, look.value] as const, () => {
  // The page drawn again answers no picture the last one was asked for.
  dropAsked()
  loaded.value = false
  slow.value = false
  clearTimeout(slowTimer)
  slowTimer = setTimeout(() => { slow.value = true }, SPINNER_AFTER_MS)
}, { immediate: true })


/** A PNG of what the frame shows now, as its page draws it. */
function picture(): Promise<Blob> {
  const page = frame.value?.contentWindow
  if (!page || !loaded.value)
    return Promise.reject(new Error(t('picture.notLoaded')))
  const request = ++lastRequest
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => {
      asked.delete(request)
      reject(new Error(t('picture.timedOut')))
    }, PICTURE_TIMEOUT_MS)
    asked.set(request, {
      resolve: (picture) => { clearTimeout(timer); resolve(picture) },
      reject: (error) => { clearTimeout(timer); reject(error) },
    })
    page.postMessage({ widget: 'picture', request }, '*')
  })
}
defineExpose({ picture })

function onMessage(event: MessageEvent): void {
  if (event.source !== frame.value?.contentWindow)
    return
  if (event.data?.widget === 'picture') {
    const waiting = asked.get(event.data.request)
    asked.delete(event.data.request)
    if (event.data.picture instanceof Blob)
      waiting?.resolve(event.data.picture)
    else
      waiting?.reject(new Error(String(event.data.error)))
  } else if (event.data?.replyBlock === 'unavailable')
    emit('unavailable')
  else if (event.data?.widget === 'height' && typeof event.data.height === 'number')
    emit('height', event.data.height)
}
onMounted(() => addEventListener('message', onMessage))
onBeforeUnmount(() => {
  clearTimeout(slowTimer)
  removeEventListener('message', onMessage)
  dropAsked()
})
</script>

<template>
  <iframe
    ref="frame"
    :key="look"
    :name="look"
    :src="source"
    :sandbox="HTML_SANDBOX"
    :title="title"
    class="block size-full border-0 bg-transparent transition-opacity duration-200"
    :class="loaded ? 'opacity-100' : 'opacity-0'"
    @load="loaded = true"
  />
  <div
    class="pointer-events-none absolute inset-0 flex items-center justify-center text-fg-muted transition-opacity duration-200"
    :class="!loaded && slow ? 'opacity-100' : 'opacity-0'"
    aria-hidden="true"
  >
    <IndeterminateSpinner :size="18" />
  </div>
</template>
