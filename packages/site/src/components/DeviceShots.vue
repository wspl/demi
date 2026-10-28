<script setup lang="ts">
// What the stage shows besides the browser and the phone (`demo/message.ts`):
// the machines the conversation works on now, close, with what their
// terminals print; or every machine around the app at the scene's end. A
// machine close stands beside the browser, where the phone stood, or beside
// the phone, where the browser stood, alone or with a second one beside it. The machines are pictures with transparent backgrounds;
// where they stand, the wires, the dots that travel them, the lights and the
// terminal text are drawn here, so they follow the scene.
//
// Each piece comes and goes from the side it lives on as it fades: the
// machines from the stage's right, the server in the overview where it
// stands.
//
//   beside the browser:            beside the phone:        every:
//   +-- browser ------------+      +-phone-+  [terminal]    [app]-▶[server]-┬▶[mac]
//   |  conversation [term]  |      |       |  [machine]                     ├▶[win]
//   |               [mach]  |      |       |  name+light                    ├▶[vps]
//   +------------- name+light      +-------+                                └▶[home lab]
import { computed, onBeforeUnmount, onMounted, ref, useId, watch } from 'vue'
import DeviceLogo from '@demicodes/web-ui/devices/DeviceLogo.vue'
import type { DeviceShot, ShotDevice, ShotMachine } from '@/demo/message'
import server from '@/assets/devices/server.webp'
import macbook from '@/assets/devices/macbook.webp'
import windowsPc from '@/assets/devices/windows-pc.webp'
import vps from '@/assets/devices/vps.webp'
import homeLab from '@/assets/devices/home-lab.webp'

const props = defineProps<{ shot: DeviceShot | null }>()
/** What this stage's SVG ids start with, so another stage's never answer its references. */
const ids = useId()
/** Whether the reader asks for less motion: the data then stands still on its wires. */
const lessMotion = ref(false)
let motionQuery: MediaQueryList | undefined
const hearMotion = (): void => {
  lessMotion.value = motionQuery?.matches ?? false
}
onMounted(() => {
  motionQuery = window.matchMedia('(prefers-reduced-motion: reduce)')
  hearMotion()
  motionQuery.addEventListener('change', hearMotion)
})
onBeforeUnmount(() => motionQuery?.removeEventListener('change', hearMotion))

/** A place in shares of the stage: a picture's width, and where its bottom centre stands. */
interface Spot {
  width: number
  x: number
  bottom: number
}

/** Each machine: its picture, its picture's size, its screen on it if it has one, its system and its places. */
interface Machine {
  src: string
  w: number
  h: number
  screen?: [x: number, y: number, width: number, height: number]
  /** The name the scene's conversation calls it by. */
  name: string
  logo: 'apple' | 'windows' | 'ubuntu' | 'debian'
  /** Close beside the browser, beside the phone alone or with a second machine, and in the overview. */
  browser: Spot
  phone: Spot
  pair?: Spot
  every: Spot
}

/** Where a machine stands close. */
type Stand = 'browser' | 'phone' | 'pair'

const MACHINES: Record<ShotDevice, Machine> = {
  macbook: {
    src: macbook, w: 1108, h: 698, screen: [136, 44, 836, 479], name: 'MacBook', logo: 'apple',
    browser: { width: 0.44, x: 0.775, bottom: 0.99 },
    phone: { width: 0.5, x: 0.68, bottom: 0.93 },
    every: { width: 0.17, x: 0.86, bottom: 0.23 },
  },
  'windows-pc': {
    src: windowsPc, w: 1237, h: 655, screen: [36, 43, 856, 456], name: 'Windows PC', logo: 'windows',
    browser: { width: 0.52, x: 0.735, bottom: 0.99 },
    phone: { width: 0.52, x: 0.68, bottom: 0.93 },
    every: { width: 0.17, x: 0.86, bottom: 0.47 },
  },
  vps: {
    src: vps, w: 1400, h: 437, name: 'VPS', logo: 'ubuntu',
    browser: { width: 0.4, x: 0.795, bottom: 0.99 },
    phone: { width: 0.46, x: 0.68, bottom: 0.91 },
    pair: { width: 0.26, x: 0.53, bottom: 0.93 },
    every: { width: 0.19, x: 0.86, bottom: 0.65 },
  },
  'home-lab': {
    src: homeLab, w: 917, h: 708, name: 'Home Lab', logo: 'debian',
    browser: { width: 0.26, x: 0.81, bottom: 0.99 },
    phone: { width: 0.3, x: 0.68, bottom: 0.94 },
    pair: { width: 0.18, x: 0.87, bottom: 0.93 },
    every: { width: 0.13, x: 0.86, bottom: 0.92 },
  },
}

