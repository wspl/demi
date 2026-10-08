import { computed, effectScope, onScopeDispose, watch } from 'vue'
import { emptyChangeSet, type ChangeSetSource } from '@demicodes/web-ui/files/changes'
import { findRequest } from '@demicodes/web-ui/files/request-changes'
import { keptChangeSet } from '@demicodes/web-ui/files/kept-source'
import type { ConversationFileService } from '@demicodes/web-ui/plugins/page'
import { conversationFileRoutes, fileSource, hostFiles } from '../api/files'
import { useResources } from '../state/resources'
import { executionFor } from '../targets/execution'
import { readEditCopies, workingTreeReads } from './changes'
import { ConversationWatch } from './file-watch'
import { directRoute } from '../direct'
import { directFileReads, directWatchLink } from '../direct/operations'
import { useConversations } from './store'

/** Each conversation's service, made once for the page's lifetime. */
const services = new Map<string, ConversationFileService>()

/**
 * A conversation's files (`plugin-pages.md` § The conversation files
 * service): the Host's tree and contents while its device and directory are
 * known, the working tree's changes, and a request's changes from the
 * transcripts the page holds. What it reads it keeps with the
 * Host's other files, and while a component shows any of them its file
 * watch is open, whose reports have what they name read again
 * (`plugin-pages.md` § What the service keeps).
 */
export function conversationFiles(conversationId: string): ConversationFileService {
  const known = services.get(conversationId)
  if (known) {
    return known
  }
  const conversations = useConversations()
  const resources = useResources()
  const summary = computed(() => conversations.items.find((item) => item.id === conversationId) ?? null)
  const execution = computed(() => (summary.value ? executionFor(summary.value) : null))
  const device = computed(() => {
    const target = execution.value
    return target ? resources.deviceById(target.deviceId) : null
  })
  const watcher = new ConversationWatch(conversationId, () => {
    const target = execution.value
    return target?.path && device.value ? { files: hostFiles(device.value.id), root: target.path } : null
  }, directWatchLink(() => directRoute(conversationId)))
  // For the page's lifetime: a Host that comes back online has the watch
  // connect at once, and the watch moves whenever the path to its device
  // changes (`direct-channel.md` § Choosing the path).
  effectScope(true).run(() => {
    watch(() => device.value?.state === 'online' ? device.value.id : null, (online, _previous, onCleanup) => {
      if (online === null)
        return
      watcher.online()
      const moving = directRoute(conversationId)?.device.onChange(() => watcher.move())
      onCleanup(() => moving?.())
    }, { immediate: true })
  })
  const workspace = computed(() => {
    const target = execution.value
    if (!target || !device.value || !target.path) {
      return null
    }
    return {
      source: fileSource(conversationFileRoutes(conversationId), device.value, watcher, (reads) =>
        directFileReads(() => directRoute(conversationId), reads)),
      root: target.path,
      // The Cloud's own session directory has no name worth showing; it is the workspace.
      name: target.directory === null ? 'Workspace' : undefined,
    }
  })
  // The working tree's place, as plain values: a product state that leaves
  // the Host and the directory as they are keeps the change set, so what
  // shows it is not shown again, which would read an unconfirmed list anew.
  const deviceId = computed(() => device.value?.id ?? null)
  const root = computed(() => execution.value?.path ?? null)
  const changes = computed(() => {
    if (!root.value || deviceId.value === null) {
      return null
    }
    return keptChangeSet(workingTreeReads(conversationId), root.value, {
      files: hostFiles(deviceId.value),
      follower: watcher,
    })
  })

  const service: ConversationFileService = {
    get workspace() {
      return workspace.value
    },
    get root() {
      return root.value
    },
    get changes(): ChangeSetSource {
      return changes.value ?? emptyChangeSet
    },
    edit: readEditCopies,
    request(node, request) {
      // The conversation's transcripts as the page holds them: its own agent's and each subagent's.
      const conversation = conversations.items.find((item) => item.id === conversationId)
      return conversation ? findRequest(conversation, node, request) : null
    },
    showChanges() {
      // The list shows for the calling component while it lives, for the Host and root it has now.
      const scope = effectScope()
      scope.run(() => {
        watch(changes, (set, _previous, onCleanup) => {
          const showing = set?.show()
          onCleanup(() => showing?.release())
        }, { immediate: true })
      })
      onScopeDispose(() => scope.stop())
    },
  }
  services.set(conversationId, service)
  return service
}
