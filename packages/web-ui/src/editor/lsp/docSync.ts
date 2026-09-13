import { ViewPlugin, type ViewUpdate } from '@codemirror/view'
import { Facet } from '@codemirror/state'
import type { EditorHost } from '../host/types'
import { getPluginTargetKey } from './utils'

interface OpenedEntry {
  version: number
  languageId: string
  content: string
  sentPlugins: Set<string>
}

interface HostDocState {
  openedFiles: Map<string, OpenedEntry>
  unsubscribeWorkdirs: (() => void) | null
}

const hostStates = new WeakMap<EditorHost, HostDocState>()

export const lspFileUri = Facet.define<string, string>({
  combine: (values) => values[0] ?? '',
})

function getHostState(host: EditorHost): HostDocState {
  let state = hostStates.get(host)
  if (!state) {
    state = {
      openedFiles: new Map<string, OpenedEntry>(),
      unsubscribeWorkdirs: null,
    }
    hostStates.set(host, state)
  }
  return state
}

function ensureWorkdirSubscription(host: EditorHost) {
  const state = getHostState(host)
  if (state.unsubscribeWorkdirs) return

  state.unsubscribeWorkdirs = host.lsp.subscribeWorkdirs(() => {
    for (const [resourceUri, entry] of state.openedFiles) {
      syncTargets(host, resourceUri, entry)
    }
  })
}

function syncTargets(host: EditorHost, resourceUri: string, entry: OpenedEntry) {
  const targets = host.lsp.getTargetsForResource(resourceUri)
  const activeTargetKeys = new Set(targets.map(getPluginTargetKey))

  entry.sentPlugins = new Set(
    [...entry.sentPlugins].filter((targetKey) => activeTargetKeys.has(targetKey)),
  )

  for (const target of targets) {
    const targetKey = getPluginTargetKey(target)
    if (entry.sentPlugins.has(targetKey)) continue
    entry.sentPlugins.add(targetKey)
    host.lsp.sendNotification(target, 'textDocument/didOpen', {
      textDocument: {
        uri: host.lsp.toDocumentUri(target, resourceUri),
        languageId: entry.languageId,
        version: entry.version,
        text: entry.content,
      },
    })
  }
}

export function syncOpen(host: EditorHost, resourceUri: string, content: string) {
  const state = getHostState(host)
  ensureWorkdirSubscription(host)

  const languageId = host.lsp.getLanguageId(resourceUri)

  let entry = state.openedFiles.get(resourceUri)
  if (!entry) {
    entry = { version: 1, languageId, content, sentPlugins: new Set() }
    state.openedFiles.set(resourceUri, entry)
  } else {
    entry.content = content
    entry.languageId = languageId
  }

  syncTargets(host, resourceUri, entry)
}

export function syncChange(host: EditorHost, resourceUri: string, content: string) {
  const entry = getHostState(host).openedFiles.get(resourceUri)
  if (!entry) return
  entry.version++
  entry.content = content
  // Only query targets once, then filter by sentPlugins
  const targets = host.lsp.getTargetsForResource(resourceUri)
  for (const target of targets) {
    if (!entry.sentPlugins.has(getPluginTargetKey(target))) continue
    host.lsp.sendNotification(target, 'textDocument/didChange', {
      textDocument: { uri: host.lsp.toDocumentUri(target, resourceUri), version: entry.version },
      contentChanges: [{ text: content }],
    })
  }
}

export function lspDocSyncExtension(host: EditorHost) {
  const docSyncPlugin = ViewPlugin.fromClass(class {
    timeout = -1

    update(update: ViewUpdate) {
      if (!update.docChanged) return
      if (this.timeout > -1) clearTimeout(this.timeout)
      this.timeout = window.setTimeout(() => {
        this.timeout = -1
        const resourceUri = update.view.state.facet(lspFileUri)
        if (!resourceUri) return
        syncChange(host, resourceUri, update.view.state.doc.toString())
      }, 200)
    }

    destroy() {
      if (this.timeout > -1) clearTimeout(this.timeout)
    }
  })

  return [docSyncPlugin]
}
