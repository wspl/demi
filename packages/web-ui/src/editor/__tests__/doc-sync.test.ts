import { describe, expect, it } from 'bun:test'
import { createEditorHostDefaults } from '../host/defaults'
import type { EditorHost, EditorLspTarget } from '../host/types'
import { syncChange, syncOpen } from '../lsp/docSync'

interface SentNotification {
  pluginId: string
  method: string
  params: unknown
}

function createTarget(pluginId: string): EditorLspTarget {
  return {
    hostId: 'test',
    cwd: '/repo',
    pluginId,
    capabilities: {},
  }
}

function createHost(currentTargets: () => EditorLspTarget[], sent: SentNotification[]): {
  host: EditorHost
  emitWorkdirChange(): void
} {
  let workdirListener: (() => void) | null = null

  const host: EditorHost = {
    ...createEditorHostDefaults(),
    lsp: {
      ...createEditorHostDefaults().lsp,
      getTargetsForResource: () => currentTargets(),
      getLanguageId: () => 'typescript',
      toDocumentUri: (target, resourceUri) => `${resourceUri}#${target.pluginId}`,
      sendNotification: (target, method, params) => {
        sent.push({ pluginId: target.pluginId, method, params })
      },
      subscribeWorkdirs(listener) {
        workdirListener = listener
        return () => {
          if (workdirListener === listener) workdirListener = null
        }
      },
    },
  }

  return {
    host,
    emitWorkdirChange() {
      workdirListener?.()
    },
  }
}

describe('lsp doc sync', () => {
  it('reopens a target with current content after it disappears and later returns', () => {
    const primary = createTarget('primary')
    const secondary = createTarget('secondary')
    let targets = [primary, secondary]
    const sent: SentNotification[] = []
    const { host, emitWorkdirChange } = createHost(() => targets, sent)
    const resourceUri = 'editor://test/repo/src/main.ts'

    syncOpen(host, resourceUri, 'one')

    targets = [primary]
    emitWorkdirChange()
    syncChange(host, resourceUri, 'two')

    targets = [primary, secondary]
    emitWorkdirChange()

    const secondaryOpenEvents = sent.filter((entry) =>
      entry.pluginId === 'secondary' && entry.method === 'textDocument/didOpen',
    )

    expect(secondaryOpenEvents).toHaveLength(2)
    expect(secondaryOpenEvents[1]?.params).toEqual({
      textDocument: {
        uri: `${resourceUri}#secondary`,
        languageId: 'typescript',
        version: 2,
        text: 'two',
      },
    })
  })
})
