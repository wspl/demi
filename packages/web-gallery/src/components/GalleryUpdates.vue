<script setup lang="ts">
import { onUnmounted, ref, watch } from 'vue'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import SettingsUpdates from '@demicodes/web-ui/settings/SettingsUpdates.vue'
import type { UpgradeChannel, ServerUpgrade, UpgradeStep, UpgradeStepKind } from '@demicodes/web-ui/settings/server-upgrade'
import type { StepState } from '@demicodes/web-ui/ui/update-steps'
import { slow } from '../fixtures/slow'
import { productWould } from '../product-would'

/** Which moment of the server's updates the specimen starts at. */
export type UpdatesMoment = 'upToDate' | 'checking' | 'available' | 'downloading' | 'devices' | 'failed' | 'checkFailed'

/**
 * Settings › Updates with a stand-in server that runs 0.1.5469:
 * Beta offers nothing newer and Canary offers 0.1.5500. Check Now looks
 * again after a moment, a channel looks for the newest of it, and Update Now
 * runs every step of an update in a few seconds, waiting on two devices for
 * a while, and says the product would restart, where the product shows
 * every page the update's screen.
 */
const props = withDefaults(defineProps<{ moment?: UpdatesMoment }>(), { moment: 'available' })

const RUNNING = '0.1.5469'
const NEWER = '0.1.5500'
const NOTES = 'https://github.com/example/demi/releases/tag/v0.1.5500'
const MB = 1024 * 1024
const AT = '2026-10-25T09:00:00Z'
const KINDS: readonly UpgradeStepKind[] = ['download', 'unpack', 'checkConfiguration', 'downloadGvisor', 'installImage', 'checkDatabases', 'updateDevices', 'restart']

/** The steps with those before `kind` done as a typical update does them, `kind` as `state`, and those after waiting. */
function stepsAt(kind: UpgradeStepKind, state: StepState): UpgradeStep[] {
  const reached = KINDS.indexOf(kind)
  return KINDS.map((each, index): UpgradeStep => {
    if (index > reached)
      return { kind: each, state: { state: 'waiting' } }
    if (index === reached)
      return { kind: each, state }
    return { kind: each, state: doneState(each) }
  })
}

function doneState(kind: UpgradeStepKind): StepState {
  switch (kind) {
    case 'download':
      return { state: 'done', tookMs: 21_000, summary: { kind: 'bytes', bytes: 214 * MB } }
    case 'unpack':
      return { state: 'done', tookMs: 4_000, summary: { kind: 'bytes', bytes: 310 * MB } }
    case 'downloadGvisor':
      return { state: 'skipped', why: 'installed' }
    case 'installImage':
      return { state: 'done', tookMs: 9_000, summary: { kind: 'bytes', bytes: 96 * MB } }
    case 'checkDatabases':
      return { state: 'done', tookMs: 800, summary: { kind: 'count', count: 2 } }
    case 'updateDevices':
      return { state: 'done', tookMs: 70_000, summary: { kind: 'devices', done: 5, total: 5 } }
    default:
      return { state: 'done', tookMs: 600, summary: { kind: 'none' } }
  }
}

const ago = (seconds: number) => new Date(Date.now() - seconds * 1000).toISOString()
const later = (seconds: number) => new Date(Date.now() + seconds * 1000).toISOString()

