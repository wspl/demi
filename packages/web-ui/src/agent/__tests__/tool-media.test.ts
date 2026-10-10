import { expect, test } from 'bun:test'
import type { ToolMediaSource, ToolResultContentBlock } from '@demicodes/protocol'
import { foldedMedia, toolMedia } from '../tool-media'
import type { WorkStep } from '../work-groups'
import { createdAt, model } from './agent-harness'

// What a call shows under its row (`file-previews.md` § Media a tool
// returned), read off its result as the transcript stores it: a shell
// result's status and note stay the call's own, the media show in the order
// of the result, and a medium that is gone shows a line that says why where
// it was.

const shot: ToolMediaSource = { type: 'ref', ref: 'a'.repeat(64), mediaType: 'image/png' }
const clip: ToolMediaSource = { type: 'ref', ref: 'b'.repeat(64), mediaType: 'video/mp4' }
const status = { type: 'text', text: 'status: exited\nexitCode: 0\ncommandId: 7\noutput:\n[image 1: image/png, 1280 × 720 px, 15822 bytes]' } as const
const note = { type: 'text', text: '[image 1: attached as image/jpeg of 2000 × 1125 px, fitted from image/png of 2560 × 1440 px]' } as const
const report: ToolResultContentBlock = {
  type: 'document',
  source: { type: 'ref', ref: 'c'.repeat(64), mediaType: 'application/pdf', fileName: 'document-2.pdf' },
}
const notStored: ToolResultContentBlock = {
  type: 'gone',
  kind: 'video',
  mediaType: 'video/mp4',
  cause: { type: 'not_stored', error: 'the object store refused the write' },
}

const cases: { name: string; output: ToolResultContentBlock[]; shows: ReturnType<typeof toolMedia> }[] = [
  {
    name: 'a screenshot',
    output: [status, { type: 'image', source: shot }, note],
    shows: [{ kind: 'image', source: shot, name: 'Look' }],
  },
  {
    name: 'a recording',
    output: [status, { type: 'video', source: clip }, note],
    shows: [{ kind: 'video', source: clip, name: 'Look' }],
  },
  {
    name: 'a recording that was not stored, with the reason',
    output: [status, notStored, note],
    shows: [{ kind: 'gone', text: 'Video not stored: the object store refused the write' }],
  },
  {
    name: 'several, in the order of the result',
    output: [{ type: 'video', source: clip }, notStored, { type: 'image', source: shot }],
    shows: [
      { kind: 'video', source: clip, name: 'Look' },
      { kind: 'gone', text: 'Video not stored: the object store refused the write' },
      { kind: 'image', source: shot, name: 'Look' },
    ],
  },
  {
    name: 'no document, which its line in the output stands for',
    output: [status, { type: 'image', source: shot }, report],
    shows: [{ kind: 'image', source: shot, name: 'Look' }],
  },
  {
    name: 'a result without media',
    output: [status, { type: 'text', text: 'Tool failed: the Host is offline' }],
    shows: [],
  },
]

for (const { name, output, shows } of cases) {
  test(`under its row, a call shows ${name}`, () => {
    expect(toolMedia(output, 'Look')).toEqual(shows)
  })
}

// A call inside a folded group still shows its media, under the group's row
// (`runtime.md` § Work groups): the loop that viewed two screenshots shows
// both, named by its call, after the media of the call before it.
const second: ToolMediaSource = { type: 'ref', ref: 'd'.repeat(64), mediaType: 'image/png' }
const call = (id: string, description: string, output: ToolResultContentBlock[]): WorkStep => ({
  type: 'tool_call',
  id,
  createdAt,
  model,
  toolUseId: `${id}-use`,
  toolName: 'shell',
  status: 'completed',
  input: JSON.stringify({ script: 'demi browser screenshot t1 | demi file view', description }),
  output,
  view: null,
})

test('under a folded group, its calls show their media in order', () => {
  const steps: WorkStep[] = [
    { type: 'thinking', id: 't', createdAt, model, text: 'Look at the page twice.', signature: null },
    call('a', 'Open the page', [status]),
    call('b', 'Take two screenshots', [status, { type: 'image', source: shot }, { type: 'image', source: second }, note]),
    call('c', 'Record the sign-in', [notStored]),
  ]
  expect(foldedMedia(steps)).toEqual([
    { kind: 'image', source: shot, name: 'Take two screenshots' },
    { kind: 'image', source: second, name: 'Take two screenshots' },
    { kind: 'gone', text: 'Video not stored: the object store refused the write' },
  ])
})
