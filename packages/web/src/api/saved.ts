import { apiRequest, jsonBody, readResponse } from './client'
import {
  fromDraftAnswerSchema,
  savedAnswerSchema,
  savedMessageSchema,
  savedRemovedSchema,
  type FromDraftAnswer,
  type SavedRemoved,
  type SavedAnswer,
  type SavedFromDraft,
  type SavedMessage,
  type SavedSave,
  type SavedWrite,
} from './generated/web-api'

/** A conversation's messages saved for later. */
function savedPath(conversationId: string, messageId?: string): string {
  const list = `/conversations/${encodeURIComponent(conversationId)}/saved`
  return messageId === undefined ? list : `${list}/${encodeURIComponent(messageId)}`
}

/**
 * Reads the saved message `messageId` whole, as a use of it needs, where the
 * list holds only its opening.
 */
export async function readSaved(conversationId: string, messageId: string, signal: AbortSignal): Promise<SavedMessage> {
  const response = await apiRequest(savedPath(conversationId, messageId), { signal })
  return readResponse(response, savedMessageSchema)
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

/**
 * Deletes the message; one gone already answers the same. Answers the
 * revisions after it, and the message as it was, whole, null when it was gone.
 */
export async function deleteSaved(conversationId: string, messageId: string, signal: AbortSignal): Promise<SavedRemoved> {
  const response = await apiRequest(savedPath(conversationId, messageId), { method: 'DELETE', signal })
  return readResponse(response, savedRemovedSchema)
}
