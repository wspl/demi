import { expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import { hasImageIntegrity } from '../image-integrity'

for (const [file, mime] of [
  ['valid.png', 'image/png'], ['valid.jpg', 'image/jpeg'], ['valid.gif', 'image/gif'],
  ['valid.webp', 'image/webp'], ['animated.gif', 'image/gif'], ['animated.webp', 'image/webp'],
]) {
  const bytes = new Uint8Array(readFileSync(new URL(`./fixtures/images/${file}`, import.meta.url)))
  test(`${file}: accepts a complete image and rejects every truncated prefix`, () => {
    expect(hasImageIntegrity(bytes, mime)).toBe(true)
    for (let n = 0; n < bytes.length; n++) {
      expect(hasImageIntegrity(bytes.subarray(0, n), mime)).toBe(false)
    }
  })
  test(`${file}: rejects mismatched image MIME types`, () => {
    for (const other of ['image/png', 'image/jpeg', 'image/gif', 'image/webp'].filter(value => value !== mime)) {
      expect(hasImageIntegrity(bytes, other)).toBe(false)
    }
  })
}

test('PNG CRC catches payload corruption even when IEND survives', () => {
  const bytes = new Uint8Array(readFileSync(new URL('./fixtures/images/valid.png', import.meta.url)))
  bytes[45] ^= 1
  expect(hasImageIntegrity(bytes, 'image/png')).toBe(false)
})

test('PNG header plus fabricated IEND cannot disguise a truncated IDAT chunk', () => {
  const bytes = new Uint8Array(readFileSync(new URL('./fixtures/images/valid.png', import.meta.url)))
  const broken = new Uint8Array([...bytes.subarray(0, 45), ...bytes.subarray(-12)])
  expect(hasImageIntegrity(broken, 'image/png')).toBe(false)
})