/** How long a piece takes to go, as the styles' `.node` has it. */
const SINK_MS = 400
/**
 * How long after it is shown a machine stands in place, as the styles'
 * `.node.shown` and `.with-phone` have it: its delay and most of its way in.
 */
const ARRIVE_MS = { browser: 900, phone: 1100 }
/** How long a terminal takes between two lines, so each reads as it comes. */
const LINE_MS = 350
/**
 * How many lines a terminal holds, and a screen: each keeps its size whatever
 * it prints, as a terminal window does, and shows its latest lines once it
 * has more.
 */
const ROWS = { terminal: 5, screen: 8 }

const SERVER = { w: 837, h: 756, every: { width: 0.13, x: 0.58, bottom: 0.6 } }
/** Where the wire from the app starts in the overview: the right edge of the app, which DemoStage shrinks to the left. */
const APP = { x: 0.47, y: 0.5 }

const box = ref<HTMLElement | null>(null)
const size = ref({ w: 0, h: 0 })
let observer: ResizeObserver | undefined
onMounted(() => {
  observer = new ResizeObserver(([entry]) => (size.value = { w: entry!.contentRect.width, h: entry!.contentRect.height }))
  observer.observe(box.value!)
})
onBeforeUnmount(() => observer?.disconnect())

const close = computed(() => (props.shot?.kind === 'close' ? props.shot : null))
function closeOf(name: ShotDevice): ShotMachine | undefined {
  return close.value?.machines.find((each) => each.device === name)
}
/**
 * Whether the overview shows: a moment after the scene asks for it, and
 * until a moment after it moves on, so every machine leaves where it stood
 * before it comes back in its next place.
 */
const every = ref(false)
/** Whether the machines are leaving the overview. */
const parting = ref(false)
let changing: ReturnType<typeof setTimeout> | undefined
watch(() => props.shot?.kind === 'all', (all) => {
  clearTimeout(changing)
  parting.value = !all && every.value
  changing = setTimeout(() => {
    every.value = all
    parting.value = false
  }, SINK_MS)
})
onBeforeUnmount(() => clearTimeout(changing))

/**
 * What each machine's terminal printed last, and where it stood: a machine
 * going down keeps both, so it leaves as it was.
 */
const last = ref<Partial<Record<ShotDevice, { lines: string[]; done: boolean; stand: Stand }>>>({})
/** The machines that were up before the latest shot and are still: they move over, rather than come up. */
const staying = ref<ReadonlySet<ShotDevice>>(new Set())
watch(close, (shot, before) => {
  const up = new Set(shot?.machines.map((each) => each.device))
  staying.value = new Set(before?.machines.map((each) => each.device).filter((name) => up.has(name)))
  if (!shot)
    return
  const stand: Stand = shot.beside === 'browser' ? 'browser' : shot.machines.length > 1 ? 'pair' : 'phone'
  const next = { ...last.value }
  for (const each of shot.machines)
    next[each.device] = { lines: each.lines, done: each.done, stand: MACHINES[each.device][stand] ? stand : 'phone' }
  last.value = next
})

/**
 * How many of each machine's lines show. A terminal prints nothing until its
 * machine stands in place, then one line at a time, whenever the scene sent
 * them: the scene tells what runs, and the stage when it is seen.
 */
