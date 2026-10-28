<script setup lang="ts">
// One demo frame, drawn at the size of the screen it stands for (a desktop
// browser's window or a phone's screen) and scaled to the space it has, so
// the app lays itself out as it would on that screen. It loads once it nears
// the viewport, and plays the demo it is given; a frame the page hides never
// loads, and says it is hidden. The demo's camera works inside the frame
// (`src/demo/camera.ts`), and the frame says where it points.
import { onBeforeUnmount, onMounted, ref, watch } from 'vue'
import type { DemoId } from '@/features'
import { askToPlay, cameraOf, shotOf, type DeviceShot } from '@/demo/message'

const props = defineProps<{
  demo: DemoId
  form: 'desktop' | 'phone'
  width: number
  height: number
}>()
const emit = defineEmits<{
  /** The scene closed the desktop's tab, or opened it again. */
  tab: [closed: boolean]
  /** The frame's app has loaded and is playing. */
  ready: []
  /** The page hides the frame, which so never loads, or shows it again. */
  hidden: [hidden: boolean]
  /** The camera came close on a part of the screen, or showed all of it again, cutting there or moving. */
  focus: [close: boolean, cut: boolean]
  /** What the stage shows beside the app (`demo/message.ts`). */
  shot: [shot: DeviceShot | null]
}>()

const box = ref<HTMLElement | null>(null)
const frame = ref<HTMLIFrameElement | null>(null)
const scale = ref(0)
const source = ref<string | null>(null)
const ready = ref(false)
let loadedWith: DemoId | null = null
let resize: ResizeObserver | undefined
let near: IntersectionObserver | undefined

function hear(event: MessageEvent<unknown>): void {
  if (event.source !== frame.value?.contentWindow)
    return
  const data = event.data as { demi?: string; closed?: unknown } | null
  if (data?.demi === 'tab' && typeof data.closed === 'boolean')
    emit('tab', data.closed)
  const shot = cameraOf(data)
  if (shot !== undefined)
    emit('focus', shot.rect !== null, shot.cut)
  const device = shotOf(data)
  if (device !== undefined)
    emit('shot', device)
  if (data?.demi !== 'ready')
    return
  ready.value = true
  emit('ready')
  if (props.demo !== loadedWith)
    askToPlay(event.source as Window, props.demo)
}

watch(() => props.demo, (demo) => {
  const target = frame.value?.contentWindow
  if (ready.value && target)
    askToPlay(target, demo)
})

onMounted(() => {
  const element = box.value!
  resize = new ResizeObserver(() => {
    scale.value = element.clientWidth / props.width
    emit('hidden', element.clientWidth === 0)
  })
  resize.observe(element)
  near = new IntersectionObserver((entries) => {
    if (!entries.some((entry) => entry.isIntersecting))
      return
    near?.disconnect()
    loadedWith = props.demo
    source.value = `${import.meta.env.BASE_URL}demo.html?demo=${props.demo}&form=${props.form}`
  }, { rootMargin: '600px 0px' })
  near.observe(element)
  window.addEventListener('message', hear)
})

onBeforeUnmount(() => {
  resize?.disconnect()
  near?.disconnect()
  window.removeEventListener('message', hear)
})
</script>

<template>
  <div ref="box" class="box" :style="{ aspectRatio: `${width} / ${height}` }">
    <iframe
      v-if="source"
      ref="frame"
      class="frame"
      :class="{ shown: ready }"
      :src="source"
      :width="width"
      :height="height"
      :style="{ transform: `scale(${scale})` }"
      title=""
      tabindex="-1"
      aria-hidden="true"
      loading="lazy"
    />
  </div>
</template>

<style scoped>
.box {
  position: relative;
  width: 100%;
  overflow: hidden;
}

.frame {
  position: absolute;
  top: 0;
  left: 0;
  border: 0;
  opacity: 0;
  transform-origin: 0 0;
  pointer-events: none;
  transition: opacity 0.3s ease;
}

.frame.shown {
  opacity: 1;
}
</style>
