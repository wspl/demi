<script setup lang="ts">
// The stage of a demo that plays on several devices at once: a laptop's
// browser, a tablet and a phone, each a frame of its own playing the same
// conversation on its screen's size. The scene says which show
// (`DevicesShot`), and each comes and goes from its own side as it fades:
// the phone from the right, pushing the browser aside, the tablet from the
// left as the browser goes up, the browser back down from above as the two
// mobile ones go down. Every screen
// follows what is typed on any of them. The three play from their start
// together, once all have loaded and the stage shows, and pause while it is
// hidden or off screen.
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { useOnScreen } from '@/onScreen'
import { BatteryFullIcon, LaptopIcon, LockIcon, SignalIcon, SmartphoneIcon, TabletIcon, WifiIcon } from '@lucide/vue'
import { FORMS, type DemoId } from '@/features'
import DeviceLogo from '@demicodes/web-ui/devices/DeviceLogo.vue'
import type { SystemLogo } from '@demicodes/web-ui/devices/mark'
import type { DevicesShot, ShotDevice, StageCard } from '@/demo/message'
import DemoFrame from './DemoFrame.vue'

const props = defineProps<{
  demo: DemoId
  /** What the demo shows, for a screen reader. */
  label: string
  /** Whether the stage shows: each time it does, the demo plays from its start. */
  active: boolean
}>()

type Device = 'laptop' | 'tablet' | 'phone'
const DEVICES: readonly Device[] = ['laptop', 'tablet', 'phone']
const NAMES: Record<Device, string> = { laptop: 'MacBook', tablet: 'iPad', phone: 'iPhone' }
const ICONS = { laptop: LaptopIcon, tablet: TabletIcon, phone: SmartphoneIcon }

const shot = ref<DevicesShot>('browser')
const run = ref(0)
/** Whether the devices take their places at once, as a play starts or the stage takes its size. */
const settling = ref(true)
function play(): void {
  settling.value = true
  shot.value = 'browser'
  machines.value = []
  using.value = null
  latest.value = null
  run.value += 1
  requestAnimationFrame(() => requestAnimationFrame(() => (settling.value = false)))
}
/**
 * Whether the browser stands, unseen and a little smaller, in its place in
 * the middle before it comes back: it left from where the phone had pushed
 * it, and comes back where it stands, growing as it fades in.
 */
const returning = ref(false)
watch(shot, (next) => {
  if (next !== 'back')
    return
  returning.value = true
  requestAnimationFrame(() => requestAnimationFrame(() => (returning.value = false)))
})
/** The machines the conversation has reached, in the order it reached them, and the one its work runs on now. */
const machines = ref<ShotDevice[]>([])
const using = ref<ShotDevice | null>(null)
/** The machine a line too narrow for every tag shows alone: the one at work now, or the one last at work. */
const latest = ref<ShotDevice | null>(null)
watch(using, (machine) => {
  if (machine)
    latest.value = machine
})
/**
 * The devices too narrow under them for every machine's tag, which show the
 * latest machine alone, by its logo and its light.
 */
const NARROW: ReadonlySet<Device> = new Set(['phone'])
/** Each machine as its tag names it, with its system's logo. */
const MACHINE_TAGS: Record<ShotDevice, { name: string; logo: SystemLogo }> = {
  'macbook': { name: 'MacBook', logo: 'apple' },
  'windows-pc': { name: 'Windows PC', logo: 'windows' },
  'home-lab': { name: 'Home Lab', logo: 'debian' },
  'vps': { name: 'VPS', logo: 'ubuntu' },
}

/** The laptop's screen says how to frame the devices; the others play the same and say nothing. */
function hear(card: StageCard | null): void {
  if (card?.kind !== 'devices' || !props.active)
    return
  shot.value = card.shot
  machines.value = card.machines
  using.value = card.using
}

