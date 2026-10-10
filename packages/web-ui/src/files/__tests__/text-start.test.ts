import { expect, test } from 'bun:test'
import { effectScope, ref } from 'vue'
import { until } from '@vueuse/core'
import { TEXT_FILE_BYTES } from '@demicodes/protocol'
import { createMemoryFileSource, dir, textFile } from '../memory-source'
import { decodeTextStart, useTextStart } from '../text-start'

// Cost: one 8 MiB file encoded and decoded, under a second.

test('a text file over the text limit shows its first 8 MiB, without the character the cut splits', async () => {
  // 'é' is two bytes: the last one starts one byte before the limit, so the cut splits it.
  const log = `${'a'.repeat(TEXT_FILE_BYTES - 1)}é and more after the limit`
  const source = createMemoryFileSource({
    platform: 'linux',
    home: '/',
    root: dir({ 'big.log': textFile(log, '2026-10-07T10:00:00Z'), 'small.log': textFile('fits', '2026-10-07T10:00:00Z') }),
  })
  // The text read refuses it, as the Host's text route does.
  await expect(source.read('/big.log')).rejects.toMatchObject({ kind: 'too-large' })
  const path = ref<string | null>('/big.log')
  const scope = effectScope()
  const start = scope.run(() => useTextStart(() => source.contents, () => path.value))!
  await until(start).toMatch((state) => state?.phase === 'ready')
  const shown = start.value
  expect(shown?.phase === 'ready' && { length: shown.text.length, end: shown.text.slice(-1), shown: shown.shown, size: shown.size })
    .toEqual({ length: TEXT_FILE_BYTES - 1, end: 'a', shown: TEXT_FILE_BYTES, size: log.length })
  // No file shown, nothing to say.
  path.value = null
  expect(start.value).toBeNull()
  scope.stop()
})

test('binary bytes show the card, as the text route refuses them, and other bytes show as text', () => {
  expect(decodeTextStart(new Uint8Array([0x68, 0x00, 0x69]))).toBeNull()
  // A Latin-1 byte is no reason to refuse the text: it shows as U+FFFD.
  expect(decodeTextStart(new Uint8Array([0x68, 0xff, 0x69]))).toBe('h\uFFFDi')
  // A NUL past the first 8 KiB does not make the bytes binary.
  const late = new Uint8Array(8 * 1024 + 2).fill(0x61)
  late[8 * 1024 + 1] = 0
  expect(decodeTextStart(late)?.length).toBe(8 * 1024 + 2)
  expect(decodeTextStart(new TextEncoder().encode('plain'))).toBe('plain')
})
