import { expect, test } from 'bun:test'
import { menuEntries } from '../entries'

test('the menu lists the exposes in the state\'s order, each naming its host as the page does', () => {
  const expose = (id: string, deviceId: string) => ({
    id,
    number: 1,
    deviceId,
    address: '127.0.0.1:5173',
    url: `https://${id}.expose.demi.example/`,
    expiresAt: '2026-09-17T00:59:00.000Z',
  })
  const names: Record<string, string> = { managed: 'Cloud', laptop: 'laptop' }
  const rows = menuEntries([expose('a', 'managed'), expose('b', 'laptop')], (id) => names[id] ?? id)
  expect(rows).toEqual([
    { id: 'a', address: '127.0.0.1:5173', hostName: 'Cloud', url: 'https://a.expose.demi.example/', expiresAt: '2026-09-17T00:59:00.000Z' },
    { id: 'b', address: '127.0.0.1:5173', hostName: 'laptop', url: 'https://b.expose.demi.example/', expiresAt: '2026-09-17T00:59:00.000Z' },
  ])
})
