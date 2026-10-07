// The slot a check runs in (browse.md § One browser per slot): slot n for a
// checkout at `../demi-slots/<n>`, slot 0 for any other, such as the user's
// own. The slot decides the servers' ports and the folder that holds the
// slot's browser, state and output, so two slots never share anything.
import { basename, dirname, join, resolve } from 'node:path'

/** The ports of slot n's servers (AGENTS.md § Parallel Development Mode). */
export interface SlotPorts {
  backend: number
  web: number
  gallery: number
  /**
   * Where the browser reaches the web app: the slot's network, which passes
   * the page's requests and sockets on to the web server and adds what
   * `demi.net` sets.
   */
  network: number
  /**
   * The local preview domain service, `demi-preview.localhost:<port>`,
   * which the slot's backend registers its namespace with.
   */
  preview: number
}

export interface Slot {
  /** The checkout's root, the repository the tool runs from. */
  root: string
  number: number
  ports: SlotPorts
  /** `.cache/browse/` in the checkout: the browser, state and output. */
  folder: string
}

/** The folder that holds the numbered slots beside the user's checkout. */
const SLOTS = 'demi-slots'

/** The slot of the checkout at `root`. */
export function slotNumber(root: string): number {
  const path = resolve(root)
  if (basename(dirname(path)) !== SLOTS) {
    return 0
  }
  const name = basename(path)
  if (!/^[1-9]$/.test(name)) {
    return 0
  }
  return Number(name)
}

/** Slot n's ports: 33n0 the backend, 33n1 the web app, 33n2 the gallery, 33n3 the network, 33n4 the preview domain. */
export function slotPorts(number: number): SlotPorts {
  const base = 3300 + number * 10
  return { backend: base, web: base + 1, gallery: base + 2, network: base + 3, preview: base + 4 }
}

/** The slot of the checkout this tool belongs to. */
export function currentSlot(): Slot {
  const root = resolve(import.meta.dir, '../../..')
  const number = slotNumber(root)
  return { root, number, ports: slotPorts(number), folder: join(root, '.cache/browse') }
}

/** The files and folders the slot's tool keeps under its folder. */
export function slotPaths(slot: Slot) {
  return {
    socket: join(slot.folder, 'daemon.sock'),
    daemonLog: join(slot.folder, 'daemon.log'),
    profile: join(slot.folder, 'profile'),
    shots: join(slot.folder, 'shots'),
    logs: join(slot.folder, 'logs'),
    state: join(slot.folder, 'state.json'),
    /** The page's logs a server that ends for changed code leaves for the next one. */
    pageLogs: join(slot.folder, 'page-logs.json'),
    /** The `keep` a server that ends for changed code leaves for the next one. */
    keep: join(slot.folder, 'keep.bin'),
    /** The modules the calls' scripts run as while they run. */
    calls: join(slot.folder, 'calls'),
    runnerHome: join(slot.folder, 'runner'),
    /** The development backend's data directory, which outlives the backend until `down --wipe`. */
    backendData: join(slot.folder, 'backend'),
  }
}

/** The address of a server of the slot, as the browser and the tool reach it. */
export function local(port: number): string {
  return `http://127.0.0.1:${port}`
}
