import { z } from 'zod'
import type { ExposeCall } from '@demicodes/plugin-expose'
import { apiRequest, jsonBody, readResponse } from './client'

/** The list comes with the `expose` plugin's state on the channel; only its methods go through these. */

/** Calls the `expose` plugin's method `method` for the expose `id`, which answers nothing. */
async function call(method: 'renew' | 'remove', id: string, signal?: AbortSignal): Promise<void> {
  const response = await apiRequest(`/plugins/expose/calls/${method}`, {
    method: 'POST',
    signal,
    ...jsonBody({ expose: id } satisfies ExposeCall),
  })
  await readResponse(response, z.null())
}

/** Moves the expose's expiry to one hour from now. */
export function renewExpose(id: string, signal?: AbortSignal): Promise<void> {
  return call('renew', id, signal)
}

/** Destroys the expose and ends its connections. */
export function removeExpose(id: string, signal?: AbortSignal): Promise<void> {
  return call('remove', id, signal)
}
