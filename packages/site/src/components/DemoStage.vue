<script setup lang="ts">
// A demo's demo: Demi in a desktop browser window, and on a phone in front
// of it, showing the same conversation. The two appear together, once both
// apps have loaded, rather than as empty frames first; a narrow screen hides
// the desktop, and the phone appears alone once it has loaded.
import { ChevronLeftIcon, ChevronRightIcon, CopyIcon, LockIcon, PanelLeftIcon, PlusIcon, ShareIcon, SignalIcon, WifiIcon, BatteryFullIcon } from '@lucide/vue'
import type { DemoId } from '@/features'
import { computed, ref, watch } from 'vue'
import DemoFrame from './DemoFrame.vue'
import DeviceShots from './DeviceShots.vue'
import { CAMERA_MS, type DeviceShot } from '@/demo/message'

const props = defineProps<{
  demo: DemoId
  /** What the demo shows, for a screen reader. */
  label: string
  /**
   * Whether the stage shows the machines a scene works on (DeviceShots), and
   * the browser and the phone go down to make room for them and come back
   * up, as the hero's does.
   */
  lifts?: boolean
}>()
/** Whether the desktop demo closed its tab, which leaves the address bar empty. */
const closed = ref(false)
/** What the stage shows beside the app, as the desktop's demo says. */
const shot = ref<DeviceShot | null>(null)
watch(() => props.demo, () => {
  closed.value = false
  shot.value = null
})
const desktopReady = ref(false)
const desktopHidden = ref(false)
const phoneReady = ref(false)
const shown = computed(() => phoneReady.value && (desktopReady.value || desktopHidden.value))
/**
 * Whether the desktop's camera is close on a part of its screen: the phone
 * steps out of the way meanwhile, as it would cover the screen's right side.
 */
const focused = ref(false)
/**
 * Whether the phone is up on a stage that lifts: only away from the desk,
 * where it leads, and in the overview; at the desk the machines stand at the
 * browser's corner instead. A narrow screen, which hides the desktop, shows
 * the phone alone all the time.
 */
const phoneLeads = computed(() => (shot.value?.kind === 'close' && shot.value.beside === 'phone') || shot.value?.kind === 'all')
/** Whether the phone goes or comes at once, as the camera cuts. */
const cut = ref(false)
function focus(close: boolean, at: boolean): void {
  cut.value = at
  focused.value = close
}
</script>

<template>
  <div class="stage" :class="{ shown, lifts, 'shot-desk': shot?.kind === 'close' && shot.beside === 'browser', 'shot-away': shot?.kind === 'close' && shot.beside === 'phone', 'shot-every': shot?.kind === 'all', 'shot-clear': shot?.kind === 'clear' }">
    <div class="browser">
      <div class="chrome">
        <span class="lights" aria-hidden="true"><i /><i /><i /></span>
        <span class="tools" aria-hidden="true">
          <ChevronLeftIcon :size="20" /><ChevronRightIcon :size="20" /><PanelLeftIcon :size="18" />
        </span>
        <span class="address"><template v-if="!closed"><LockIcon :size="13" />demi.example.com</template><template v-else>Search or enter website name</template></span>
        <span class="tools end" aria-hidden="true">
          <ShareIcon :size="18" /><PlusIcon :size="20" /><CopyIcon :size="18" />
        </span>
      </div>
      <DemoFrame :demo="demo" form="desktop" :width="1024" :height="614" @focus="focus" @tab="closed = $event" @shot="shot = $event" @ready="desktopReady = true" @hidden="desktopHidden = $event" />
    </div>
    <div class="phone" :class="{ away: lifts ? !(phoneLeads || desktopHidden) : focused, cut }" :style="{ '--camera': `${CAMERA_MS}ms` }" aria-hidden="true">
      <div class="phone-screen">
        <DemoFrame :demo="demo" form="phone" :width="393" :height="852" @ready="phoneReady = true" />
        <span class="status">
          <b>9:41</b>
          <span class="status-icons"><SignalIcon :size="15" :stroke-width="2.6" /><WifiIcon :size="15" :stroke-width="2.6" /><BatteryFullIcon :size="19" :stroke-width="2" /></span>
        </span>
        <span class="island" />
      </div>
    </div>
    <DeviceShots v-if="lifts" :shot="shot" />
    <p class="visually-hidden">{{ label }}</p>
  </div>
</template>

<style scoped>
/*
 * On a stage that lifts, as the hero's, the browser and the phone make room
 * for the machines (DeviceShots) and come back, each fading as it moves: the
 * browser goes off to the left, and the phone comes up from below, as from a
 * pocket. At the desk a machine stands at the browser's corner. Away from
 * the desk the browser goes and the phone comes up on the left, larger, to
 * lead; the machines come in on its right. Every machine at the end: the app
 * shrinks to the left, where the wires begin.
 */
.lifts {
  --rise: cubic-bezier(0.16, 1, 0.3, 1);
  --sink: cubic-bezier(0.55, 0, 0.75, 0);
}

.lifts .browser,
.lifts .phone {
  will-change: transform, opacity;
  transition:
    transform 0.75s var(--rise) 0.3s,
    opacity 0.45s ease-out 0.3s,
    visibility 0s 0.3s;
}

.lifts.shot-away .browser,
.lifts .phone.away {
  visibility: hidden;
  opacity: 0;
  transform: translateX(-5rem);
  transition:
    transform 0.4s var(--sink),
    opacity 0.3s ease-in 0.05s,
    visibility 0s 0.4s;
}