const revealed = ref<Partial<Record<ShotDevice, number>>>({})
/** When each machine shown stands in place, and when its terminal last printed. */
const arrival = new Map<ShotDevice, number>()
/** The lines each terminal shows, kept while its machine is away, so a machine that comes back goes on from them. */
const printed = new Map<ShotDevice, string[]>()
const printedAt = new Map<ShotDevice, number>()
let pumping: ReturnType<typeof setTimeout> | undefined
function pump(): void {
  clearTimeout(pumping)
  const now = performance.now()
  let soonest = Infinity
  const next = { ...revealed.value }
  for (const [name, at] of arrival) {
    const total = last.value[name]?.lines.length ?? 0
    const shownLines = next[name] ?? 0
    if (shownLines >= total)
      continue
    const due = Math.max(at, (printedAt.get(name) ?? 0) + LINE_MS)
    if (now >= due) {
      next[name] = shownLines + 1
      printedAt.set(name, now)
      printed.set(name, last.value[name]!.lines.slice(0, shownLines + 1))
      if (shownLines + 1 < total)
        soonest = Math.min(soonest, now + LINE_MS)
    } else {
      soonest = Math.min(soonest, due)
    }
  }
  revealed.value = next
  if (soonest < Infinity)
    pumping = setTimeout(pump, soonest - now)
}
watch(() => (Object.keys(MACHINES) as ShotDevice[]).filter((name) => shown(name) && !every.value), (up, before) => {
  for (const name of up) {
    if (before?.includes(name))
      continue
    arrival.set(name, performance.now() + ARRIVE_MS[last.value[name]?.stand === 'browser' ? 'browser' : 'phone'])
    // A machine that comes back shows what it printed before and goes on
    // from there; the lines that differ, as when the play starts over, it
    // prints again.
    const lines = last.value[name]?.lines ?? []
    const shownBefore = printed.get(name) ?? []
    let kept = 0
    while (kept < shownBefore.length && kept < lines.length && shownBefore[kept] === lines[kept])
      kept++
    revealed.value = { ...revealed.value, [name]: kept }
    printed.set(name, lines.slice(0, kept))
  }
  for (const name of arrival.keys()) {
    if (!up.includes(name))
      arrival.delete(name)
  }
  pump()
})
watch(last, pump)
onBeforeUnmount(() => clearTimeout(pumping))
/** The lines a machine's terminal shows now, and whether the last one is its work's done line. */
function linesOf(name: ShotDevice): { lines: string[]; done: boolean } {
  const all = last.value[name]
  if (!all)
    return { lines: [], done: false }
  const count = revealed.value[name] ?? 0
  const rows = MACHINES[name].screen ? ROWS.screen : ROWS.terminal
  return { lines: all.lines.slice(Math.max(0, count - rows), count), done: all.done && count >= all.lines.length }
}

/** A picture's box in pixels, from its spot. */
function place(spot: Spot, w: number, h: number) {
  const width = spot.width * size.value.w
  const height = (width * h) / w
  return { left: spot.x * size.value.w - width / 2, top: spot.bottom * size.value.h - height, width, height }
}

/** Where each machine is now: in the overview, or where it stands close, down or up. */
function machineBox(name: ShotDevice) {
  const machine = MACHINES[name]
  return place(every.value ? machine.every : machine[last.value[name]?.stand ?? 'browser']!, machine.w, machine.h)
}
const serverBox = computed(() => place(SERVER.every, SERVER.w, SERVER.h))
function shown(name: ShotDevice): boolean {
  if (parting.value)
    return false
  return every.value || closeOf(name) !== undefined
}

/** A wire from `a`'s right edge to `b`'s left edge, as a gentle curve. */
function wire(a: { left: number; top: number; width: number; height: number } | { x: number; y: number }, b: { left: number; top: number; width: number; height: number }): string {
  const [x1, y1] = 'x' in a ? [a.x, a.y] : [a.left + a.width, a.top + a.height / 2]
  const [x2, y2] = [b.left, b.top + b.height / 2]
  const mx = (x1 + x2) / 2
  return `M${x1},${y1} C${mx},${y1} ${mx},${y2} ${x2},${y2}`
}

const wires = computed(() => {
  const s = serverBox.value
  const app = { x: APP.x * size.value.w, y: APP.y * size.value.h }
  // Each wire grows once both its ends stand: the app's as the server comes,
  // each machine's as that machine comes.
  return [
    { id: 'app', d: wire(app, s), delay: 0.35 },
    ...(Object.keys(MACHINES) as ShotDevice[]).map((name, order) => ({ id: name, d: wire(s, place(MACHINES[name].every, MACHINES[name].w, MACHINES[name].h)), delay: 0.5 + order * 0.15 })),
  ]
})

/** The data flowing between two machines close: a wire from one's right side to the other's left, and its label over the middle. */
const flow = computed(() => {
  const data = close.value?.flow
  if (!data)
    return null
  const a = machineBox(data.from)
  const b = machineBox(data.to)
  const y = Math.min(a.top + a.height / 2, b.top + b.height / 2)
  const [x1, x2] = [a.left + a.width + 6, b.left - 6]
  return { label: data.label, d: `M${x1},${y} L${x2},${y}`, x: (x1 + x2) / 2, y }
})