function startingAt(moment: UpdatesMoment): ServerUpgrade {
  const offered = { state: 'checked', at: AT, newer: NEWER, notesUrl: NOTES } as const
  switch (moment) {
    case 'upToDate':
      return { running: RUNNING, channel: 'beta', check: { state: 'checked', at: AT, newer: null }, run: { state: 'none' } }
    case 'checking':
      return { running: RUNNING, channel: 'beta', check: { state: 'checking' }, run: { state: 'none' } }
    case 'available':
      return { running: RUNNING, channel: 'canary', check: offered, run: { state: 'none' } }
    case 'downloading':
      return {
        running: RUNNING,
        channel: 'canary',
        check: offered,
        run: { state: 'updating', to: NEWER, steps: stepsAt('download', { state: 'running', started: ago(9), progress: { kind: 'bytes', done: 88 * MB, total: 214 * MB, perSecond: 1.8 * MB } }) },
      }
    case 'devices':
      return {
        running: RUNNING,
        channel: 'canary',
        check: offered,
        run: {
          state: 'updating',
          to: NEWER,
          steps: stepsAt('updateDevices', { state: 'running', started: ago(48), progress: { kind: 'devices', done: 3, total: 5, waiting: ['Alex’s MacBook Pro', 'Office iMac'], givesUpAt: later(72) } }),
        },
      }
    case 'failed':
      return {
        running: RUNNING,
        channel: 'canary',
        check: offered,
        run: {
          state: 'failed',
          to: NEWER,
          reason: 'DEMI_MANAGED_SUBNET overlaps host route 10.0.0.0/8',
          steps: stepsAt('checkConfiguration', { state: 'failed', reason: 'DEMI_MANAGED_SUBNET overlaps host route 10.0.0.0/8' }),
        },
      }
    case 'checkFailed':
      return { running: RUNNING, channel: 'beta', check: { state: 'failed', at: AT, reason: 'GitHub answered 403 Forbidden' }, run: { state: 'none' } }
  }
}

const state = ref<ServerUpgrade>(startingAt(props.moment))
watch(() => props.moment, (moment) => (state.value = startingAt(moment)))
let stopped = false
onUnmounted(() => (stopped = true))

async function check(): Promise<void> {
  state.value = { ...state.value, check: { state: 'checking' } }
  await slow(800)
  // Canary's newest is NEWER, offered only to a server that runs an older one.
  const newer = state.value.channel === 'canary' && state.value.running !== NEWER ? NEWER : null
  state.value = { ...state.value, check: { state: 'checked', at: new Date().toISOString(), newer, notesUrl: newer ? NOTES : null } }
}

function setChannel(channel: UpgradeChannel): Promise<void> {
  state.value = { ...state.value, channel }
  return check()
}

function at(steps: UpgradeStep[], kind: UpgradeStepKind, stepState: StepState): UpgradeStep[] {
  return steps.map((step) => (step.kind === kind ? { kind, state: stepState } : step))
}

async function update(version: string): Promise<void> {
  let steps: UpgradeStep[] = KINDS.map((kind) => ({ kind, state: { state: 'waiting' } }))
  const show = () => (state.value = { ...state.value, run: { state: 'updating', to: version, steps } })
  const started = new Date().toISOString()
  for (let done = 0; done <= 214 && !stopped; done += 214 / 8) {
    steps = at(steps, 'download', { state: 'running', started, progress: { kind: 'bytes', done: done * MB, total: 214 * MB, perSecond: done === 0 ? 0 : 1.8 * MB } })
    show()
    await slow(300)
  }
  for (const kind of KINDS.slice(0, 6)) {
    if (stopped)
      return
    if (kind !== 'download') {
      steps = at(steps, kind, { state: 'running', started: new Date().toISOString(), progress: kind === 'unpack' || kind === 'installImage' ? { kind: 'bytes', done: 40 * MB, total: 96 * MB } : null })
      show()
      await slow(kind === 'checkConfiguration' || kind === 'downloadGvisor' ? 300 : 700)
    }
    steps = at(steps, kind, doneState(kind))
    show()
  }
  const waiting = ['Alex’s MacBook Pro', 'Office iMac', 'Studio Mac mini']
  const givesUpAt = later(120)
  for (let done = 2; done <= 5 && !stopped; done += 1) {
    steps = at(steps, 'updateDevices', { state: 'running', started, progress: { kind: 'devices', done, total: 5, waiting: waiting.slice(done - 2), givesUpAt } })
    show()
    await slow(1200)
  }
  steps = at(at(steps, 'updateDevices', doneState('updateDevices')), 'restart', { state: 'running', started: new Date().toISOString(), progress: null })
  show()
  await slow(1200)
  productWould(`Restart in ${version}`)
  state.value = { ...startingAt('upToDate'), running: version, channel: state.value.channel }
}
</script>

<template>
  <SettingsUpdates
    :state="state"
    :overlay-store="appOverlayStore"
    :on-set-channel="setChannel"
    :on-check="check"
    :on-count-agents="async () => { await slow(600); return 2 }"
    :on-update="update"
  />
</template>
