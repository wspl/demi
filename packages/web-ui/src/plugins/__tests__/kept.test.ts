import { expect, test } from 'bun:test'
import { KEPT_SESSIONS, KeptSessions, type KeptSession } from '../kept'

// Cost: plain objects; milliseconds.

/** A session that records what its sessions are told, by key. */
function recording(told: string[]) {
  return (key: string): KeptSession => ({
    watched: (watched) => void told.push(`${key} ${watched ? 'watched' : 'unwatched'}`),
    dispose: () => void told.push(`${key} released`),
  })
}

/**
 * Planted defects this catches: a limit that counts the watched sessions, a
 * release of the most recently watched, and one past 16.
 */
test('at most sixteen sessions nobody watches are kept, the least recently watched released first, and a watched one never', () => {
  const told: string[] = []
  const sessions = new KeptSessions(recording(told))
  // One stays watched throughout, as a view the user looks at.
  const looked = sessions.watch('shown')
  for (let index = 0; index < KEPT_SESSIONS; index++) {
    sessions.watch(`d${index}`).end()
  }
  // The first is watched again, so the second is now the least recent.
  sessions.watch('d0').end()
  told.length = 0
  sessions.watch('d16').end()
  expect(told).toEqual(['d16 watched', 'd16 unwatched', 'd1 released'])
  expect(sessions.keys()).toHaveLength(KEPT_SESSIONS + 1)
  expect(sessions.get('shown')).toBe(looked.session)
  expect(sessions.get('d0')).toBeDefined()

  told.length = 0
  sessions.dispose()
  expect(told).toHaveLength(KEPT_SESSIONS + 1)
  expect(sessions.keys()).toEqual([])
})

test('a session two views watch stays watched until both let go, and one view letting go twice counts once', () => {
  const told: string[] = []
  const sessions = new KeptSessions(recording(told), 0)
  const panel = sessions.watch('d1')
  const phone = sessions.watch('d1')
  expect(phone.session).toBe(panel.session)
  panel.end()
  panel.end()
  expect(told).toEqual(['d1 watched'])
  phone.end()
  // With nothing kept unwatched, the last view letting go releases it.
  expect(told).toEqual(['d1 watched', 'd1 unwatched', 'd1 released'])
})
