import { expect, test } from 'bun:test'
import type { ToolMediaSource, ToolResultContentBlock } from '@demicodes/protocol'
import { toolMedia } from '../tool-media'

// What a call shows under its row (`file-previews.md` § Media a tool
// returned), read off its result as the transcript stores it: a shell
// result's status and note stay the call's own, the media show in the order
// of the result, and a text that took a medium's place shows where it was.

const shot: ToolMediaSource = { type: 'ref', ref: 'a'.repeat(64), mediaType: 'image/png' }
const clip: ToolMediaSource = { type: 'ref', ref: 'b'.repeat(64), mediaType: 'video/mp4' }
const status = { type: 'text', text: 'status: exited\nexitCode: 0\npreviewBudgetTokens: 10000\npreview:\n<binary stdout: 15822 bytes>' } as const
const note = { type: 'text', text: 'Attached stdout as image/png (15822 bytes).' } as const
const retired = '[image:image/png, removed on 2026-10-01: a tool result\'s images and videos are kept for 30 days]'
const notStored = '[video not stored: the object store refused the write]'

const cases: { name: string; output: ToolResultContentBlock[]; shows: ReturnType<typeof toolMedia> }[] = [
  {
    name: 'a screenshot',
    output: [status, { type: 'image', source: shot }, note],
    shows: [{ kind: 'image', source: shot }],
  },
  {
    name: 'a recording',
    output: [status, { type: 'video', source: clip }, note],
    shows: [{ kind: 'video', source: clip }],
  },
  {
    name: 'a screenshot retired after 30 days',
    output: [status, { type: 'text', text: retired }, note],
    shows: [{ kind: 'gone', text: retired }],
  },
  {
    name: 'a recording that was not stored',
    output: [status, { type: 'text', text: notStored }, note],
    shows: [{ kind: 'gone', text: notStored }],
  },
  {
    name: 'several, in the order of the result',
    output: [{ type: 'video', source: clip }, { type: 'text', text: retired }, { type: 'image', source: shot }],
    shows: [{ kind: 'video', source: clip }, { kind: 'gone', text: retired }, { kind: 'image', source: shot }],
  },
  {
    name: 'a result without media',
    output: [status, { type: 'text', text: 'Tool failed: the Host is offline' }],
    shows: [],
  },
]

for (const { name, output, shows } of cases) {
  test(`under its row, a call shows ${name}`, () => {
    expect(toolMedia(output)).toEqual(shows)
  })
}
