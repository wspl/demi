import { apiRequest, jsonBody, readResponse } from './client'
import {
  fromDraftAnswerSchema,
  savedAnswerSchema,
  savedListSchema,
  savedRemovedSchema,
  type FromDraftAnswer,
  type Revisions,
  type SavedAnswer,
  type SavedFromDraft,
  type SavedList,
  type SavedSave,
  type SavedWrite,
} from './generated/web-api'

/** A conversation's messages saved for later. */
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

/** Adds a message for later, leaving the draft as it is, as Undo after Delete and Retry of a message not saved do. */
export async function postSaved(conversationId: string, save: SavedSave, signal: AbortSignal): Promise<SavedAnswer> {
  const response = await apiRequest(savedPath(conversationId), { method: 'POST', signal, ...jsonBody(save) })
  return readResponse(response, savedAnswerSchema)
}

/** Writes an edit's Save: the message holds `write` in its place, and the draft is left as it is. */
export async function putSaved(conversationId: string, messageId: string, write: SavedWrite, signal: AbortSignal): Promise<SavedAnswer> {
  const response = await apiRequest(savedPath(conversationId, messageId), { method: 'PUT', signal, ...jsonBody(write) })
  return readResponse(response, savedAnswerSchema)
}

/** Deletes the message; one gone already answers the same. Answers the revisions after it. */
export async function deleteSaved(conversationId: string, messageId: string, signal: AbortSignal): Promise<Revisions> {
  const response = await apiRequest(savedPath(conversationId, messageId), { method: 'DELETE', signal })
  return (await readResponse(response, savedRemovedSchema)).revisions
}
