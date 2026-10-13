import { apiRequest, jsonBody, readResponse } from './client'
import {
  draftAnswerSchema,
  fromDraftAnswerSchema,
  savedAnswerSchema,
  savedListSchema,
  type ConversationDraft,
  type FromDraftAnswer,
  type SavedFromDraft,
  type SavedList,
  type SavedMessage,
  type SavedSave,
} from './generated/web-api'

/** A conversation's messages saved for later (`web-api.md` § Messages saved for later). */
function savedPath(conversationId: string, messageId?: string): string {
  const list = `/conversations/${encodeURIComponent(conversationId)}/saved`
  return messageId === undefined ? list : `${list}/${encodeURIComponent(messageId)}`
}

export async function loadSaved(conversationId: string, signal: AbortSignal): Promise<SavedList> {
  const response = await apiRequest(savedPath(conversationId), { signal })
  return readResponse(response, savedListSchema)
}

/** Saves the composer's message for later; the backend empties the conversation's draft with it. */
export async function saveFromDraft(conversationId: string, save: SavedFromDraft, signal: AbortSignal): Promise<FromDraftAnswer> {
  const response = await apiRequest(`${savedPath(conversationId)}/from-draft`, { method: 'POST', signal, ...jsonBody(save) })
  return readResponse(response, fromDraftAnswerSchema)
}

/** Adds a message for later, leaving the draft as it is, as Undo after Delete does. */
export async function postSaved(conversationId: string, save: SavedSave, signal: AbortSignal): Promise<SavedMessage> {
  const response = await apiRequest(savedPath(conversationId), { method: 'POST', signal, ...jsonBody(save) })
  return (await readResponse(response, savedAnswerSchema)).message
}

/** Moves the message into the draft, and the draft into the list; answers the draft. */
export async function editSaved(conversationId: string, messageId: string, signal: AbortSignal): Promise<ConversationDraft> {
  const response = await apiRequest(`${savedPath(conversationId, messageId)}/edit`, { method: 'POST', signal })
  return (await readResponse(response, draftAnswerSchema)).draft
}

export async function deleteSaved(conversationId: string, messageId: string, signal: AbortSignal): Promise<void> {
  await apiRequest(savedPath(conversationId, messageId), { method: 'DELETE', signal })
}