const ready = ref(new Set<Device>())
const shown = computed(() => ready.value.size === DEVICES.length && props.active)
watch(shown, (now) => {
  if (now)
    play()
})

const box = ref<HTMLElement | null>(null)
const onScreen = useOnScreen(box)
const paused = computed(() => !props.active || !onScreen.value)
const size = ref({ w: 0, h: 0 })
let observer: ResizeObserver | undefined
onMounted(() => {
  observer = new ResizeObserver(([entry]) => {
    settling.value = true
    size.value = { w: entry!.contentRect.width, h: entry!.contentRect.height }
    requestAnimationFrame(() => requestAnimationFrame(() => (settling.value = false)))
  })
  observer.observe(box.value!)
})
onBeforeUnmount(() => observer?.disconnect())

/** The laptop's browser bar, as tall as the other features' browser's. */
const BAR = 52
/** Each device at its largest, as a share of the stage's width: the laptop as wide as the other features' browser. */
const BASE: Record<Device, number> = { laptop: 0.87, tablet: 0.36, phone: 0.21 }
/** A tablet's and a phone's edge around the screen, as a share of the device's width. */
const EDGE = { tablet: 0.035, phone: 0.03 }

/** Each device's size at its largest, in pixels. */
const bases = computed(() => {
  const w = size.value.w
  const laptop = BASE.laptop * w
  const tablet = BASE.tablet * w
  const phone = BASE.phone * w
  return {
    laptop: { w: laptop, h: (laptop * FORMS.desktop.height) / FORMS.desktop.width + BAR },
    tablet: { w: tablet, h: ((tablet * (1 - 2 * EDGE.tablet)) * FORMS.tablet.height) / FORMS.tablet.width + 2 * EDGE.tablet * tablet },
    phone: { w: phone, h: ((phone * (1 - 2 * EDGE.phone)) * FORMS.phone.height) / FORMS.phone.width + 2 * EDGE.phone * phone },
  }
})

/** Where a device stands in a shot: its top left corner in pixels, its scale from its largest, whether it shows, and whether it is washed back. */
interface Place {
  x: number
  y: number
  scale: number
  shows: boolean
  back?: boolean
}

const places = computed<Record<Device, Place>>(() => {
  const { w, h } = size.value
  const b = bases.value
  const middle = (device: Device, scale: number) => (h - b[device].h * scale) / 2
  /** The tablet and the phone side by side in the middle, the tablet on the left. */
  const gap = w * 0.05
  const pairLeft = (w - b.tablet.w - gap - b.phone.w) / 2
  const pair = {
    tablet: { x: pairLeft, y: middle('tablet', 1), scale: 1, shows: true },
    phone: { x: pairLeft + b.tablet.w + gap, y: middle('phone', 1), scale: 1, shows: true },
  }
  const browser = { x: (w - b.laptop.w) / 2, y: 0, scale: 1, shows: true }
  /**
   * The phone taking over beside the browser, which it has pushed aside to
   * the left and made smaller: the two stand a gap apart, centered together,
   * each on the stage's middle line.
   */
  const aside = 0.72
  const takingLeft = (w - b.laptop.w * aside - gap - b.phone.w) / 2
  const taking = {
    laptop: { x: takingLeft, y: middle('laptop', aside), scale: aside, shows: true, back: true },
    phone: { x: takingLeft + b.laptop.w * aside + gap, y: middle('phone', 1), scale: 1, shows: true },
  }
  /** Where each comes in from and goes to: the phone a step to the right, the tablet to the left, the browser above, the two mobile ones below. */
  const fromRight = { ...taking.phone, x: taking.phone.x + w * 0.16, shows: false }
  const fromLeft = { ...pair.tablet, x: pair.tablet.x - w * 0.16, shows: false }
  switch (shot.value) {
    case 'browser':
      return { laptop: browser, phone: fromRight, tablet: fromLeft }
    case 'phone':
      return { ...taking, tablet: fromLeft }
    case 'both':
      return { laptop: { ...taking.laptop, y: -h * 0.04, shows: false }, ...pair }
    case 'back':
      return {
        laptop: returning.value ? { x: browser.x + (b.laptop.w * 0.08) / 2, y: (b.laptop.h * 0.08) / 2, scale: 0.92, shows: false } : browser,
        phone: { ...pair.phone, y: h * 0.2 + pair.phone.y, shows: false },
        tablet: { ...pair.tablet, y: h * 0.2 + pair.tablet.y, shows: false },
      }
  }
})