/* The phone stays down where it leads from, so it comes straight up there. */
.lifts .phone {
  transform-origin: center bottom;
}

.lifts .phone.away {
  transform: translate(-254%, 5rem) scale(1.12);
}

/* The browser recedes to the left, as small as the overview shows it, so it comes back there in place. */
.lifts.shot-away .browser {
  transform: translateX(-5rem) scale(0.5);
}

.lifts.shot-away .phone {
  transform: translateX(-254%) scale(1.12);
}

.shot-every .browser {
  transform: scale(0.5);
}

.shot-every .phone {
  transform-origin: right bottom;
  transform: translate(-212%, -24%) scale(0.5);
}

.lifts.shot-every .phone {
  transform-origin: center bottom;
  transform: translate(-187%, -24%) scale(0.5);
}

.lifts.shot-every .browser,
.lifts.shot-every .phone {
  transition:
    transform 0.9s cubic-bezier(0.65, 0, 0.35, 1),
    opacity 0.5s ease-out;
}

/*
 * Clearing the stage as a loop starts over, the browser and the phone fade
 * where they stand, then move out of sight to where they come back from.
 */
.lifts.shot-clear .browser {
  visibility: hidden;
  opacity: 0;
  transform: translateX(-5rem);
}

.lifts.shot-clear .browser,
.lifts.shot-clear .phone.away {
  transition:
    opacity 0.35s ease-in,
    transform 0s 0.35s,
    visibility 0s 0.35s;
}

.stage {
  position: relative;
  padding-right: 13%;
  padding-bottom: 4%;
  opacity: 0;
  transition: opacity 0.4s ease;
}

.stage.shown {
  opacity: 1;
}

.browser {
  overflow: hidden;
  transform-origin: left center;
  transition:
    opacity 0.5s ease,
    transform 0.8s cubic-bezier(0.45, 0, 0.2, 1);
  border: 1px solid var(--line);
  border-radius: 0.875rem;
  background: var(--paper);
  box-shadow: 0 2px 0.25rem rgb(60 40 20 / 5%), 0 1.5rem 3.75rem rgb(60 40 20 / 12%);
}

.chrome {
  display: grid;
  grid-template-columns: auto auto 1fr auto;
  gap: 1.125rem;
  align-items: center;
  height: 3.25rem;
  padding: 0 1.125rem;
  border-bottom: 1px solid var(--line);
  background: #f7f3ee;
  color: #6f6660;
}

.lights {
  display: flex;
  gap: 0.5rem;
}

.lights i {
  width: 0.75rem;
  height: 0.75rem;
  border-radius: 50%;
}

.lights i:nth-child(1) {
  background: #ee6a5f;
}

.lights i:nth-child(2) {
  background: #f5bd4f;
}

.lights i:nth-child(3) {
  background: #61c454;
}

.tools {
  display: flex;
  gap: 0.875rem;
  align-items: center;
}

.address {
  display: inline-flex;
  gap: 0.4375rem;
  align-items: center;
  justify-content: center;
  justify-self: center;
  width: min(100%, 27.5rem);
  padding: 0.375rem 0;
  border-radius: 0.5rem;
  background: #ece6de;
  color: var(--ink);
  font-size: 0.875rem;
}


/* The phone's sizes are in its own width (cqw), so its bezel stays as thin as an iPhone's at any size. */
.phone {
  position: absolute;
  right: 0;
  bottom: 0;
  width: 25%;
  border-radius: 16% / 7.4%;
  background: #1c1918;
  box-shadow: 0 1.25rem 2.75rem rgb(40 25 15 / 22%), inset 0 0 0 1.5px #4a4442;
  container-type: inline-size;
  transition:
    opacity 0.6s ease,
    transform var(--camera) cubic-bezier(0.45, 0, 0.2, 1);
}

.phone.cut {
  transition: none;
}

.phone.away {
  opacity: 0;
  transform: translateY(12%);
}

.phone-screen {
  position: relative;
  margin: 3cqw;
  overflow: hidden;
  border-radius: 13cqw;
  background: var(--paper);
}

.island {
  position: absolute;
  top: 2.6cqw;
  left: 50%;
  width: 31%;
  height: 8.6cqw;
  border-radius: 62.4375rem;
  background: #1c1918;
  transform: translateX(-50%);
}

/* The status bar over the phone's demo, which keeps its height clear. */
.status {
  position: absolute;
  top: 2.6cqw;
  right: 0;
  left: 0;
  display: flex;
  height: 8.6cqw;
  align-items: center;
  justify-content: space-between;
  padding: 0 9% 0 13%;
  color: var(--ink);
  font-size: 4.6cqw;
  pointer-events: none;
}

.status-icons :deep(svg) {
  width: 4.2cqw;
  height: 4.2cqw;
}

.status-icons :deep(svg:last-child) {
  width: 5.6cqw;
  height: 5.6cqw;
}

.status-icons {
  display: flex;
  gap: 0.25rem;
  align-items: center;
}

/* A narrow screen shows the phone alone: the desktop's window would be too small to read. */
@media (max-width: 720px) {
  .stage {
    padding: 0;
  }

  .browser {
    display: none;
  }

  .phone {
    position: relative;
    width: min(12.25rem, 58%);
    margin: 0 auto;
  }

}
</style>
