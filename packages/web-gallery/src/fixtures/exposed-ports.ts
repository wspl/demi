import { ref } from 'vue'
import type { ExposedPortChange, SettingsExposedPort } from '@demicodes/web-ui/settings/exposed-ports'

/** The id each specimen device has on a server: the host of its exposed ports' addresses. */
const DEVICE_UUIDS: Record<string, string> = {
  mac: '3f1c9a2e-7b4d-4e8a-9c21-5d6e0f1ae2a0',
  build: '8a2d4c61-0e3f-4b7a-a5c9-1f2e3d4c5b6a',
  studio: 'c7e1b2a3-4d5f-4061-8a9b-0c1d2e3f4a5b',
  lab: '1b2c3d4e-5f60-4718-9a0b-c1d2e3f4a5b6',
  cloud: '9e8d7c6b-5a49-4382-a1f0-e9d8c7b6a594',
}

function address(deviceId: string, port: number | null): string | null {
  return port === null ? null : `http://${DEVICE_UUIDS[deviceId] ?? deviceId}:${port}`
}

/** alex-mbp's address for `port`, as a provider entry's endpoint names it. */
export function macAddress(port: number): string {
  return `http://${DEVICE_UUIDS.mac}:${port}`
}

/** The port of alex-mbp an endpoint names, or null for any other address. */
export function macPortOf(endpoint: string): number | null {
  const named = new RegExp(`^https?://${DEVICE_UUIDS.mac}:(\\d+)`).exec(endpoint)
  return named ? Number(named[1]) : null
}

/** Makes a row of the gallery's exposed ports. */
let next = 0
function row(deviceId: string, port: number | null, label: string): SettingsExposedPort {
  return { id: `exp-${++next}`, deviceId, port, label, address: address(deviceId, port) }
}

/**
 * The user's exposed ports, one list for the whole gallery as a server
 * keeps one: a port exposed on a device's page is exposed for the Providers
 * specimen too, whose failed test then offers Try Again. alex-mbp's 11434,
 * which the Ollama entry names, starts not exposed.
 */
const ports = ref<SettingsExposedPort[]>([row('mac', 8188, 'ComfyUI'), row('mac', 8000, ''), row('cloud', 3000, 'Preview')])

/**
 * The gallery's exposed ports and what each control does to them, as the
 * backend would:
 *
 * - Expose Port with a port exposes it, which is done too when it is exposed
 *   already.
 * - Expose Port without a port adds a row with the port missing.
 * - A port another row of the device exposes is refused in the backend's words.
 * - Remove drops the row.
 */
export function useGalleryExposedPorts() {
  function isExposed(deviceId: string, port: number): boolean {
    return ports.value.some((exposed) => exposed.deviceId === deviceId && exposed.port === port)
  }

  async function expose(deviceId: string, port?: number): Promise<string> {
    const exposed = port === undefined ? undefined : ports.value.find((candidate) => candidate.deviceId === deviceId && candidate.port === port)
    if (exposed) {
      return exposed.id
    }
    const added = row(deviceId, port ?? null, '')
    ports.value = [...ports.value, added]
    return added.id
  }

  async function change(id: string, change: ExposedPortChange): Promise<void> {
    const exposed = ports.value.find((candidate) => candidate.id === id)
    if (!exposed) {
      return
    }
    const port = change.port === undefined ? exposed.port : change.port
    const taken = port !== null && ports.value.some((other) => other.id !== id && other.deviceId === exposed.deviceId && other.port === port)
    if (taken) {
      throw new Error(`Port ${port} is already exposed.`)
    }
    const label = change.label ?? exposed.label
    ports.value = ports.value.map((candidate) =>
      candidate.id === id ? { ...candidate, port, label, address: address(candidate.deviceId, port) } : candidate,
    )
  }

  async function remove(id: string): Promise<void> {
    ports.value = ports.value.filter((exposed) => exposed.id !== id)
  }

  return { ports, isExposed, expose, change, remove }
}