function style(device: Device): Record<string, string> {
  const place = places.value[device]
  return {
    width: `${bases.value[device].w}px`,
    transform: `translate(${place.x}px, ${place.y}px) scale(${place.scale})`,
    '--scale': String(place.scale),
  }
}
</script>

<template>
  <div ref="box" class="anywhere" :class="[`shot-${shot}`, { shown, settling, paused }]" :aria-hidden="!active">
    <div
      v-for="device in DEVICES"
      :key="device"
      class="device"
      :class="[device, { shows: places[device].shows, back: places[device].back, placing: device === 'laptop' && returning }]"
      :style="style(device)"
      aria-hidden="true"
    >
      <div v-if="device === 'laptop'" class="window">
        <div class="bar">
          <span class="lights"><i /><i /><i /></span>
          <span class="address"><LockIcon :size="13" />demi.example.com</span>
        </div>
        <DemoFrame :demo="demo" form="desktop" :width="FORMS.desktop.width" :height="FORMS.desktop.height" :run="run" :paused="paused" @ready="ready = new Set([...ready, 'laptop'])" @card="hear" />
      </div>
      <div v-else-if="device === 'tablet'" class="slab tablet-body">
        <div class="glass">
          <DemoFrame :demo="demo" form="tablet" :width="FORMS.tablet.width" :height="FORMS.tablet.height" :run="run" :paused="paused" @ready="ready = new Set([...ready, 'tablet'])" />
        </div>
      </div>
      <div v-else class="slab phone-body">
        <div class="glass">
          <DemoFrame :demo="demo" form="phone" :width="FORMS.phone.width" :height="FORMS.phone.height" :run="run" :paused="paused" @ready="ready = new Set([...ready, 'phone'])" />
          <span class="status">
            <b>9:41</b>
            <span class="status-icons"><SignalIcon :size="15" :stroke-width="2.6" /><WifiIcon :size="15" :stroke-width="2.6" /><BatteryFullIcon :size="19" :stroke-width="2" /></span>
          </span>
          <span class="island" />
        </div>
      </div>
      <div class="caption">
        <span class="name"><component :is="ICONS[device]" :size="15" :stroke-width="1.8" />{{ NAMES[device] }}</span>
        <TransitionGroup name="tag" tag="span" class="tags">
          <span v-for="machine in NARROW.has(device) ? machines.filter((each) => each === latest) : machines" :key="machine" class="tag" :class="{ using: using === machine, compact: NARROW.has(device) }" :title="MACHINE_TAGS[machine].name">
            <i class="light" />
            <DeviceLogo :logo="MACHINE_TAGS[machine].logo" :size="12" />
            <span class="tag-name"><span>{{ MACHINE_TAGS[machine].name }}</span></span>
          </span>
        </TransitionGroup>
      </div>
    </div>
    <p class="visually-hidden">{{ label }}</p>
  </div>
</template>

<style scoped>
/* The tags' endless lights stop with the demo. */
.paused :deep(*) {
  animation-play-state: paused;
}

/*
 * The stage loads its screens out of sight while another feature shows, so
 * it is ready the moment its own is chosen. The other stage goes first and
 * this one comes after, as their browsers stand in different places.
 */
.anywhere {
  position: absolute;
  inset: 0;
  visibility: hidden;
  opacity: 0;
  pointer-events: none;
  transition: opacity 0.3s ease, visibility 0s 0.3s;
}

