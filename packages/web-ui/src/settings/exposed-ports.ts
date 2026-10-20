import { t } from '../devices/locales/text'
import type { SentenceText } from '../ui/ui-text'

/**
 * A port of one of the user's devices that Demi's own requests for the
 * user may reach (`crates/backend-user-shard/src/shard/exposed_ports.rs`), as the device's page
 * shows and edits it.
 */
export interface SettingsExposedPort {
  id: string
  deviceId: string
  /** Null until the user enters it: the row exposes nothing meanwhile. */
  port: number | null
  /** Empty for none. */
  label: string
  /** `http://<device id>:<port>`, which a provider entry's endpoint names; null while the port is missing. */
  address: string | null
}

/** What one edit of an exposed port changes. */
export type ExposedPortChange = Partial<Pick<SettingsExposedPort, 'port' | 'label'>>

/** The row's name: *11434 · Ollama*, the port alone without a label, and what is missing without a port. */
export function exposedPortName(exposed: SettingsExposedPort): SentenceText {
  if (exposed.port === null) {
    return t('exposed.missing')
  }
  return exposed.label ? `${exposed.port} · ${exposed.label}` : String(exposed.port)
}

/** What is wrong with a typed port, or null for one from 1 to 65535; empty leaves it missing. */
export function exposedPortProblem(value: string): SentenceText | null {
  const typed = value.trim()
  if (typed === '') {
    return null
  }
  const port = Number(typed)
  return /^\d+$/.test(typed) && port >= 1 && port <= 65535 ? null : t('exposed.portProblem')
}
