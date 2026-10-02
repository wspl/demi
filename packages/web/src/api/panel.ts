import type { PanelBackend, PanelChange } from '@demicodes/web-ui/agent/panel-changes'
import { apiRequest, jsonBody, readResponse } from './client'
import {
  createPanelTabSchema,
  movePanelTabSchema,
  panelRevisionSchema,
  updatePanelTabSchema,
  workPanelSchema,
} from './generated/web-api'

/** The conversation's work panel over its routes (`web-api.md` § Work panel state). */
export function panelBackend(conversationId: string): PanelBackend {
  const panel = `/conversations/${encodeURIComponent(conversationId)}/panel`
  const tab = (id: string) => `${panel}/tabs/${encodeURIComponent(id)}`

  async function send(change: PanelChange): Promise<{ revision: number }> {
    switch (change.type) {
      case 'create': {
        // A kind's data is JSON the page made; the schema says so before it leaves.
        const body = createPanelTabSchema.parse({
          id: change.tab.id,
          kind: change.tab.kind,
          data: change.tab.data,
          ...(change.index === undefined ? {} : { index: change.index }),
        })
        return readResponse(await apiRequest(`${panel}/tabs`, { method: 'POST', ...jsonBody(body) }), panelRevisionSchema)
      }
      case 'update': {
        const body = updatePanelTabSchema.parse({ data: change.data })
        return readResponse(await apiRequest(tab(change.id), { method: 'PATCH', ...jsonBody(body) }), panelRevisionSchema)
      }
      case 'remove':
        return readResponse(await apiRequest(tab(change.id), { method: 'DELETE' }), panelRevisionSchema)
      case 'move': {
        const body = movePanelTabSchema.parse({ index: change.index })
        return readResponse(await apiRequest(`${tab(change.id)}/move`, { method: 'POST', ...jsonBody(body) }), panelRevisionSchema)
      }
    }
  }

  return {
    read: async () => readResponse(await apiRequest(panel), workPanelSchema),
    send,
  }
}
