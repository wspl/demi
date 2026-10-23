import { apiRequest, jsonBody, readResponse } from './client'
import {
  draftAnswerSchema,
  draftWrittenSchema,
  type ConversationDraft,
  type DraftSave,
  type DraftWritten,
} from './generated/web-api'

/** A conversation's draft. */
function draftPath(conversationId: string): string {
  return `/conversations/${encodeURIComponent(conversationId)}/draft`
}

export async function loadDraft(conversationId: string, signal: AbortSignal): Promise<ConversationDraft> {
  const response = await apiRequest(draftPath(conversationId), { signal })
  return (await readResponse(response, draftAnswerSchema)).draft
}

/**
 * Saves the draft, over whatever the backend holds; answers it as saved, with
 * the revisions after the save.
 * A save sent while the page closes is `keepalive`, so the web browser
 * delivers it after the page is gone; nobody reads its answer then.
 */
export async function saveDraft(
  conversationId: string,
  save: DraftSave,
  options: { signal: AbortSignal; keepalive?: boolean },
): Promise<DraftWritten> {
  const response = await apiRequest(draftPath(conversationId), {
    method: 'PUT',
    signal: options.signal,
    keepalive: options.keepalive,
    ...jsonBody(save),
  })
  return readResponse(response, draftWrittenSchema)
}