/** A screen's box on its machine's picture, in shares of the picture. */
function screenBox(machine: Machine) {
  const [x, y, w, h] = machine.screen!
  return { left: `${(x / machine.w) * 100}%`, top: `${(y / machine.h) * 100}%`, width: `${(w / machine.w) * 100}%`, height: `${(h / machine.h) * 100}%` }
}

/**
 * The terminal of a machine that has no screen, over its picture: beside the
 * browser, its right edge on the stage's; beside the phone, centred on it,
 * narrower with a second machine beside it.
 */
const TERMINAL = { width: 0.4, pair: 0.27, right: 0.995 }
function terminalBox(name: ShotDevice) {
  const b = machineBox(name)
  const stand = last.value[name]?.stand
  const width = (stand === 'pair' ? TERMINAL.pair : TERMINAL.width) * size.value.w
  const left = stand === 'browser' ? TERMINAL.right * size.value.w - width - b.left : (b.width - width) / 2
  return { left: `${left}px`, width: `${width}px` }
}

function px(b: { left: number; top: number; width: number; height: number }) {
  return { left: `${b.left}px`, top: `${b.top}px`, width: `${b.width}px`, height: `${b.height}px` }
}
</script>

<template>
  <div ref="box" class="shots" :class="{ every }" aria-hidden="true">
    <svg class="wires" :viewBox="`0 0 ${size.w} ${size.h}`">
      <g v-for="line in wires" :key="line.id" :style="{ '--delay': `${line.delay}s` }">
        <mask :id="`${ids}-grow-${line.id}`" maskUnits="userSpaceOnUse" x="0" y="0" :width="size.w" :height="size.h">
          <path :d="line.d" class="grow" pathLength="1" />
        </mask>
        <path :id="`${ids}-wire-${line.id}`" :d="line.d" class="wire" :mask="`url(#${ids}-grow-${line.id})`" />
        <circle v-if="every && !lessMotion" r="3.5" class="dot">
          <animateMotion dur="1.8s" repeatCount="indefinite" :keyPoints="'0;1'" keyTimes="0;1" calcMode="spline" keySplines="0.45 0 0.55 1">
            <mpath :href="`#${ids}-wire-${line.id}`" />
          </animateMotion>
        </circle>
      </g>
    </svg>
    <svg v-if="flow" class="flow" :viewBox="`0 0 ${size.w} ${size.h}`">
      <g>
        <mask :id="`${ids}-grow-flow`" maskUnits="userSpaceOnUse" x="0" y="0" :width="size.w" :height="size.h">
          <path :d="flow.d" class="grow" pathLength="1" />
        </mask>
        <path :id="`${ids}-flow-wire`" :d="flow.d" class="wire" :mask="`url(#${ids}-grow-flow)`" />
        <circle v-for="i in lessMotion ? 0 : 3" :key="i" r="3.5" class="dot">
          <animateMotion dur="0.9s" :begin="`${(i - 1) * 0.3}s`" repeatCount="indefinite" calcMode="linear">
            <mpath :href="`#${ids}-flow-wire`" />
          </animateMotion>
        </circle>
      </g>
    </svg>
    <span v-if="flow" class="packet" :style="{ left: `${flow.x}px`, top: `${flow.y}px` }">{{ flow.label }}</span>

    <div class="node server" :class="{ shown: every && !parting }" :style="px(serverBox)">
      <img :src="server" alt="">
      <span class="label"><b>Demi</b> · your server</span>
    </div>

    <template v-for="(machine, name, order) in MACHINES" :key="name">
      <div class="node machine" :class="{ shown: shown(name), staying: staying.has(name) && !every, 'with-phone': last[name]?.stand !== 'browser' && !every }" :style="{ ...px(machineBox(name)), '--order': order }">
        <img :src="machine.src" alt="">
        <span class="label">
          <DeviceLogo :logo="machine.logo" :size="13" />
          <b>{{ machine.name }}</b>
          <i class="light" :class="{ busy: closeOf(name) !== undefined && !linesOf(name).done, done: every || (closeOf(name) !== undefined && linesOf(name).done) }" />
        </span>
        <div v-if="last[name]" :class="[machine.screen ? 'display' : 'terminal', { narrow: last[name]?.stand === 'pair' }]" :style="machine.screen ? screenBox(machine) : terminalBox(name)">
          <p v-for="(line, i) in linesOf(name).lines" :key="line + i" :class="{ ok: linesOf(name).done && i === linesOf(name).lines.length - 1 }">{{ line }}<i v-if="i === linesOf(name).lines.length - 1" class="cursor" /></p>
          <p v-if="!linesOf(name).lines.length"><i class="cursor" /></p>
        </div>
      </div>
    </template>
  </div>
