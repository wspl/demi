import { expect, test } from 'bun:test'
import { LocalControlService } from '../storage/control'
import { openSqliteDatabase } from '../storage/database'
import { CONTROL_MIGRATIONS, migrate } from '../storage/migrations'
import { resolveRemoteFileRefs } from '../conversation/remote-file-refs'
import { shellQuote } from '@demicodes/shell'

test(
  'file references preserve exact device and path; inaccessible references create no grants',
  async () => {
    const db = openSqliteDatabase(':memory:')
    migrate(db, CONTROL_MIGRATIONS)
    const control = new LocalControlService(db)
    try {
      const owner = (await control.createMaster({
        email: 'owner@example.test',
        passwordHash: 'unused'
      }))!
      const other = (await control.createUser({
        email: 'other@example.test',
        passwordHash: 'unused',
        role: 'user'
      }))!
      const device = await control.createDevice({
        userId: owner.id,
        name: 'build',
        platform: 'linux',
        tokenHash: 'token'
      })
      const foreign = await control.createDevice({
        userId: other.id,
        name: 'build',
        platform: 'linux',
        tokenHash: 'other-token'
      })
      const conversation = await control.createConversation(owner.id)
      const registry = { deviceOnline: () => true, deviceIdentity: () => null }
      const ref = {
        type: 'remote_file',
        deviceId: device.id,
        path: "/srv/it's $(literal).txt"
      }
      await expect(
        resolveRemoteFileRefs(
          control,
          registry,
          conversation,
          [ref, { ...ref, deviceId: foreign.id }]
        )
      ).rejects.toThrow('not accessible')
      expect(await control.listAttachedHosts(conversation.id)).toEqual([])
      const content = await resolveRemoteFileRefs(
        control,
        registry,
        conversation,
        [ref]
      ) as Array<{
        type: string;
        text: string
      }>
      expect(content[0]?.text).toContain(device.id)
      expect(content[0]?.text)
        .toContain(shellQuote(`cat -- ${shellQuote(ref.path)}`))
      expect(
        (await control.listAttachedHosts(conversation.id)).map(
          host => host.deviceId
        )
      )
        .toEqual([device.id])
      await expect(
        resolveRemoteFileRefs(
          control,
          { ...registry, deviceOnline: () => false },
          conversation,
          [ref]
        )
      ).rejects.toThrow('offline')
    } finally {
      db.close()
    }
  }
)
