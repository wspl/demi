import { describe, expect, it } from 'bun:test'
import { fromEditorUri, toEditorUri } from '../editorUri'

describe('editor resource URIs', () => {
  it('encodes path segments while preserving path separators', () => {
    expect(toEditorUri('local', '/tmp/hello world/#draft?.ts')).toBe(
      'editor://local/tmp/hello%20world/%23draft%3F.ts',
    )
  })

  it('round-trips host ids and file paths', () => {
    const uri = toEditorUri('ssh-prod', '/srv/current/src/App.vue')
    expect(fromEditorUri(uri)).toEqual({
      hostId: 'ssh-prod',
      filePath: '/srv/current/src/App.vue',
    })
  })

  it('rejects non-editor URIs', () => {
    expect(fromEditorUri('file:///tmp/demo.ts')).toBe(null)
  })
})
