// What the tool remembers of a slot between calls and across a restart
// of its server: the process groups it started, the browser's, the paired
// runner, the development account, whether it copied the user's `.env`, the
// browser's emulation and the conditions `demi.net` set. One JSON file in the
// slot's folder, decoded against its schema when read.
import { existsSync, mkdirSync, readFileSync, renameSync, writeFileSync } from 'node:fs'
import { dirname } from 'node:path'
import { z } from 'zod'
import { slotPaths, type Slot } from './slot'

const groupSchema = z.object({
  pgid: z.number().int().positive(),
  started: z.string(),
  command: z.array(z.string()),
  log: z.string(),
})

/** The browser settings `demi.emulate` changes; unset ones keep the defaults. */
export const emulationSchema = z.object({
  width: z.number().int().positive().optional(),
  height: z.number().int().positive().optional(),
  scale: z.number().positive().optional(),
  theme: z.enum(['light', 'dark']).optional(),
  device: z.string().optional(),
  locale: z.string().optional(),
  timezone: z.string().optional(),
})
export type Emulation = z.infer<typeof emulationSchema>

/**
 * The conditions `demi.net` sets on the page's network, which the browser keeps
 * until it stops. `unreachable` holds the page's traffic while the browser
 * still reports a network; `offline` also tells the page it is offline.
 */
export const conditionsSchema = z.object({
  /** The round trip it adds, in milliseconds: half on each way. */
  latencyMs: z.number().nonnegative(),
  /** The limit on each way, in kilobits per second; null for none. */
  kbps: z.number().positive().nullable(),
  reach: z.enum(['online', 'unreachable', 'offline']),
})
export type Conditions = z.infer<typeof conditionsSchema>

/** The slot's browser: its process group, where its DevTools endpoint listens, and whether it shows a window. */
const browserSchema = groupSchema.extend({
  endpoint: z.string(),
  headed: z.boolean(),
})

/**
 * The runner `demi.runner` paired: its generation, which names it and counts up
 * with each `demi.runner({ fresh: true })`, and the device name the backend paired it as,
 * null until the pairing ends.
 */
export const runnerSchema = z.object({
  generation: z.number().int().positive(),
  device: z.string().nullable(),
})
export type RunnerRecord = z.infer<typeof runnerSchema>

const stateSchema = z.object({
  servers: z.object({
    backend: groupSchema.optional(),
    web: groupSchema.optional(),
    gallery: groupSchema.optional(),
    runner: groupSchema.optional(),
  }),
  /**
   * The account the backend seeded. A null password is the one the slot's
   * `.env` names as `DEMI_DEV_PASSWORD`, which the state never copies.
   */
  account: z.object({ email: z.string(), password: z.string().nullable() }).optional(),
  /** Whether `demi.up` copied the user's `.env` into the slot, which `demi.down` then deletes. */
  envCopied: z.boolean().optional(),
  emulation: emulationSchema.optional(),
  net: conditionsSchema.optional(),
  browser: browserSchema.optional(),
  runner: runnerSchema.optional(),
})
export type SlotState = z.infer<typeof stateSchema>
export type ServerName = keyof SlotState['servers']

export function readState(slot: Slot): SlotState {
  const path = slotPaths(slot).state
  if (!existsSync(path)) {
    return { servers: {} }
  }
  return stateSchema.parse(JSON.parse(readFileSync(path, 'utf8')))
}

export function writeState(slot: Slot, state: SlotState): void {
  const path = slotPaths(slot).state
  mkdirSync(dirname(path), { recursive: true })
  // A reader never sees half a file.
  const staging = `${path}.${process.pid}`
  writeFileSync(staging, `${JSON.stringify(state, null, 2)}\n`)
  renameSync(staging, path)
}

/** Reads the state, lets `change` edit it, and writes it back. */
export function updateState(slot: Slot, change: (state: SlotState) => void): SlotState {
  const state = readState(slot)
  change(state)
  writeState(slot, state)
  return state
}
