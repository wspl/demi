import { computed, effectScope, onScopeDispose, shallowRef, watch } from 'vue'
import { baseName } from '@demicodes/utils'
import { errorFields } from '@demicodes/web-ui/infra/errors'
import { pageLog } from '@demicodes/web-ui/infra/log'
import { changedPlaces, namedRepositories, type ChangedPlaces, type FoundRepositories } from '@demicodes/web-ui/files/repositories'
import { useOnScreen } from '@demicodes/web-ui/files/on-screen'
import { emptyChangeSet, type ChangeSetSource } from '@demicodes/web-ui/files/changes'
import { findRequest } from '@demicodes/web-ui/files/request-changes'
import { keptChangeSet, keptHistory } from '@demicodes/web-ui/files/kept-source'
import type { CommitHistorySource } from '@demicodes/web-ui/files/history'
import type { ConversationFileService } from '@demicodes/web-ui/plugins/page'
import type { FileBrowserSource } from '@demicodes/web-ui/files/types'
import { t as shared } from '@demicodes/web-ui/locales/text'
import { conversationFileRoutes, fileSource, deviceFiles } from '../api/files'
import { useResources } from '../state/resources'
import { executionFor } from '../targets/execution'
import { historyReads, readEditCopies, readRepositories, workingTreeReads } from './changes'
import { ConversationWatch, type FollowDevice } from './file-watch'
import { directRoute } from '../direct'
import { directFileReads, directWatchLink } from '../direct/operations'
import { agentDirectories } from '@demicodes/web-ui/agent/working-directory'
import { useConversations } from './store'

/** Nothing to choose from, when the device cannot list the repositories: the view reads the directory as before. */
const NO_REPOSITORIES: FoundRepositories = { inRepository: false, repositories: [] }

/** Each conversation's service, made once for the page's lifetime. */
const services = new Map<string, ConversationFileService>()

/** Each conversation's file watch, made once for the page's lifetime. */
const watches = new Map<string, ConversationWatch>()

/**
 * The file watch of a conversation's device, on the relay or its direct
 * channel: what the conversation's views keep of its device's files is subscribed
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
export const followDevice: FollowDevice = (deviceId, cameOnline) => {
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
 * The files of a device the conversation reaches, as its attachment picker
 * lists them, with sizes and times: its primary device's on the
 * conversation's own watch, an attached device's on the conversation's watch
 * with each subscription naming the device.
 */
export function conversationDeviceSource(
  conversationId: string,
  deviceId: string,
  primary: boolean,
): FileBrowserSource {
  const resources = useResources()
  const watcher = conversationWatch(conversationId)
  return fileSource({
    directory: `/conversations/${encodeURIComponent(conversationId)}/devices/${encodeURIComponent(deviceId)}/fs`,
  }, resources.deviceById(deviceId), {
    details: true,
    watch: (files) => primary ? watcher.channel(deviceId, files) : watcher.attached(deviceId, files),
  })
}

