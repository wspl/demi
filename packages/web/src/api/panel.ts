import { panelStateSchema, type PanelState } from '@demicodes/web-ui/agent/panel-tabs'
import { apiRequest, jsonBody, readResponse } from './client'

/** The conversation's saved work panel (`web-api.md` § Work panel state). */
function panelPath(conversationId: string): string {
  return `/conversations/${encodeURIComponent(conversationId)}/panel`
}

export async function loadPanel(conversationId: string): Promise<PanelState> {
  const response = await apiRequest(panelPath(conversationId))
  return readResponse(response, panelStateSchema)
}

export async function savePanel(conversationId: string, state: PanelState): Promise<void> {
  await apiRequest(panelPath(conversationId), { method: 'PUT', ...jsonBody(state) })
}
