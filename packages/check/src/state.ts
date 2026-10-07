// What the tool remembers of a slot between commands and across a restart
// of its daemon: the process groups it started, the development account,
// whether it copied the user's `.env`, and the browser's emulation. One JSON
// file in the slot's folder, decoded against its schema when read.
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

/** The browser settings `emulate` changes; unset ones keep the defaults. */
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
  /** Whether `up` copied the user's `.env` into the slot, which `down` then deletes. */
  envCopied: z.boolean().optional(),
  emulation: emulationSchema.optional(),
  /** The address the page showed when the daemon ended for changed code, which the next browser opens again. */
  reopen: z.string().optional(),
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
