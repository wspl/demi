import { computed, effectScope, onScopeDispose, watch } from 'vue'
import { useOnScreen } from '@demicodes/web-ui/files/on-screen'
import { emptyChangeSet, type ChangeSetSource } from '@demicodes/web-ui/files/changes'
import { findRequest } from '@demicodes/web-ui/files/request-changes'
import { keptChangeSet, keptHistory } from '@demicodes/web-ui/files/kept-source'
import type { CommitHistorySource } from '@demicodes/web-ui/files/history'
import type { ConversationFileService } from '@demicodes/web-ui/plugins/page'
import type { FileBrowserSource } from '@demicodes/web-ui/files/types'
import { t as shared } from '@demicodes/web-ui/locales/text'
import { conversationFileRoutes, fileSource, hostFiles } from '../api/files'
import { useResources } from '../state/resources'
import { executionFor } from '../targets/execution'
import { historyReads, readEditCopies, workingTreeReads } from './changes'
import { ConversationWatch, type FollowHost } from './file-watch'
import { directRoute } from '../direct'
import { directFileReads, directWatchLink } from '../direct/operations'
import { useConversations } from './store'

/** Each conversation's service, made once for the page's lifetime. */
const services = new Map<string, ConversationFileService>()

/** Each conversation's file watch, made once for the page's lifetime. */
const watches = new Map<string, ConversationWatch>()

/**
 * The file watch of a conversation's Host, on the relay or its direct
 * channel: what the conversation's views keep of its Host's files is subscribed
 * on it. Each of its watches opens while its device is online, and at once when
 * the device comes online, as a stopped Cloud that started.
 */
export function conversationWatch(conversationId: string): ConversationWatch {
  let watcher = watches.get(conversationId)
  if (!watcher) {
    watcher = new ConversationWatch(
      conversationId,
      directWatchLink(() => directRoute(conversationId)),
      undefined,
      followDevice,
    )
    watches.set(conversationId, watcher)
  }
  return watcher
}

/** Follows a device's state from the synchronization channel, the Cloud's included, for the page's lifetime. */
export const followDevice: FollowHost = (deviceId, cameOnline) => {
  const resources = useResources()
  const online = () => resources.deviceById(deviceId)?.state === 'online'
  effectScope(true).run(() => {
    watch(online, (now) => {
      if (now)
        cameOnline()
    })
  })
  return online
}

/**
 * The files of a Host the conversation reaches, as its attachment picker
 * lists them, with sizes and times: its primary Host's on the
 * conversation's own watch, an attached host's on the conversation's watch
 * with each subscription naming the host.
 */
export function conversationHostSource(
  conversationId: string,
  deviceId: string,
  primary: boolean,
): FileBrowserSource {
  const resources = useResources()
  const watcher = conversationWatch(conversationId)
  return fileSource({
    directory: `/conversations/${encodeURIComponent(conversationId)}/hosts/${encodeURIComponent(deviceId)}/fs`,
  }, resources.deviceById(deviceId), {
    details: true,
    watch: (files) => primary ? watcher.channel(deviceId, files) : watcher.attached(deviceId, files),
  })
}

/**
 * A conversation's files (`packages/web-ui/src/files/file-cache.ts`):
 *
 * - The Host's tree and contents while its device and directory are known.
 * - The working tree's changes.
 * - A request's changes, from the transcripts the page holds.
 *
 * What it reads it keeps with the Host's other files, each entry subscribed on
 * the conversation's file watch while it is kept, and a change of one on screen
 * reads it again.
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
  // For the page's lifetime: while the Host is online, the watch moves
  // whenever the path to its device changes (`packages/web/src/direct/index.ts`).
  effectScope(true).run(() => {
    watch(() => device.value?.state === 'online' ? device.value.id : null, (online, _previous, onCleanup) => {
      if (online === null)
        return
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
  const name = computed(() => execution.value?.directory === null ? shared('common.workspace') : undefined)
  const workspace = computed(() => source.value && root.value
    ? { source: source.value, root: root.value, name: name.value }
    : null)
  // One change set per working tree the views show: the conversation's, and each directory a child works in.
  const trees = new Map<string, ReturnType<typeof keptChangeSet>>()
  function treeChanges(directory: string | undefined) {
    const host = deviceId.value
    const conversationRoot = root.value
    if (!conversationRoot || host === null) {
      return null
    }
    const tree = directory ?? conversationRoot
    const key = `${host}\0${tree}`
    let set = trees.get(key)
    if (!set) {
      const files = hostFiles(host)
      set = keptChangeSet(workingTreeReads(conversationId, tree === conversationRoot ? undefined : tree), tree, {
        files,
        watch: watcher.channel(host, files),
      })
      trees.set(key, set)
    }
    return set
  }
  // One history per working tree the views show, as for its changes.
  const histories = new Map<string, CommitHistorySource>()
  function treeHistory(directory: string | undefined): CommitHistorySource | null {
    const host = deviceId.value
    const conversationRoot = root.value
    if (!conversationRoot || host === null) {
      return null
    }
    const tree = directory ?? conversationRoot
    const key = `${host}\0${tree}`
    let history = histories.get(key)
    if (!history) {
      const files = hostFiles(host)
      history = keptHistory(historyReads(conversationId, tree === conversationRoot ? undefined : tree), tree, {
        files,
        watch: watcher.channel(host, files),
      })
      histories.set(key, history)
    }
    return history
  }

  const service: ConversationFileService = {
    get workspace() {
      return workspace.value
    },
    get root() {
      return root.value
    },
    changes(directory): ChangeSetSource {
      return treeChanges(directory) ?? emptyChangeSet
    },
    history: treeHistory,
    edit: readEditCopies,
    request(node, request) {
      // The conversation's transcripts as the page holds them: its own agent's and each subagent's.
      const conversation = conversations.items.find((item) => item.id === conversationId)
      return conversation ? findRequest(conversation, node, request) : null
    },
    showChanges(directory) {
      // The list shows for the calling component while it lives, for the Host and working tree it has now.
      const scope = effectScope()
      const onScreen = useOnScreen()
      scope.run(() => {
        watch(() => treeChanges(directory?.()), (set, _previous, onCleanup) => {
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
