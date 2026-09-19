import { expect, test } from 'bun:test'
import { buildCommandContext, DEFAULT_LOCALE } from '../runner/command-context'
import { LocalControlService } from '../storage/control'
import { openSqliteDatabase } from '../storage/database'
import { CONTROL_MIGRATIONS, migrate } from '../storage/migrations'

test('a command context names its conversation and caller and carries the conversation\'s user\'s locale', async () => {
  const db = openSqliteDatabase(':memory:')
  try {
    migrate(db, CONTROL_MIGRATIONS)
    const control = new LocalControlService(db)
    const reporter = (await control.createMaster({ email: 'a@example.test', passwordHash: '' }))!
    const silent = (await control.createUser({ email: 'b@example.test', passwordHash: '', role: 'user' }))!
    const locale = { timeZone: 'Asia/Shanghai', languages: ['zh-CN', 'en'] }
    await control.patchUserPreferences(reporter.id, { locale })
    const mine = await control.createConversation(reporter.id)
    const theirs = await control.createConversation(silent.id)
    const agent = { kind: 'agent', node: 'node' } as const
    expect(await buildCommandContext(control, mine.id, agent))
      .toEqual({ conversation: mine.id, caller: agent, locale })
    // Until the user's browser reports one, the default.
    expect(await buildCommandContext(control, theirs.id, { kind: 'user' }))
      .toEqual({ conversation: theirs.id, caller: { kind: 'user' }, locale: DEFAULT_LOCALE })
    await expect(buildCommandContext(control, 'missing', agent)).rejects.toThrow('No conversation missing')
  } finally {
    db.close()
  }
})
