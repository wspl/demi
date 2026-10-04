import { apiRequest, jsonBody, readResponse } from './client'
import {
  conversationPermissionsSchema,
  decidePermissionSchema,
  type ConversationPermissions,
  type PermissionDecision,
} from './generated/web-api'

/** A conversation's permission routes (`web-api.md` § Conversation permissions). */
function permissionsPath(conversationId: string): string {
  return `/conversations/${encodeURIComponent(conversationId)}/permissions`
}

export async function readPermissions(conversationId: string): Promise<ConversationPermissions> {
  const response = await apiRequest(permissionsPath(conversationId))
  return readResponse(response, conversationPermissionsSchema)
}

/** Decides the request: an allow grants its category for the conversation, a deny decides it alone. */
export async function decidePermission(
  conversationId: string,
  requestId: string,
  decision: PermissionDecision,
): Promise<void> {
  const body = decidePermissionSchema.parse({ decision })
  await apiRequest(`${permissionsPath(conversationId)}/requests/${encodeURIComponent(requestId)}`, {
    method: 'POST',
    ...jsonBody(body),
  })
}

export async function revokePermission(conversationId: string, category: string): Promise<void> {
  await apiRequest(`${permissionsPath(conversationId)}/grants/${encodeURIComponent(category)}`, {
    method: 'DELETE',
  })
}
