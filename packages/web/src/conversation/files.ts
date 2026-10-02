import { computed, effectScope, onScopeDispose, watch, type EffectScope } from 'vue'
import { useDocumentVisibility } from '@vueuse/core'
import type { ConversationFileService } from '@demicodes/web-ui/plugins/page'
import { conversationFileRoutes, fileSource } from '../api/files'
import { useProduct } from '../state/product'
import { useResources } from '../state/resources'
import { executionFor } from '../targets/execution'
import { readEditCopies } from './changes'
import { useConversations } from './store'
import { useWorkPanel } from './work'

/** Whether the page is visible: one listener, for the page's lifetime, that every service shares. */
const pageVisibility = useDocumentVisibility()

/** Each conversation's service, made once for the page's lifetime. */
const services = new Map<string, ConversationFileService>()

/**
 * A conversation's files (`plugin-pages.md` § The conversation files
 * service): the Host's tree and contents while its device and directory are
 * known, and the working tree's changes, which the service lists again by
 * itself while anything shows them: when the conversation's working-tree
 * revision changes, since a job ended, and when the page is shown again,
 * since the user may have changed files outside Demi meanwhile
 * (`web-api.md` § File text and working tree changes).
 */
export function conversationFiles(conversationId: string): ConversationFileService {
  const known = services.get(conversationId)
  if (known) {
    return known
  }
  const conversations = useConversations()
  const product = useProduct()
  const resources = useResources()
  const work = useWorkPanel()
  const summary = computed(() => conversations.items.find((item) => item.id === conversationId) ?? null)
  const execution = computed(() => (summary.value ? executionFor(summary.value) : null))
  const workspace = computed(() => {
    const target = execution.value
    const device = target ? resources.deviceById(target.deviceId) : null
    if (!target || !device || !target.path) {
      return null
    }
    return {
      source: fileSource(conversationFileRoutes(conversationId), device),
      root: target.path,
      // The Cloud's own session directory has no name worth showing; it is the workspace.
      name: target.directory === null ? 'Workspace' : undefined,
    }
  })
  const changes = work.stateFor(conversationId).changes
  /** The summary's count of the conversation's ended jobs, as the sync channel brings it. */
  const revision = () =>
    product.snapshot?.conversations.find((entry) => entry.id === conversationId)?.workingTreeRevision ?? null
  /** The revision the list last followed; a change while nothing showed it makes the list stale. */
  let followed = revision()
  let showing = 0
  let watching: EffectScope | null = null

  function showChanges(): void {
    showing += 1
    if (showing === 1) {
      if (revision() !== followed) {
        changes.markStale()
      }
      changes.ensureFresh()
      watching = effectScope(true)
      watching.run(() => {
        watch(revision, (next) => {
          followed = next
          changes.refresh()
        })
        watch(pageVisibility, (state) => {
          if (state === 'visible') {
            changes.refresh()
          }
        })
      })
    }
    onScopeDispose(() => {
      showing -= 1
      if (showing === 0) {
        followed = revision()
        watching?.stop()
        watching = null
      }
    })
  }

  const service: ConversationFileService = {
    get workspace() {
      return workspace.value
    },
    get root() {
      return execution.value?.path ?? null
    },
    changes,
    edit: readEditCopies,
    showChanges,
  }
  services.set(conversationId, service)
  return service
}
