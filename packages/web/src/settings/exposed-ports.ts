import { t } from './locales/text'
import { computed } from 'vue'
import { defineStore } from 'pinia'
import { reportError } from '@demicodes/web-ui/infra/errors'
import type { ExposedPortChange, SettingsExposedPort } from '@demicodes/web-ui/settings/exposed-ports'
import { apiRequest, ApiError, jsonBody, readResponse } from '../api/client'
import {
  exposedPortAnswerSchema,
  type ExposedPortDto,
  type ExposedPortPatch,
  type NewExposedPort,
} from '../api/generated/web-api'
import { useProduct, type Answer } from '../state/product'

/** The rows oldest first, as the backend lists them: `exposed` in its place by id. */
function withPort(ports: ExposedPortDto[], exposed: ExposedPortDto): ExposedPortDto[] {
  return ports.some((candidate) => candidate.id === exposed.id)
    ? ports.map((candidate) => (candidate.id === exposed.id ? exposed : candidate))
    : [...ports, exposed]
}

function portsAnswer(next: (current: ExposedPortDto[]) => ExposedPortDto[]): Answer {
  return (read) => ({ type: 'exposed_ports', exposedPorts: next(read.exposedPorts) })
}

/** A refusal the user fixes in the field, such as a port another row exposes. */
function refusedField(error: unknown): boolean {
  return error instanceof ApiError && error.status >= 400 && error.status < 500 && error.status !== 404
}

/**
 * The user's exposed ports as a device's page and a failed request's
 * Expose Port change them (`port-exposure.md` § Exposed ports): each change
 * shows its answer, and the `exposed_ports` part brings it to every page.
 */
export const useExposedPortSettings = defineStore('exposed-port-settings', () => {
  const product = useProduct()

  const ports = computed<SettingsExposedPort[]>(() => product.snapshot?.exposedPorts ?? [])

  /** Whether the user's device `deviceId` exposes `port` now. */
  function isExposed(deviceId: string, port: number): boolean {
    return ports.value.some((exposed) => exposed.deviceId === deviceId && exposed.port === port)
  }

  /**
   * The name of the user's device `endpoint` names by its id, when every
   * user of an entry reaches it: on a shared instance, where the master's
   * entries are everyone's (`port-exposure.md` § Exposed ports, Whose).
   */
  function sharedDevice(endpoint: string): string | null {
    if (product.snapshot?.mode !== 'shared' || !URL.canParse(endpoint)) {
      return null
    }
    const host = new URL(endpoint).hostname
    return product.snapshot.devices.find((device) => device.id === host)?.name ?? null
  }

  /**
   * Exposes `port` of `deviceId`, which is done too when it is exposed
   * already, or adds a row with the port missing; answers its id, or null
   * when it failed.
   */
  async function expose(deviceId: string, port?: number): Promise<string | null> {
    const at = product.sent()
    try {
      const response = await apiRequest('/exposed-ports', {
        method: 'POST',
        ...jsonBody({ deviceId, ...(port === undefined ? {} : { port }) } satisfies NewExposedPort),
      })
      const answer = await readResponse(response, exposedPortAnswerSchema)
      product.answered(at, portsAnswer((current) => withPort(current, answer.port)))
      return answer.port.id
    } catch (error) {
      reportError(t('exposedPorts.exposeFailed'), error, { userVisible: true })
      return null
    }
  }

  /** Writes a change; a refusal the user can fix is thrown for its field, any other failure is a toast. */
  async function change(id: string, change: ExposedPortChange): Promise<void> {
    const at = product.sent()
    try {
      const response = await apiRequest(`/exposed-ports/${encodeURIComponent(id)}`, {
        method: 'PATCH',
        ...jsonBody(change satisfies ExposedPortPatch),
      })
      const answer = await readResponse(response, exposedPortAnswerSchema)
      product.answered(at, portsAnswer((current) => withPort(current, answer.port)))
    } catch (error) {
      if (refusedField(error)) {
        throw error
      }
      reportError(t('exposedPorts.saveFailed'), error, { userVisible: true })
    }
  }

  async function remove(id: string): Promise<void> {
    const at = product.sent()
    try {
      await apiRequest(`/exposed-ports/${encodeURIComponent(id)}`, { method: 'DELETE' })
      product.answered(at, portsAnswer((current) => current.filter((exposed) => exposed.id !== id)))
    } catch (error) {
      reportError(t('exposedPorts.removeFailed'), error, { userVisible: true })
    }
  }

  return { ports, isExposed, sharedDevice, expose, change, remove }
})