</template>

<style scoped>
/*
 * Two motions: a piece comes in fast and settles as it fades in, and goes
 * gathering speed as it fades out. One that takes another's place starts as
 * that one goes.
 */
.shots {
  --rise: cubic-bezier(0.16, 1, 0.3, 1);
  --sink: cubic-bezier(0.55, 0, 0.75, 0);

  position: absolute;
  inset: 0;
  pointer-events: none;
}

/* Data between two machines: a stream of dots towards the receiver, and its label over it. */
.flow {
  position: absolute;
  inset: 0;
  width: 100%;
  height: 100%;
  overflow: visible;
}

/* The data's wire grows from the sender once both machines stand, and the data flows once it reaches the receiver. */
.flow .grow {
  animation: grow 0.6s cubic-bezier(0.65, 0, 0.35, 1) 1.1s both;
}

.flow .dot {
  animation: dot-in 0.3s ease-out 1.6s both;
}

@keyframes grow {
  from {
    stroke-dashoffset: 1;
  }

  to {
    stroke-dashoffset: 0;
  }
}

@keyframes dot-in {
  from {
    opacity: 0;
  }
}

.flow .wire {
  stroke: #d9b49a;
}

.packet {
  position: absolute;
  padding: 0.125rem 0.5rem;
  border-radius: 62.4375rem;
  background: var(--paper);
  box-shadow: 0 0.25rem 0.75rem rgb(40 25 15 / 12%);
  color: var(--ink);
  font: 0.6875rem / 1.4 var(--font-mono, ui-monospace, Menlo, monospace);
  white-space: nowrap;
  transform: translate(-50%, 0.625rem);
  animation: packet-in 0.45s var(--rise) 1.5s both;
}

@keyframes packet-in {
  from {
    opacity: 0;
    transform: translate(-50%, 0.25rem) scale(0.6);
  }
}

/*
 * The overview's wires grow from the app outwards, each as its far end
 * comes, and their data flows once they reach it.
 */
.wires {
  position: absolute;
  inset: 0;
  width: 100%;
  height: 100%;
  overflow: visible;
}

.grow {
  fill: none;
  stroke: #fff;
  stroke-width: 8;
  stroke-dasharray: 1;
  stroke-dashoffset: 1;
  transition: stroke-dashoffset 0.3s ease-in;
}

.every .grow {
  stroke-dashoffset: 0;
  transition: stroke-dashoffset 0.7s cubic-bezier(0.65, 0, 0.35, 1) var(--delay);
}

.wires .dot {
  animation: dot-in 0.3s ease-out calc(var(--delay) + 0.6s) both;
}

.wire {
  fill: none;
  stroke: #cdbcad;
  stroke-width: 1.6;
  stroke-dasharray: 4 6;
}

.dot {
  fill: #d9824a;
}

.node {
  position: absolute;
  /* Its own layer: the picture's shadow is drawn once, not again in every frame of a move. */
  will-change: transform, opacity;
  visibility: hidden;
  opacity: 0;
  transform: translateX(4rem);
  transition:
    transform 0.4s var(--sink),
    opacity 0.3s ease-in 0.05s,
    visibility 0s 0.4s;
}

.node.shown {
  visibility: visible;
  opacity: 1;
  transform: none;
  transition:
    transform 0.75s var(--rise) 0.3s,
    opacity 0.45s ease-out 0.3s,
    visibility 0s 0.3s;
}

/* Beside the phone, a machine comes up once the phone has moved over from its place. */
.machine.shown.with-phone {
  transition-delay: 0.5s;
}

/* A machine already up moves over to make room for a second one. */
.machine.shown.staying {
  transition:
    left 0.7s cubic-bezier(0.65, 0, 0.35, 1),
    top 0.7s cubic-bezier(0.65, 0, 0.35, 1),
    width 0.7s cubic-bezier(0.65, 0, 0.35, 1),
    height 0.7s cubic-bezier(0.65, 0, 0.35, 1);
}

/* The server in the overview grows into its place. */
.node.server:not(.shown) {
  transform: scale(0.85);
}

/*
 * At the end the server comes up as the app has shrunk, then the machines
 * one by one from the top, so none passes another on its way up.
 */
