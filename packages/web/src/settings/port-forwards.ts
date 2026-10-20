import { t } from './locales/text'
import { computed } from 'vue'
import { defineStore } from 'pinia'
import { reportError } from '@demicodes/web-ui/infra/errors'
import type { ForwardHost, PortForwardChange, SettingsPortForward } from '@demicodes/web-ui/settings/port-forwards'
import { apiRequest, ApiError, jsonBody, readResponse } from '../api/client'
import { portForwardAnswerSchema, type PortForwardDto, type PortForwardPatch } from '../api/generated/web-api'
import { useProduct, type Answer } from '../state/product'
import { deviceMarkOf, hostKindOf, hostLabel } from '../state/hosts'

/** The forwards oldest first, as the backend lists them: `forward` in its place by id. */
function withForward(forwards: PortForwardDto[], forward: PortForwardDto): PortForwardDto[] {
  return forwards.some((candidate) => candidate.id === forward.id)
    ? forwards.map((candidate) => (candidate.id === forward.id ? forward : candidate))
    : [...forwards, forward]
}

function forwardsAnswer(next: (current: PortForwardDto[]) => PortForwardDto[]): Answer {
  return (read) => ({ type: 'port_forwards', portForwards: next(read.portForwards) })
}

/** A refusal the user fixes in the field, such as a taken name or a port out of range. */
function refusedField(error: unknown): boolean {
  return error instanceof ApiError && error.status >= 400 && error.status < 500 && error.status !== 404
}

/**
 * The user's port forwards as Settings › Port Forwarding changes them
 * (`crates/runner-protocol/src/forward.rs`): each change shows its answer,
 * and the `port_forwards` part brings it, and every state the runners
 * announce, to every page.
 */
export const usePortForwardSettings = defineStore('port-forward-settings', () => {
  const product = useProduct()

  const forwards = computed<SettingsPortForward[]>(() => product.snapshot?.portForwards ?? [])
  /** The user's Hosts, the Cloud first, as a forward's ends are chosen among them. */
  const hosts = computed<ForwardHost[]>(() => {
    const devices = product.snapshot?.devices ?? []
    return devices
      .map((device): ForwardHost => {
        const kind = hostKindOf(device)
        return { id: device.id, name: hostLabel(kind, device), kind, state: device.state, mark: deviceMarkOf(device, devices) }
      })
      .sort((a, b) => Number(b.kind === 'cloud') - Number(a.kind === 'cloud'))
  })

  /** Add Forward: one at once, with the backend's defaults; answers its id, or null when it failed. */
  async function add(): Promise<string | null> {
    const at = product.sent()
    try {
      const response = await apiRequest('/port-forwards', { method: 'POST', ...jsonBody({} satisfies PortForwardPatch) })
      const { forward } = await readResponse(response, portForwardAnswerSchema)
      product.answered(at, forwardsAnswer((current) => withForward(current, forward)))
      return forward.id
    } catch (error) {
      reportError(t('portForwards.addFailed'), error, { userVisible: true })
      return null
    }
  }

  /** Writes a change; a refusal the user can fix is thrown for its field, any other failure is a toast. */
  async function change(id: string, change: PortForwardChange): Promise<void> {
    const at = product.sent()
    try {
      const response = await apiRequest(`/port-forwards/${encodeURIComponent(id)}`, {
        method: 'PATCH',
        ...jsonBody(change satisfies PortForwardPatch),
      })
      const { forward } = await readResponse(response, portForwardAnswerSchema)
      product.answered(at, forwardsAnswer((current) => withForward(current, forward)))
    } catch (error) {
      if (refusedField(error)) {
        throw error
      }
      reportError(t('portForwards.saveFailed'), error, { userVisible: true })
    }
  }

  /** Try Again: the Listen On runner tries the port again and announces what it finds. */
  async function tryAgain(id: string): Promise<void> {
    try {
      await apiRequest(`/port-forwards/${encodeURIComponent(id)}/retry`, { method: 'POST' })
    } catch (error) {
      reportError(t('portForwards.retryFailed'), error, { userVisible: true })
    }
  }

  async function remove(id: string): Promise<void> {
    const at = product.sent()
    try {
      await apiRequest(`/port-forwards/${encodeURIComponent(id)}`, { method: 'DELETE' })
      product.answered(at, forwardsAnswer((current) => current.filter((forward) => forward.id !== id)))
    } catch (error) {
      reportError(t('portForwards.removeFailed'), error, { userVisible: true })
    }
  }

  return { forwards, hosts, add, change, tryAgain, remove }
})
