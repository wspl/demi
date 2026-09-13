import { describe, expect, it } from 'bun:test'
import { toEditorUri } from '../editorUri'
import { createEditorHostDefaults } from '../host/defaults'
import { inferLanguageIdFromResourceUri } from '../language/languageId'

describe('language id inference', () => {
  it('infers language ids from resource URIs via the shared helper', () => {
    expect(inferLanguageIdFromResourceUri(toEditorUri('local', '/tmp/App.vue'))).toBe('vue')
    expect(inferLanguageIdFromResourceUri(toEditorUri('local', '/tmp/no-extension'))).toBe('plaintext')
    expect(inferLanguageIdFromResourceUri(toEditorUri('local', '/tmp/custom.foo'))).toBe('foo')
  })

  it('reuses the shared helper in the default host', () => {
    expect(createEditorHostDefaults().lsp.getLanguageId).toBe(inferLanguageIdFromResourceUri)
  })
})
