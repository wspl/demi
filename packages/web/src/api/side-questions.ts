import { apiRequest, jsonBody, readResponse } from './client'
import {
  askRequestSchema,
  sideFollowUpSchema,
  sideQuestionAnswerSchema,
  sideQuestionPatchSchema,
  sideQuestionsSchema,
  type AskRequest,
  type SideQuestion,
  type SideQuestions,
} from './generated/web-api'

/** A conversation's side question routes (`web-api-conversations.md` § Side questions). */
function sidesPath(conversationId: string): string {
  return `/conversations/${encodeURIComponent(conversationId)}/side-questions`
}

export async function readSideQuestions(conversationId: string): Promise<SideQuestions> {
  return readResponse(await apiRequest(sidesPath(conversationId)), sideQuestionsSchema)
}

/** Asks the side question under the id the page chose; the same request again finds it. */
export async function askSideQuestion(conversationId: string, request: AskRequest): Promise<SideQuestion> {
  const response = await apiRequest(sidesPath(conversationId), {
    method: 'POST',
    ...jsonBody(askRequestSchema.parse(request)),
  })
  return (await readResponse(response, sideQuestionAnswerSchema)).sideQuestion
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

export async function foldSideQuestion(conversationId: string, sideId: string, folded: boolean): Promise<SideQuestion> {
  const response = await apiRequest(`${sidesPath(conversationId)}/${encodeURIComponent(sideId)}`, {
    method: 'PATCH',
    ...jsonBody(sideQuestionPatchSchema.parse({ folded })),
  })
  return (await readResponse(response, sideQuestionAnswerSchema)).sideQuestion
}