.anywhere.shown {
  visibility: visible;
  opacity: 1;
  transition: opacity 0.3s ease 0.25s;
}

/*
 * The device's scale, which its caption undoes to keep its size: a number
 * the browser can ease, so the caption moves with the device in the same
 * motion rather than jumping to its end.
 */
@property --scale {
  syntax: "<number>";
  inherits: true;
  initial-value: 1;
}

/*
 * A device comes and goes in one eased motion as it fades, a short way from
 * its own side, and one that steps aside is washed back. Nothing clips the
 * stage, so the devices keep their edges and their shadows whole.
 */
.device {
  position: absolute;
  top: 0;
  left: 0;
  transform-origin: 0 0;
  opacity: 0;
  transition:
    transform 0.9s cubic-bezier(0.65, 0, 0.25, 1),
    --scale 0.9s cubic-bezier(0.65, 0, 0.25, 1),
    opacity 0.6s ease;
}

.device.shows {
  opacity: 1;
}

/*
 * One that goes leaves quickly, and one that comes in where another goes
 * waits for it a moment, so the two never show through each other.
 */
.device:not(.shows) {
  transition:
    transform 0.6s cubic-bezier(0.55, 0, 0.75, 0),
    --scale 0.6s cubic-bezier(0.55, 0, 0.75, 0),
    opacity 0.35s ease-in;
}

/*
 * One that comes in where another has just gone waits until it has, and
 * comes in decelerating, as one set down rather than pushed: the tablet
 * after the browser, the browser after the two mobile ones.
 */
.shot-both .tablet.shows,
.shot-back .laptop.shows {
  transition:
    transform 0.8s cubic-bezier(0.16, 1, 0.3, 1) 0.45s,
    --scale 0.8s cubic-bezier(0.16, 1, 0.3, 1) 0.45s,
    opacity 0.5s ease-out 0.45s;
}

.laptop {
  z-index: 0;
}

.phone {
  z-index: 2;
}

.tablet {
  z-index: 1;
}

.settling .device,
.device.placing {
  transition: none;
}

/* The browser the phone has pushed aside fades back; it stands apart from the phone, so nothing shows through it. */
.device.shows.back {
  opacity: 0.45;
}

.device.shows {
  transition:
    transform 0.9s cubic-bezier(0.65, 0, 0.25, 1),
    --scale 0.9s cubic-bezier(0.65, 0, 0.25, 1),
    opacity 0.6s ease;
}

/* The laptop's browser, as the other features' browser is drawn. */
.window {
  overflow: hidden;
  border: 1px solid var(--line);
  border-radius: 0.875rem;
  background: var(--paper);
  box-shadow: 0 2px 0.25rem rgb(60 40 20 / 5%), 0 1.5rem 3.75rem rgb(60 40 20 / 12%);
}

.bar {
  display: grid;
  grid-template-columns: auto 1fr;
  align-items: center;
  height: 52px;
  padding: 0 1.125rem;
  border-bottom: 1px solid var(--line);
  background: #f7f3ee;
}

.lights {
  display: flex;
  gap: 0.5rem;
}

.lights i {
  width: 0.75rem;
  height: 0.75rem;
  border-radius: 50%;
  background: #ee6a5f;
}

.lights i:nth-child(2) {
  background: #f5bd4f;
}

