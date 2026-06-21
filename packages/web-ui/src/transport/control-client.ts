import type { ProviderConfig } from '@demi/agent/client'
import type {
  ControlApi,
  ControlMethod,
  ControlRequest,
  ControlResponse,
  ModelInfo,
  PrepareSessionParams,
  ProviderInfo,
  WorkspaceInfo,
} from './protocol'

interface PendingCall {
  resolve: (value: unknown) => void
  reject: (error: Error) => void
}

export function connectControlClient(url: string): Promise<ControlApi> {
  return new Promise((resolve, reject) => {
    const socket = new WebSocket(url)
    const pending = new Map<number, PendingCall>()
    let nextId = 1

    socket.addEventListener('message', (event) => {
      const text = typeof event.data === 'string' ? event.data : String(event.data)
      const response = JSON.parse(text) as ControlResponse
      const waiter = pending.get(response.id)
      if (!waiter) return
      pending.delete(response.id)
      if (response.ok) waiter.resolve(response.result)
      else waiter.reject(new Error(response.error))
    })

    socket.addEventListener('close', () => {
      for (const waiter of pending.values()) waiter.reject(new Error('Control socket closed'))
      pending.clear()
    })

    socket.addEventListener('error', () => reject(new Error('Control socket failed to connect')), { once: true })

    function call(method: ControlMethod, params: unknown): Promise<unknown> {
      return new Promise((settle, fail) => {
        const id = nextId++
        pending.set(id, { resolve: settle, reject: fail })
        const request: ControlRequest = { id, method, params }
        socket.send(JSON.stringify(request))
      })
    }

    socket.addEventListener(
      'open',
      () => {
        const api: ControlApi = {
          listProviders: () => call('listProviders', undefined) as Promise<ProviderInfo[]>,
          listModels: (params) => call('listModels', params) as Promise<ModelInfo[]>,
          prepareSession: (params: PrepareSessionParams) => call('prepareSession', params) as Promise<ProviderConfig>,
          defaultWorkspace: () => call('defaultWorkspace', undefined) as Promise<WorkspaceInfo>,
        }
        resolve(api)
      },
      { once: true },
    )
  })
}
