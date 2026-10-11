import { computed, effectScope, onScopeDispose, watch } from 'vue'
import { useOnScreen } from '@demicodes/web-ui/files/on-screen'
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

/** Each conversation's file watch, made once for the page's lifetime. */
const watches = new Map<string, ConversationWatch>()

/**
 * The file watch of a conversation's Host, on the relay or its direct
 * channel (`web-api.md` § File watch): what the conversation's views keep
 * of its Host's files is subscribed on it.
 */
export function conversationWatch(conversationId: string): ConversationWatch {
  let watcher = watches.get(conversationId)
  if (!watcher) {
    watcher = new ConversationWatch(conversationId, directWatchLink(() => directRoute(conversationId)))
    watches.set(conversationId, watcher)
  }
  return watcher
}

/**
 * A conversation's files (`plugin-pages.md` § The conversation files
 * service): the Host's tree and contents while its device and directory are
 * known, the working tree's changes, and a request's changes from the
 * transcripts the page holds. What it reads it keeps with the
 * Host's other files, each entry subscribed on the conversation's file
 * watch while it is kept, and a change of one on screen reads it again
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
  const watcher = conversationWatch(conversationId)
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
  // The workspace and the working tree's place, from plain values and the
  // source the device's files keep: a product state that leaves the Host
  // and the directory as they are, as each frame of a turn does, keeps them
  // as they are, so the views that show them have nothing to render, and the
  // change set is not shown again, which would read a list that is not current anew.
  const deviceId = computed(() => device.value?.id ?? null)
  const root = computed(() => execution.value?.path ?? null)
  const source = computed(() => {
    const host = device.value
    return host
      ? fileSource(conversationFileRoutes(conversationId), host, {
          watch: (files) => watcher.channel(host.id, files),
          through: (reads) => directFileReads(() => directRoute(conversationId), reads),
        })
      : null
  })
  // The Cloud's own session directory has no name worth showing; it is the workspace.
  const name = computed(() => execution.value?.directory === null ? 'Workspace' : undefined)
  const workspace = computed(() => source.value && root.value
    ? { source: source.value, root: root.value, name: name.value }
    : null)
  const changes = computed(() => {
    if (!root.value || deviceId.value === null) {
      return null
    }
    const files = hostFiles(deviceId.value)
    return keptChangeSet(workingTreeReads(conversationId), root.value, {
      files,
      watch: watcher.channel(deviceId.value, files),
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
      const onScreen = useOnScreen()
      scope.run(() => {
        watch(changes, (set, _previous, onCleanup) => {
          const showing = set?.show()
          // The list follows the Host only while the component is on screen.
          const following = watch(onScreen, (shown) => (shown ? showing?.back() : showing?.away()), { immediate: true })
          onCleanup(() => {
            following()
            showing?.release()
          })
        }, { immediate: true })
      })
      onScopeDispose(() => scope.stop())
    },
  }
  services.set(conversationId, service)
  return service
}