.lights i:nth-child(3) {
  background: #61c454;
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

/*
 * A tablet's and a phone's body. The screen's sizes are in the body's width
 * (cqw), so the edge stays thin at any size; the body's corners are in its
 * own shares, as an iPhone's are.
 */
.slab {
  position: relative;
  background: #1c1918;
  box-shadow: 0 1.25rem 2.75rem rgb(40 25 15 / 22%), inset 0 0 0 1.5px #4a4442;
  container-type: inline-size;
}

.tablet-body {
  border-radius: 6% / 4.5%;
}

.phone-body {
  border-radius: 16% / 7.4%;
}

.glass {
  position: relative;
  overflow: hidden;
  background: var(--paper);
}

.tablet-body .glass {
  margin: 3.5cqw;
  border-radius: 3cqw;
}

.phone-body .glass {
  margin: 3cqw;
  border-radius: 13cqw;
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

.status-icons {
  display: flex;
  gap: 0.25rem;
  align-items: center;
}

.status-icons :deep(svg) {
  width: 4.2cqw;
  height: 4.2cqw;
}

/*
 * Under each device, a line the same size whatever the device's scale: its
 * name on the left, and on the right the machines the conversation has
 * reached, each by its system's logo; the one at work now opens to its
 * name, its light blinking, so the line stays one line on a phone too.
 */
.caption {
  position: absolute;
  top: 100%;
  left: 0;
  right: 0;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 0.5rem;
  padding-top: 0.65rem;
  font-size: 0.8125rem;
  transform: scale(calc(1 / var(--scale)));
  transform-origin: 0 0;
  width: calc(100% * var(--scale));
}

.name {
  display: inline-flex;
  align-items: center;
  gap: 0.35rem;
  color: var(--ink);
  font-weight: 600;
  white-space: nowrap;
}

/* The tags' room is kept from the start, so the line never grows as the first one comes. */
.tags {
  display: flex;
  flex-wrap: nowrap;
  align-items: center;
  justify-content: flex-end;
  gap: 0.3rem;
  min-height: 1.5rem;
}

.tag {
  display: inline-flex;
  box-sizing: border-box;
  height: 1.5rem;
  align-items: center;
  gap: 0.3rem;
  padding: 0.15rem 0.5rem;
  border: 1px solid var(--line);
  border-radius: 999px;
  background: var(--paper);
  color: var(--ink-soft);
  font-size: 0.75rem;
  font-weight: 500;
  white-space: nowrap;
}

.tag.using {
  color: var(--ink);
}

/*
 * The name never squeezes: as the work moves to its machine, the tag first
 * makes room and the name then fades in; as the work moves away, the name
 * fades out first and the tag then closes.
 */
.tag-name {
  display: grid;
  grid-template-columns: 0fr;
  margin-left: -0.3rem;
  transition:
    grid-template-columns 0.3s cubic-bezier(0.4, 0, 0.2, 1) 0.12s,
    margin-left 0.3s cubic-bezier(0.4, 0, 0.2, 1) 0.12s;
}

.tag-name > span {
  overflow: hidden;
  white-space: nowrap;
  opacity: 0;
  transition: opacity 0.12s ease;
}

.tag.using .tag-name {
  grid-template-columns: 1fr;
  margin-left: 0;
  transition:
    grid-template-columns 0.3s cubic-bezier(0.4, 0, 0.2, 1),
    margin-left 0.3s cubic-bezier(0.4, 0, 0.2, 1);
}

.tag.using .tag-name > span {
  opacity: 1;
  transition: opacity 0.2s ease 0.25s;
}

/* Alone on a narrow line, a tag never opens to its name: its logo and its light say it. */
.tag.compact .tag-name {
  display: none;
}

.light {
  width: 0.4rem;
  height: 0.4rem;
  border-radius: 50%;
  background: #cfc5bb;
  transition: background 0.3s ease;
}

.tag.using .light {
  background: #e9a23b;
  animation: blink 0.9s ease-in-out infinite;
}

@keyframes blink {
  50% {
    opacity: 0.3;
  }
}

.tag-enter-active {
  transition: opacity 0.3s ease, transform 0.3s ease;
}

.tag-enter-from {
  opacity: 0;
  transform: translateY(0.2rem);
}

@media (prefers-reduced-motion: reduce) {
  .device {
    transition: opacity 0.3s ease;
  }

  .tag.using .light {
    animation: none;
  }
}
</style>
