import { apiRequest, readResponse } from './client'
import { exposeAnswerSchema } from './generated/web-api'

/** The list comes from the `GET /api/state` snapshot; only writes go through these. */

/** Moves the expose's expiry to one hour from now (`POST /exposes/:id/renew`). */
export async function renewExpose(
  id: string,
  signal?: AbortSignal,
): Promise<void> {
  const response = await apiRequest(`/exposes/${encodeURIComponent(id)}/renew`, {
    method: 'POST',
    signal,
  })
  await readResponse(response, exposeAnswerSchema)
}

/** Destroys the expose and ends its connections (`DELETE /exposes/:id`). */
export async function removeExpose(
  id: string,
  signal?: AbortSignal,
): Promise<void> {
  await apiRequest(`/exposes/${encodeURIComponent(id)}`, {
    method: 'DELETE',
    signal,
  })
}
