import { expect, test } from 'bun:test'
import { shellEditsViewSchema } from '@demicodes/agent'
import { World } from './world'
import { model } from './driver'

for (const target of ['runner:paired', 'cloud'] as const) {
  test(`retained edits are call history on ${target}`, async () => {
    const world = await World.create({ runners: ['paired'] })
    try {
      const conversation = await world.conversation(target)
      await conversation.turn({ model: [model.shell('create', `set -e
printf 'before\\n' > note.txt
printf 'created\\n' | demi file create native.txt
printf '\\0binary' > asset.bin
printf 'temporary' > removed.txt
rm removed.txt`), model.say('Created the files.')] })
      const first = shellEditsViewSchema.parse(conversation.transcript().find(block => block.type === 'tool_call')?.view)
      expect(first.files.map(file => file.path.split('/').at(-1))).toEqual(['note.txt', 'native.txt', 'asset.bin'])
      expect(first.files[2]?.edits).toEqual([{ kept: false }])
      const read = async (commandId: string, path: string, id = conversation.id) =>
        world.api(`/api/conversations/${id}/commands/${commandId}/changes/file?${new URLSearchParams({ path, edit: '0' })}`)
      await conversation.turn({ model: [model.shell('patch', `demi file patch <<'PATCH'
--- a/note.txt
+++ b/note.txt
@@ -1 +1 @@
-before
+after
--- a/native.txt
+++ b/native.txt
@@ -1 +1 @@
-created
+patched
PATCH`), model.say('Patched both files.')] })
      const calls = conversation.transcript().filter(block => block.type === 'tool_call')
      const second = shellEditsViewSchema.parse(calls.at(-1)?.view)
      expect(second.files.map(file => file.kind)).toEqual(['modified', 'modified'])
      expect(await read(first.commandId, first.files[0]!.path)).toEqual({ original: '', modified: 'before\n' })
      expect(await read(second.commandId, second.files[0]!.path)).toEqual({ original: 'before\n', modified: 'after\n' })
      const answer = conversation.transcript().filter(block => block.type === 'text').at(-1)!
      const fork = await world.api<{ conversation: { id: string } }>(`/api/conversations/${conversation.id}/fork`, { id: crypto.randomUUID(), blockId: answer.id })
      expect(await read(first.commandId, first.files[0]!.path, fork.conversation.id)).toEqual({ original: '', modified: 'before\n' })
      await world.api(`/api/conversations/${conversation.id}`, { archived: true }, 'PATCH')
      if (target === 'runner:paired') {
        await world.killRunner('paired')
      }
      expect(await read(second.commandId, second.files[1]!.path)).toEqual({ original: 'created\n', modified: 'patched\n' })
      const response = await world.backend.session.fetch(`/api/conversations/${conversation.id}/commands/${first.commandId}/changes/file?${new URLSearchParams({ path: first.files[2]!.path, edit: '0' })}`)
      expect(response.status).toBe(404)
    } finally {
      await world.close()
    }
  }, 60_000)
}
