import { reactive } from 'vue'
import type { SettingsUser } from '@demicodes/web-ui/settings/types'
import { DAY, HOUR, MINUTE, ago } from './time'

/** An account's Cloud disks until an administrator changes them. */
const STANDARD = { systemGb: 16, homeGb: 16 }

/**
 * The server's accounts as Settings › Users lists them: the master, two
 * admins and four users, one of them disabled since a colleague left and
 * one, Lena, who has not chosen her own password yet. Bo, who builds large
 * projects, has a 64 GB home; everyone else the standard 16 GB each.
 */
export function galleryUsers(): SettingsUser[] {
  return [
    { id: ('alex'), nickname: 'Alex', email: 'alex@example.com', role: 'master', disabled: false, choosesPassword: false, lastActiveAt: ago(MINUTE), cloudStorage: STANDARD },
    { id: 'ana', nickname: 'Ana', email: 'ana@example.com', role: 'admin', disabled: false, choosesPassword: false, lastActiveAt: ago(3 * HOUR), cloudStorage: STANDARD },
    { id: 'mei', nickname: 'Mei', email: 'mei@example.com', role: 'admin', disabled: false, choosesPassword: false, lastActiveAt: ago(2 * DAY), cloudStorage: STANDARD },
    { id: 'bo', nickname: 'Bo', email: 'bo@example.com', role: 'user', disabled: false, choosesPassword: false, lastActiveAt: ago(25 * MINUTE), cloudStorage: { systemGb: 16, homeGb: 64 } },
    { id: 'sam', nickname: 'Sam', email: 'sam@example.com', role: 'user', disabled: false, choosesPassword: false, lastActiveAt: ago(9 * DAY), cloudStorage: STANDARD },
    { id: 'lena', nickname: 'Lena', email: 'lena@example.com', role: 'user', disabled: false, choosesPassword: true, lastActiveAt: null, cloudStorage: STANDARD },
    { id: 'joe', nickname: 'Joe', email: 'joe@example.com', role: 'user', disabled: true, choosesPassword: false, lastActiveAt: ago(40 * DAY), cloudStorage: STANDARD },
  ]
}

/** A specimen's accounts and whose page it is: the master's, or an admin's. */
export function createUsersState(viewerId: string) {
  return reactive({ users: galleryUsers(), viewerId })
}

export type UsersState = ReturnType<typeof createUsersState>


const FIRST_PASSWORD_CHARACTERS = 'abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789'

/** A first password as the server makes one: 20 random letters and digits. */
export function fakeFirstPassword(): string {
  const bytes = crypto.getRandomValues(new Uint8Array(20))
  return Array.from(bytes, (byte) => FIRST_PASSWORD_CHARACTERS[byte % FIRST_PASSWORD_CHARACTERS.length]).join('')
}
