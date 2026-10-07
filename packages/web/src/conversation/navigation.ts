import { z } from 'zod'
import { useRouter, type RouteLocationRaw } from 'vue-router'
import type { CreateWorkspace } from '../api/generated/web-api'
import { useResources } from '../state/resources'
import { useConversations } from './store'

/**
 * What a history entry that showed a new conversation keeps, the project
 * the conversation was started in: a new conversation has no record before
 * its first send, so a reload of the entry, which finds nothing at its
 * address, opens a new conversation again (`web-application.md` § What the
 * reload opens). The web browser keeps it with the entry across a reload.
 */
const newConversationEntrySchema = z.object({
  newConversation: z.object({ projectId: z.string().nullable() }),
})

/** The address of the new conversation `id`, with the mark its history entry keeps. */
function newConversationAt(id: string, projectId: string | null): RouteLocationRaw {
  return { path: `/chat/${id}`, state: { newConversation: { projectId } } }
}

/**
 * The page's ways into a conversation, shared by the sidebar, the keyboard
 * shortcuts, the search window and the New Project dialog, so each opens one
 * the same way.
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
   * Shows the conversation a search found, scrolled to the message that
   * matched, which is marked for a moment; at its end when only its title
   * matched.
   */
  function openFound(id: string, blockId: string | null): void {
    conversations.reveal = blockId === null ? null : { conversationId: id, blockId }
    open(id)
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
    resources.sidebarOpen = false
    void router.push(newConversationAt(id, projectId))
  }

  /**
   * Shows a new conversation, in the project or outside any, in place of
   * the address shown, as the chat's own address and a conversation
   * deleted while it shows do.
   */
  function replaceWithNew(projectId: string | null = null): void {
    void router.replace(newConversationAt(conversations.create(projectId), projectId))
  }

  /**
   * The address names a conversation the page does not have: when its
   * history entry showed a new conversation, as a reload of it finds, a new
   * conversation in the same project takes its place, or outside any when
   * the project is gone. Answers whether one did.
   */
  function reopenNew(): boolean {
    const entry = newConversationEntrySchema.safeParse(router.options.history.state)
    if (!entry.success) {
      return false
    }
    const { projectId } = entry.data.newConversation
    replaceWithNew(resources.projects.some((project) => project.id === projectId) ? projectId : null)
    return true
  }

  /**
   * Creates the project and opens a new conversation in it, as the project's
   * own New Conversation does (`product.md` § Conversations and projects).
   */
  async function createProject(draft: CreateWorkspace): Promise<void> {
    create(await resources.createProject(draft))
  }

  return { open, openFound, create, replaceWithNew, reopenNew, createProject }
}
