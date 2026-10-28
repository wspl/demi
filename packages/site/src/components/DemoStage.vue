<script setup lang="ts">
// A demo's demo: Demi in a desktop browser window, and on a phone in front
// of it, showing the same conversation. The two appear together, once both
// apps have loaded, rather than as empty frames first; a narrow screen hides
// the desktop, and the phone appears alone once it has loaded.
import { ChevronLeftIcon, ChevronRightIcon, CopyIcon, LockIcon, PanelLeftIcon, PlusIcon, ShareIcon, SignalIcon, WifiIcon, BatteryFullIcon } from '@lucide/vue'
import type { DemoId } from '@/features'
import { computed, ref, watch } from 'vue'
import DemoFrame from './DemoFrame.vue'
import { CAMERA_MS } from '@/demo/message'

const props = defineProps<{
  demo: DemoId
  /** What the demo shows, for a screen reader. */
  label: string
}>()
/** Whether the desktop demo closed its tab, which leaves the address bar empty. */
const closed = ref(false)
watch(() => props.demo, () => (closed.value = false))
const desktopReady = ref(false)
const desktopHidden = ref(false)
const phoneReady = ref(false)
const shown = computed(() => phoneReady.value && (desktopReady.value || desktopHidden.value))
/**
 * Whether the desktop's camera is close on a part of its screen: the phone
 * steps out of the way meanwhile, as it would cover the screen's right side.
 */
const focused = ref(false)
/** Whether the phone goes or comes at once, as the camera cuts. */
const cut = ref(false)
function focus(close: boolean, at: boolean): void {
  cut.value = at
  focused.value = close
}
</script>

<template>
  <div class="stage" :class="{ shown }">
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
      <DemoFrame :demo="demo" form="desktop" :width="1024" :height="614" @focus="focus" @tab="closed = $event" @ready="desktopReady = true" @hidden="desktopHidden = $event" />
    </div>
    <div class="phone" :class="{ away: focused, cut }" :style="{ '--camera': `${CAMERA_MS}ms` }" aria-hidden="true">
      <div class="phone-screen">
        <DemoFrame :demo="demo" form="phone" :width="393" :height="852" @ready="phoneReady = true" />
        <span class="status">
          <b>9:41</b>
          <span class="status-icons"><SignalIcon :size="15" :stroke-width="2.6" /><WifiIcon :size="15" :stroke-width="2.6" /><BatteryFullIcon :size="19" :stroke-width="2" /></span>
        </span>
        <span class="island" />
      </div>
    </div>
    <p class="visually-hidden">{{ label }}</p>
  </div>
</template>

<style scoped>
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
