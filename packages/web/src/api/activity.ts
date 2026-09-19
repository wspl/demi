import { apiRequest } from './client'

/**
 * A user operation in the live browser view is conversation activity
 * (`resource-lifecycle.md` § Activity); the view reports it at most every 30
 * seconds.
 */
export async function reportActivity(conversationId: string): Promise<void> {
  try {
    await apiRequest(`/conversations/${encodeURIComponent(conversationId)}/activity`, {
      method: 'POST',
    })
  } catch {
    // Activity is a hint: the next operation reports again.
  }
}