.every .server.shown {
  transition-delay: 0.1s;
}

.every .machine.shown {
  transition-delay: calc(0.3s + var(--order) * 0.15s);
}

.node img {
  display: block;
  width: 100%;
  height: auto;
  filter: drop-shadow(0 1rem 1.5rem rgb(40 25 15 / 16%));
}

.label {
  position: absolute;
  top: calc(100% + 0.5rem);
  left: 50%;
  display: inline-flex;
  gap: 0.375rem;
  align-items: center;
  color: var(--ink-faint);
  font-size: 0.8125rem;
  white-space: nowrap;
  transform: translateX(-50%);
}

.shots:not(.every) .label {
  padding: 0.25rem 0.625rem;
  border-radius: 62.4375rem;
  background: var(--paper);
  box-shadow: 0 0.25rem 0.75rem rgb(40 25 15 / 12%);
}

.label b {
  color: var(--ink);
  font-weight: 600;
}

.light {
  width: 0.5rem;
  height: 0.5rem;
  border-radius: 50%;
  background: #cfc5bb;
  transition: background 0.3s ease;
}

.light.busy {
  background: #e9a23b;
  animation: blink 0.9s ease-in-out infinite;
}

.light.done {
  background: #3fa66a;
}

@keyframes blink {
  50% {
    opacity: 0.35;
  }
}

.display,
.terminal {
  position: absolute;
  box-sizing: border-box;
  overflow: hidden;
  background: #2a2523;
  color: #ddd4cb;
  font: 0.75rem / 1.5 var(--font-mono, ui-monospace, Menlo, monospace);
}

/*
 * The screen is dark in the picture itself, so its edge is the picture's own:
 * the text alone goes over it, a little soft and glowing, under the same grain
 * as the picture and the screen's shade at its edges, so it reads as on the
 * screen rather than over it.
 */
.display {
  padding: 3.5% 4%;
  background: none;
  color: #d3c9bd;
  font-size: 0.6875rem;
  text-shadow: 0 0 0.35em rgb(211 201 189 / 40%);
  filter: blur(0.35px);
}

.display::after {
  position: absolute;
  inset: 0;
  background: url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='120' height='120'%3E%3Cfilter id='n'%3E%3CfeTurbulence type='fractalNoise' baseFrequency='0.9' numOctaves='2' stitchTiles='stitch'/%3E%3C/filter%3E%3Crect width='120' height='120' filter='url(%23n)' opacity='0.55'/%3E%3C/svg%3E");
  box-shadow: inset 0 0 1.25em rgb(0 0 0 / 45%);
  content: '';
  mix-blend-mode: overlay;
  opacity: 0.35;
  pointer-events: none;
}

.terminal {
  bottom: calc(100% + 1.25rem);
  height: calc(1.5em * 5 + 1.5rem);
  /* A terminal that makes room for a second machine changes size as it moves over. */
  transition-property: left, width, height, padding, font-size;
  transition-duration: 0.7s;
  transition-timing-function: cubic-bezier(0.65, 0, 0.35, 1);
  padding: 0.75rem 0.875rem;
  border-radius: 0.625rem;
  box-shadow: 0 0.75rem 2rem rgb(40 25 15 / 18%);
}

/* Beside a second machine, the terminal is narrower and its text smaller. */
.terminal.narrow {
  height: calc(1.5em * 5 + 1.25rem);
  padding: 0.625rem 0.75rem;
  font-size: 0.6875rem;
}

/* The cursor after the latest line, as a terminal waits for more. */
.cursor {
  display: inline-block;
  width: 0.5em;
  height: 1.1em;
  margin-left: 0.25em;
  vertical-align: -0.15em;
  background: currentcolor;
  animation: caret 1s steps(1) infinite;
}

@keyframes caret {
  50% {
    opacity: 0;
  }
}

/* In the overview a machine shows no work: its terminal folds away. */
.every .display,
.every .terminal {
  visibility: hidden;
  transform: scale(0.6);
  transform-origin: center bottom;
  transition:
    transform 0.3s var(--sink),
    visibility 0s 0.3s;
}

.display p,
.terminal p {
  margin: 0;
  overflow: hidden;
  white-space: nowrap;
  text-overflow: ellipsis;
  animation: line-in 0.25s ease both;
}

.ok {
  color: #8fd4a6;
}

@keyframes line-in {
  from {
    opacity: 0;
    transform: translateY(0.25rem);
  }
}
</style>
