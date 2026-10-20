import { apiRequest, jsonBody, readResponse } from './client'
import {
  conversationPermissionsSchema,
  decidePermissionSchema,
  type ConversationPermissions,
  type PermissionDecision,
  type RequestedValues,
} from './generated/web-api'

/** A conversation's permission routes. */
function permissionsPath(conversationId: string): string {
  return `/conversations/${encodeURIComponent(conversationId)}/permissions`
}

export async function readPermissions(conversationId: string): Promise<ConversationPermissions> {
  const response = await apiRequest(permissionsPath(conversationId))
  return readResponse(response, conversationPermissionsSchema)
}

/**
 * Decides the request: an allow grants its category for the conversation, a
 * deny decides it alone, and an allow of a credential request carries the
 * credential's values, kept as the request says.
 */
export async function decidePermission(
  conversationId: string,
  requestId: string,
  decision: PermissionDecision,
  credential?: RequestedValues,
): Promise<void> {
  const body = decidePermissionSchema.parse({ decision, credential })
  await apiRequest(`${permissionsPath(conversationId)}/requests/${encodeURIComponent(requestId)}`, {
    method: 'POST',
    ...jsonBody(body),
  })
}
