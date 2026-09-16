import { describe, expect, test } from 'bun:test'
import { exposeUrl } from '../expose/records'

/** `expose.md` § The expose record: the URL's scheme and port come from the
 * backend's configured public URL — the printed URL must open. */
describe('exposeUrl', () => {
  test('a local backend on a port carries it', () => {
    expect(exposeUrl('a'.repeat(26), 'expose.localhost', 'http://localhost:3271'))
      .toBe(`http://${'a'.repeat(26)}.expose.localhost:3271/`)
  })

  test('a public URL on the default port carries none', () => {
    expect(exposeUrl('a'.repeat(26), 'expose.demi.example', 'https://demi.example'))
      .toBe(`https://${'a'.repeat(26)}.expose.demi.example/`)
    expect(exposeUrl('a'.repeat(26), 'expose.demi.example', 'https://demi.example:443'))
      .toBe(`https://${'a'.repeat(26)}.expose.demi.example/`)
  })
})
