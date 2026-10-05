import type { PanelBackend, PanelChange } from '@demicodes/web-ui/agent/panel-changes'
import { apiRequest, jsonBody, readResponse } from './client'
import { panelChangesSchema, panelRevisionSchema, workPanelSchema } from './generated/web-api'

/** A change as the panel's route names it; the route's schema checks it before it leaves. */
function changeBody(change: PanelChange) {
  switch (change.type) {
    case 'create':
      return {
        op: 'create',
        id: change.tab.id,
        kind: change.tab.kind,
        data: change.tab.data,
        ...(change.index === undefined ? {} : { index: change.index }),
      }
    case 'update':
      return { op: 'update', id: change.id, data: change.data }
    case 'remove':
      return { op: 'delete', id: change.id }
    case 'move':
      return { op: 'move', id: change.id, index: change.index }
  }
}

/** The conversation's work panel over its routes (`web-api.md` § Work panel state). */
export function panelBackend(conversationId: string): PanelBackend {
  const panel = `/conversations/${encodeURIComponent(conversationId)}/panel`
  return {
    read: async () => readResponse(await apiRequest(panel), workPanelSchema),
    async send(changes) {
      // A kind's data is JSON the page made; the schema says so before it leaves.
      const body = panelChangesSchema.parse({ changes: changes.map(changeBody) })
      return readResponse(await apiRequest(`${panel}/changes`, { method: 'POST', ...jsonBody(body) }), panelRevisionSchema)
    },
  }
}
