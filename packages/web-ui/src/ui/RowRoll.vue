<script setup lang="ts">
import { nextTick, onBeforeUnmount, ref, watch } from 'vue'
import { CHROME_ROLL_EASING, CHROME_ROLL_MS } from './chrome-roll'

/**
 * A row of a list that rolls what it shows over what it showed when its face
 * changes, at any height: the old face moves up and out as the new one comes
 * up from under it into its place, the roll a step makes over the one before
 * (`ChromeRoll`, whose 28px face keeps its icon standing while only the label
 * rolls). A row that takes another's place rolls over it this way: Stopped
 * into Requesting, Requesting into the step it waited for or the answer
 * (`product.md` § Following the agent's work). A row mounted with its face
 * does not roll.
 */
const props = defineProps<{
  /** When this changes, the row rolls. */
  faceKey: string
}>()

const root = ref<HTMLElement>()
const face = ref<HTMLElement>()
const rolling = ref(false)
/** The copy of the old face moving out, and the animations under way. */
let outgoing: HTMLElement | null = null
let animations: Animation[] = []
/** Counts the changes of face, so only the latest one rolls when two come before a render. */
let changes = 0

function finish(): void {
  // A roll cancelled by the next one or by unmounting ends here too.
  for (const animation of animations) {
    animation.cancel()
  }
  animations = []
  outgoing?.remove()
  outgoing = null
  rolling.value = false
}

// The snapshot is taken from the DOM still showing the old face: this runs
// before the row renders the new one.
watch(() => props.faceKey, () => {
  const old = face.value
  if (!old) {
    return
  }
  const change = ++changes
  const height = old.offsetHeight
  const copy = old.cloneNode(true) as HTMLElement
  // The copy is a picture of the old face: nothing in it is read, focused or clicked, and no id is there twice.
  copy.inert = true
  copy.setAttribute('aria-hidden', 'true')
  copy.querySelectorAll('[id]').forEach((element) => element.removeAttribute('id'))
  copy.style.cssText = 'position:absolute;top:0;left:0;right:0;pointer-events:none'
  void nextTick(() => {
    const host = root.value
    const current = face.value
    if (!host || !current || change !== changes) {
      return
    }
    finish()
    outgoing = copy
    host.append(copy)
    rolling.value = true
    const timing: KeyframeAnimationOptions = { duration: CHROME_ROLL_MS, easing: CHROME_ROLL_EASING }
    animations = [
      copy.animate([{ transform: 'translateY(0)' }, { transform: `translateY(${-height}px)` }], timing),
      current.animate([{ transform: `translateY(${height}px)` }, { transform: 'translateY(0)' }], timing),
    ]
    // A shorter face narrows the row as it rolls in, rather than cutting the
    // old face off in the first frame; a taller one is the row's height at once,
    // its face rising into it.
    const shorter = current.offsetHeight
    if (shorter < height) {
      animations.push(host.animate([{ height: `${height}px` }, { height: `${shorter}px` }], timing))
    }
    // A cancelled roll rejects `finished`; the roll that cancelled it cleans up.
    animations[1]!.finished.then(finish, () => {})
  })
}, { flush: 'pre' })

onBeforeUnmount(() => {
  changes += 1
  finish()
})
</script>

<template>
  <!-- Clipped only while it rolls, so a face's focus ring or shadow is never cut. -->
  <div ref="root" class="relative" :class="rolling ? 'overflow-hidden' : ''">
    <div ref="face">
      <slot />
    </div>
  </div>
</template>
