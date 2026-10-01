import { apiRequest, jsonBody, readResponse } from './client'
import {
  draftAnswerSchema,
  type ConversationDraft,
  type DraftSave,
  type ReplacedDraftAction,
} from './generated/web-api'

/** A conversation's draft (`web-api.md` § Conversation drafts). */
function draftPath(conversationId: string): string {
  return `/conversations/${encodeURIComponent(conversationId)}/draft`
}

export async function loadDraft(conversationId: string, signal: AbortSignal): Promise<ConversationDraft> {
  const response = await apiRequest(draftPath(conversationId), { signal })
  return (await readResponse(response, draftAnswerSchema)).draft
}

/**
 * Saves the draft. A save sent while the page closes is `keepalive`, so the
 * web browser delivers it after the page is gone; nobody reads its answer then.
 */
export async function saveDraft(
  conversationId: string,
  save: DraftSave,
  options: { signal: AbortSignal; keepalive?: boolean },
): Promise<ConversationDraft> {
  const response = await apiRequest(draftPath(conversationId), {
    method: 'PUT',
    signal: options.signal,
    keepalive: options.keepalive,
    ...jsonBody(save),
  })
  return (await readResponse(response, draftAnswerSchema)).draft
}

/** Restores or dismisses the replaced version the request names. */
export async function changeReplacedDraft(
  conversationId: string,
  request: ReplacedDraftAction,
  signal: AbortSignal,
): Promise<ConversationDraft> {
  const response = await apiRequest(`${draftPath(conversationId)}/replaced`, {
    method: 'POST',
    signal,
    ...jsonBody(request),
  })
  return (await readResponse(response, draftAnswerSchema)).draft
}
