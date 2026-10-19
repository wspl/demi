import { expect, test } from 'bun:test'
import type { Block, ToolResultContentBlock } from '@demicodes/protocol'
import { transcriptPictures, type TranscriptPicturesContext } from '../transcript-pictures'
import { mediumKey } from '../../files/media-viewer'
import type { FileContents } from '../../files/types'
import type { MessageFiles } from '../../markdown/types'
import { createdAt, model } from './agent-harness'

// The pictures of a transcript in the order they stand, which the media
// viewer steps through (`file-previews.md` § The media viewer): the images the
// messages show, a Host image by its file, the user's attached files and the
// media tools returned.

/** The Host's kept files, which a message's Host images show; never read here. */
const contents: FileContents = {
  url: (path) => `/raw?path=${path}`,
  describe: () => new Promise(() => {}),
  showDescription: () => { throw new Error('Listing pictures shows no file') },
}

const files: MessageFiles = {
  cwd: '/work',
  contents,
  attachment: (id) => id === 'a1'
    ? { state: 'found', attachment: { name: 'login.png', mediaType: 'image/png', sha256: '1'.repeat(64), url: '/blobs/1', size: 15822, width: 480, height: 300 } }
    : { state: 'missing' },
}
const context: TranscriptPicturesContext = {
  node: null,
  files: () => files,
  address: (source) => source.type === 'url' ? source.url : `/blob/${source.ref}`,
}

const image = (ref: string): ToolResultContentBlock => ({ type: 'image', source: { type: 'ref', ref, mediaType: 'image/png', width: 100, height: 50 } })
const base = { createdAt, model }

const blocks: Block[] = [
  {
    ...base, type: 'user', id: 'u1', turnId: 't1', preamble: null,
    content: [
      { type: 'text', text: 'See ' },
      { type: 'image', source: { type: 'ref', ref: 'mine', mediaType: 'image/png', width: 480, height: 300 } },
      { type: 'attachment', name: 'mine.png', id: 'a1', mediaType: 'image/png', sizeBytes: 1, sha256: 'mine' },
      { type: 'text', text: ' and ![alt](out/typed.png).' },
    ],
  },
  {
    ...base, type: 'tool_call', id: 'c1', toolUseId: 'c1', toolName: 'shell', status: 'completed', run: 'r1',
    input: JSON.stringify({ script: 'demi file view a.png', description: 'Show the page' }),
    output: [{ type: 'text', text: 'ok' }, image('shot-1'), image('shot-2')],
  } as Block,
  { ...base, type: 'text', id: 'x1', text: 'Done:\n\n![Chart](out/chart.png)\n<attachment id=\"a1\" />\n\n[a link](out/a.png)' },
]

test('the pictures of a transcript come in the order they stand: the user\'s, a call\'s, then the reply\'s', () => {
  expect(transcriptPictures(blocks, context).map((medium) => [mediumKey(medium), medium.name])).toEqual([
    ['/blob/mine', 'mine.png'],
    ['file:/work/out/typed.png', 'alt'],
    ['/blob/shot-1', 'Show the page'],
    ['/blob/shot-2', 'Show the page'],
    ['file:/work/out/chart.png', 'Chart'],
    ['/blobs/1', 'login.png'],
  ])
})

test('a subagent\'s brief names its pictures as a message does', () => {
  const brief: Block[] = [{ ...base, type: 'user', id: 'u2', turnId: 't2', preamble: null, content: [{ type: 'text', text: 'Look at ![x](out/a.png)' }] }]
  expect(transcriptPictures(brief, { ...context, node: 'child-1' }).map(mediumKey)).toEqual(['file:/work/out/a.png'])
})
