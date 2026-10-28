<script setup lang="ts">
// A pale ornament across the border between two screens, in the empty band
// between the content of the screen above and the content of the screen
// below, reaching in from one side of the window, which cuts it. It never
// lies under content: it takes the band's height less a margin on each
// side, keeps its proportions, and stays hidden when the band is too low
// for it or the window too narrow. Every ornament is a slim line with an object at its tip, and
// all show at one length: the longest every band holds, at most half the
// window. No stylesheet can measure the bands, so the component measures
// the screens.
import { onBeforeUnmount, onMounted, ref, useId } from 'vue'

const props = defineProps<{
  src1x: string
  src2x: string
  /** The 2x image's size, for its proportions. */
  width: number
  height: number
  side: 'left' | 'right'
}>()

/** The space kept clear between an ornament and any content. */
const CLEAR = 24
/** A shorter ornament is too small to read; one whose band cannot hold it hides. */
const MIN_LENGTH = 280
/** The longest an ornament reaches across the window. */
const MAX_SHARE = 0.5
/** A window narrower than this hides every scene, as the install screen's. */
const MIN_WINDOW = 900

const id = useId()
const root = ref<HTMLElement | null>(null)
/** The band between the two screens' content, from the border, and the longest ornament it holds; null while it has none. */
const band = ref<{ from: number; to: number; holds: number } | null>(null)
/** The ornament's height for each pixel of its length. */
const ratio = props.height / props.width
let observer: ResizeObserver | undefined

/** Where the ornament lies, from the border, or null while it is hidden. */
const shown = computed(() => {
  const length = sharedLength.value
  if (!band.value || length === null || band.value.holds < length)
    return null
  const height = length * ratio
  return { top: band.value.from + (band.value.to - band.value.from - height) / 2, height }
})

/** Whether an element is something a reader sees: text, a picture or a filled or outlined box. */
function visible(element: Element): boolean {
  if (element.closest('[data-decor]'))
    return false
  const style = getComputedStyle(element)
  if (style.visibility === 'hidden' || style.display === 'none')
    return false
  return (element.childElementCount === 0 && (element.textContent ?? '').trim() !== '')
    || ['IMG', 'IFRAME', 'svg'].includes(element.tagName)
    || style.backgroundColor !== 'rgba(0, 0, 0, 0)'
    || style.borderTopWidth !== '0px'
    || style.boxShadow !== 'none'
}

/** The top or bottom edge of a screen's content, in page coordinates. */
function edge(screen: Element, which: 'top' | 'bottom'): number {
  let found = which === 'top' ? Infinity : -Infinity
  for (const element of screen.querySelectorAll('*')) {
    const rect = element.getBoundingClientRect()
    if (rect.width < 2 || rect.height < 2 || !visible(element))
      continue
    found = which === 'top' ? Math.min(found, rect.top) : Math.max(found, rect.bottom)
  }
  return found + scrollY
}

function measure(): void {
  const element = root.value
  const above = element?.previousElementSibling
  const below = element?.nextElementSibling
  if (!element || !above || !below || innerWidth < MIN_WINDOW) {
    band.value = null
    holds.delete(id)
    return
  }
  const border = element.getBoundingClientRect().top + scrollY
  const from = edge(above, 'bottom') + CLEAR - border
  const to = edge(below, 'top') - CLEAR - border
  // Its band's height and the window's width bound how long it can be.
  const most = Math.min((to - from) / ratio, innerWidth * MAX_SHARE)
  band.value = { from, to, holds: most }
  if (most >= MIN_LENGTH)
    holds.set(id, most)
  else
    holds.delete(id)
}

onMounted(() => {
  const element = root.value!
  // A screen keeps its height while its content settles (fonts, a demo
  // loading, the tour's texts mounting), so the observer watches the
  // screens' parts as well as the screens.
  observer = new ResizeObserver(measure)
  for (const screen of [element.previousElementSibling, element.nextElementSibling]) {
    if (!screen)
      continue
    observer.observe(screen)
    for (const part of screen.querySelectorAll(':scope > *, :scope > * > *'))
      observer.observe(part)
  }
  void document.fonts.ready.then(measure)
  measure()
})

onBeforeUnmount(() => {
  observer?.disconnect()
  holds.delete(id)
})
</script>

<script lang="ts">
import { computed, reactive } from 'vue'

/** The longest ornament each shown ornament's band holds, by ornament. */
const holds = reactive(new Map<string, number>())
/** The one length every shown ornament takes: what the tightest band holds. */
const sharedLength = computed(() => (holds.size ? Math.min(...holds.values()) : null))
</script>

<template>
  <div ref="root" class="interlude" aria-hidden="true">
    <img
      v-if="shown"
      data-decor
      :class="side"
      :src="src1x"
      :srcset="`${src1x} ${width / 2}w, ${src2x} ${width}w`"
      :sizes="`${Math.round((shown.height * width) / height)}px`"
      alt=""
      :width="width"
      :height="height"
      :style="{ top: `${shown.top}px`, height: `${shown.height}px` }"
    />
  </div>
</template>

<style scoped>
.interlude {
  position: relative;
  height: 0;
}

img {
  position: absolute;
  width: auto;
  opacity: 0.25;
  pointer-events: none;
  user-select: none;
}

.left {
  left: 0;
}

.right {
  right: 0;
}
</style>
