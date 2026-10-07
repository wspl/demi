import { useRouter } from 'vue-router'
import type { CreateWorkspace } from '../api/generated/web-api'
import { useResources } from '../state/resources'
import { useConversations } from './store'

/**
 * The page's ways into a conversation, shared by the sidebar, the keyboard
 * shortcuts and the New Project dialog, so each opens one the same way.
 */
export function useConversationNavigation() {
  const router = useRouter()
  const resources = useResources()
  const conversations = useConversations()

  /** Shows the conversation; on a narrow layout the sidebar gives way to it. */
  function open(id: string): void {
    resources.sidebarOpen = false
    void router.push(`/chat/${id}`)
  }

  /**
   * Opens a new conversation in the project, or outside any: a draft that
   * the first send makes a conversation. New on the empty draft already
   * shown gives it back with the focus in its composer.
   */
  function create(projectId: string | null): void {
    const id = conversations.create(projectId)
    if (router.currentRoute.value.params.id === id) {
      conversations.composerFocusRequests += 1
    }
    open(id)
  }

  /**
   * Creates the project and opens a new conversation in it, as the project's
   * own New Conversation does (`product.md` § Conversations and projects).
   */
  async function createProject(draft: CreateWorkspace): Promise<void> {
    create(await resources.createProject(draft))
  }

  return { open, create, createProject }
}