/**
 * A conversation's files (`packages/web-ui/src/files/file-cache.ts`):
 *
 * - The Device's tree and contents while its device and directory are known.
 * - The working tree's changes.
 * - A request's changes, from the transcripts the page holds.
 *
 * What it reads it keeps with the device's other files, each entry subscribed on
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
  const conversation = computed(() => conversations.items.find((item) => item.id === conversationId) ?? null)
  const execution = computed(() => (conversation.value ? executionFor(conversation.value) : null))
  const device = computed(() => {
    const target = execution.value
    return target ? resources.deviceById(target.deviceId) : null
  })
  const watcher = conversationWatch(conversationId)
  // For the page's lifetime: while the device is online, the watch moves
  // whenever the path to its device changes (`packages/web/src/direct/index.ts`).
  effectScope(true).run(() => {
    watch(() => device.value?.state === 'online' ? device.value.id : null, (online, _previous, onCleanup) => {
      if (online === null)
        return
      const moving = directRoute(conversationId)?.device.onChange(() => watcher.move())
      onCleanup(() => moving?.())
    }, { immediate: true })
  })
  // The tree and the working tree's place, from plain values and the
  // source the device's files keep: a product state that leaves the device
  // and the directory as they are, as each frame of a turn does, keeps them
  // as they are, so the views that show them have nothing to render, and the
  // change set is not shown again, which would read a list that is not current anew.
  const deviceId = computed(() => device.value?.id ?? null)
  const root = computed(() => execution.value?.path ?? null)
  const source = computed(() => {
    const current = device.value
    return current
      ? fileSource(conversationFileRoutes(conversationId), current, {
          watch: (files) => watcher.channel(current.id, files),
          through: (reads) => directFileReads(() => directRoute(conversationId), reads),
        })
      : null
  })
  // The Cloud's own session directory has no name worth showing, so it shows as the conversation's folder.
  const name = computed(() => execution.value?.directory === null ? shared('common.conversationFolder') : undefined)
  const tree = computed(() => source.value && root.value
    ? { source: source.value, root: root.value, name: name.value }
    : null)
  // The main agent's directory, which carries the Cloud's folder name, and each running subagent's.
  const directories = computed(() => {
    const own = root.value
    if (!own)
      return []
    return agentDirectories(own, conversation.value?.subagents ?? []).map((path) => ({
      path,
      ...(path === own && name.value ? { name: name.value } : {}),
    }))
  })
  // One change set per working tree the views show: the conversation's, and each directory a child works in.
  const trees = new Map<string, ReturnType<typeof keptChangeSet>>()
  function treeChanges(directory: string | undefined) {
    const device = deviceId.value
    const conversationRoot = root.value
    if (!conversationRoot || device === null) {
      return null
    }
    const tree = directory ?? conversationRoot
    const key = `${device}\0${tree}`
    let set = trees.get(key)
    if (!set) {
      const files = deviceFiles(device)
      set = keptChangeSet(workingTreeReads(conversationId, tree === conversationRoot ? undefined : tree), tree, {
        files,
        watch: watcher.channel(device, files),
      })
      trees.set(key, set)
    }
    return set
  }
  // One history per working tree the views show, as for its changes.
  const histories = new Map<string, CommitHistorySource>()
  function treeHistory(directory: string | undefined): CommitHistorySource | null {
    const device = deviceId.value
    const conversationRoot = root.value
    if (!conversationRoot || device === null) {
      return null
    }
    const tree = directory ?? conversationRoot
    const key = `${device}\0${tree}`
    let history = histories.get(key)
    if (!history) {
      const files = deviceFiles(device)
      history = keptHistory(historyReads(conversationId, tree === conversationRoot ? undefined : tree), tree, {
        files,
        watch: watcher.channel(device, files),
      })
      histories.set(key, history)
    }
    return history
  }

  // Where the agents changed files: the same folders keep their list, so a
  // file changed in a folder known already asks the device nothing.
  const places = computed<ChangedPlaces | null>((previous) => {
    const own = root.value
    const next = own && conversation.value ? changedPlaces(conversation.value, own) : null
    if (!previous || !next)
      return next
    const sameDirectories = previous.directories.length === next.directories.length
      && previous.directories.every((directory, index) => directory === next.directories[index])
    if (!sameDirectories)
      return next
    return previous.latest === next.latest ? previous : { directories: previous.directories, latest: next.latest }
  })
  // The views on screen that show the repositories: the device is asked only
  // while one does, and the transcripts are read for it only then.
  const repositoryViewers = shallowRef(0)
  // What the device found, for the device and directory it was asked of, and the
  // folders it was asked about, which only grow: a folder the page no longer
  // holds the transcript of keeps the repository found there.
  const found = shallowRef<{ tree: string; asked: ReadonlySet<string>; answer: FoundRepositories; fromDevice: boolean } | null>(null)
  let followingRepositories = false
  function followRepositories(): void {
    if (followingRepositories)
      return
    followingRepositories = true
    effectScope(true).run(() => {
      const asking = computed((previous?: { tree: string; online: boolean; directories: readonly string[] } | null) => {
        const own = root.value
        const current = device.value
        if (repositoryViewers.value === 0 || !own || !current)
          return null
        const next = { tree: `${current.id}\0${own}`, online: current.state === 'online', directories: places.value?.directories ?? [] }
        const same = previous && previous.tree === next.tree && previous.online === next.online && previous.directories === next.directories
        return same ? previous : next
      })
      watch(asking, (asked, _previous, onCleanup) => {
        if (!asked)
          return
        const known = found.value?.tree === asked.tree ? found.value : null
        if (!known)
          found.value = null
        // An offline device lists nothing new: the view reads the directory as it did before there was a choice.
        if (!asked.online) {
          found.value = known ?? { tree: asked.tree, asked: new Set(), answer: NO_REPOSITORIES, fromDevice: false }
          return
        }
        if (known?.fromDevice && asked.directories.every((directory) => known.asked.has(directory)))
          return
        const folders = new Set([...(known?.asked ?? []), ...asked.directories])
        const stop = new AbortController()
        onCleanup(() => stop.abort())
        readRepositories(conversationId, [...folders], stop.signal).then(
          (answer) => (found.value = { tree: asked.tree, asked: folders, answer, fromDevice: true }),
          (error: unknown) => {
            if (stop.signal.aborted)
              return
            found.value ??= { tree: asked.tree, asked: new Set(), answer: NO_REPOSITORIES, fromDevice: false }
            pageLog.warn('repositories.read_failed', { conversation: conversationId, ...errorFields(error) })
          },
        )
      }, { immediate: true })
    })
  }
  const repositories = computed(() => {
    const own = root.value
    const known = found.value
    if (!own || !known)
      return null
    return namedRepositories(known.answer, own, name.value ?? baseName(own), places.value?.latest ?? null)
  })

  const service: ConversationFileService = {
    get tree() {
      return tree.value
    },
    get root() {
      return root.value
    },
    get directories() {
      return directories.value
    },
    changes(directory): ChangeSetSource {
      return treeChanges(directory) ?? emptyChangeSet
    },
    history: treeHistory,
    repositories() {
      return repositories.value
    },
    showRepositories() {
      followRepositories()
      // Counted while the calling component is on screen, until its scope ends.
      const onScreen = useOnScreen()
      let counted = false
      const count = (shown: boolean) => {
        if (shown !== counted) {
          counted = shown
          repositoryViewers.value += shown ? 1 : -1
        }
      }
      watch(onScreen, count, { immediate: true })
      onScopeDispose(() => count(false))
    },
    edit: readEditCopies,
    request(node, request) {
      // The conversation's transcripts as the page holds them: its own agent's and each subagent's.
      const conversation = conversations.items.find((item) => item.id === conversationId)
      return conversation ? findRequest(conversation, node, request) : null
    },
    showChanges(directory) {
      // The list shows for the calling component while it lives, for the device and working tree it has now.
      const scope = effectScope()
      const onScreen = useOnScreen()
      scope.run(() => {
        watch(() => {
          const shown = directory?.()
          return shown === null ? null : treeChanges(shown)
        }, (set, _previous, onCleanup) => {
          const showing = set?.show()
          // The list follows the device only while the component is on screen.
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
