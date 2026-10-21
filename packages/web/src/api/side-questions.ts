import { apiRequest, jsonBody, readResponse } from './client'
import type { SideQuestion } from '@demicodes/protocol'
import {
  askRequestSchema,
  sideFollowUpSchema,
  sideQuestionAnswerSchema,
  sideQuestionPatchSchema,
  type AskRequest,
  type SideQuestionAnswer,
} from './generated/web-api'

/** A side question as an answer names it, with its id. */
export type AnsweredSide = SideQuestion & { id: string }

function answered(answer: SideQuestionAnswer): AnsweredSide {
  return { id: answer.id, ...answer.sideQuestion }
}

/** A conversation's side question routes. */
function sidesPath(conversationId: string): string {
  return `/conversations/${encodeURIComponent(conversationId)}/side-questions`
}

/** Asks the side question under the id the page chose; the same request again finds it. */
export async function askSideQuestion(conversationId: string, request: AskRequest): Promise<AnsweredSide> {
  const response = await apiRequest(sidesPath(conversationId), {
    method: 'POST',
    ...jsonBody(askRequestSchema.parse(request)),
  })
  return answered(await readResponse(response, sideQuestionAnswerSchema))
}

export async function followUpSideQuestion(conversationId: string, sideId: string, text: string): Promise<void> {
  await apiRequest(`${sidesPath(conversationId)}/${encodeURIComponent(sideId)}/messages`, {
    method: 'POST',
    ...jsonBody(sideFollowUpSchema.parse({ text })),
  })
}

export async function retrySideQuestion(conversationId: string, sideId: string): Promise<void> {
  await apiRequest(`${sidesPath(conversationId)}/${encodeURIComponent(sideId)}/retry`, { method: 'POST' })
}

export async function foldSideQuestion(conversationId: string, sideId: string, folded: boolean): Promise<AnsweredSide> {
  const response = await apiRequest(`${sidesPath(conversationId)}/${encodeURIComponent(sideId)}`, {
    method: 'PATCH',
    ...jsonBody(sideQuestionPatchSchema.parse({ folded })),
  })
  return answered(await readResponse(response, sideQuestionAnswerSchema))
}
