import type { z } from 'zod'
import {
  controlResponseSchema,
  controlResults,
  type ControlApi,
  type ControlMethod,
  type ControlRequest,
} from './protocol'

interface PendingCall {
  resolve: (value: unknown) => void
  reject: (error: Error) => void
  timer: ReturnType<typeof setTimeout>
}

export interface ControlConnection extends ControlApi {
  close(): void
}

export interface ControlClientOptions {
  signal?: AbortSignal
  /** Bounds both socket opening and each response wait. */
  timeoutMs?: number
}

/** The host owns this connection and closes it when its workspace is disposed. */
export function connectControlClient(
  url: string,
  options: ControlClientOptions = {},
): Promise<ControlConnection> {
  return new Promise((resolve, reject) => {
    options.signal?.throwIfAborted()
    const timeoutMs = options.timeoutMs ?? 15_000
    if (
      !Number.isSafeInteger(timeoutMs) ||
      timeoutMs <= 0 ||
      timeoutMs > 2_147_483_647
    ) {
      throw new Error(
        'Control timeout must be an integer from 1 to 2147483647 milliseconds',
      )
    }
    const socket = new WebSocket(url)
    const pending = new Map<number, PendingCall>()
    let nextId = 1
    let phase: 'connecting' | 'open' | 'closed' = 'connecting'

    function finish(error: Error): void {
      if (phase === 'closed') {
        return
      }
      phase = 'closed'
      clearTimeout(openTimer)
      socket.removeEventListener('open', opened)
      socket.removeEventListener('message', received)
      socket.removeEventListener('close', closed)
      socket.removeEventListener('error', failed)
      options.signal?.removeEventListener('abort', aborted)
      for (const waiter of pending.values()) {
        clearTimeout(waiter.timer)
        waiter.reject(error)
      }
      pending.clear()
      reject(error)
      socket.close()
    }

    function received(event: MessageEvent): void {
      try {
        if (typeof event.data !== 'string') {
          throw new Error('Expected a text control response')
        }
        const response = controlResponseSchema.parse(JSON.parse(event.data))
        const waiter = pending.get(response.id)
        // Late responses after a timeout and duplicate replies cannot settle another call.
        if (!waiter) {
          return
        }
        clearTimeout(waiter.timer)
        if (response.ok) {
          waiter.resolve(response.result)
        } else {
          waiter.reject(new Error(response.error))
        }
        pending.delete(response.id)
      } catch {
        // Malformed envelopes and method results fail the connection without logging payloads.
        finish(new Error('Invalid control response'))
      }
    }

    function call<T>(
      method: ControlMethod,
      params: unknown,
      schema: z.ZodType<T>,
    ): Promise<T> {
      if (phase !== 'open') {
        return Promise.reject(new Error('Control socket is closed'))
      }
      return new Promise((settle, fail) => {
        const id = nextId++
        const timer = setTimeout(() => {
          pending.delete(id)
          fail(new Error(`Control ${method} timed out`))
        }, timeoutMs)
        pending.set(id, {
          resolve: (value) => settle(schema.parse(value)),
          reject: fail,
          timer,
        })
        const request: ControlRequest = { id, method, params }
        try {
          socket.send(JSON.stringify(request))
        } catch {
          finish(new Error('Control socket failed to send'))
        }
      })
    }

    function opened(): void {
      if (phase !== 'connecting') {
        return
      }
      phase = 'open'
      clearTimeout(openTimer)
      socket.removeEventListener('open', opened)
      resolve({
        listProviders: () =>
          call('listProviders', undefined, controlResults.listProviders),
        listModels: (params) =>
          call('listModels', params, controlResults.listModels),
        prepareSession: (params) =>
          call('prepareSession', params, controlResults.prepareSession),
        defaultWorkspace: () =>
          call('defaultWorkspace', undefined, controlResults.defaultWorkspace),
        close: () => finish(new Error('Control socket closed')),
      })
    }
    const closed = () => finish(new Error('Control socket closed'))
    const failed = () => finish(new Error('Control socket failed'))
    const aborted = () => finish(new Error('Control connection aborted'))
    const openTimer = setTimeout(
      () => finish(new Error('Control connection timed out')),
      timeoutMs,
    )
    socket.addEventListener('open', opened)
    socket.addEventListener('message', received)
    socket.addEventListener('close', closed)
    socket.addEventListener('error', failed)
    options.signal?.addEventListener('abort', aborted, { once: true })
  })
}
