import { reactive } from 'vue'
import type { SettingsUser } from '@demicodes/web-ui/settings/types'
import { DAY, HOUR, MINUTE, ago } from './time'

/**
 * The server's accounts as Settings › Users lists them: the master, two
 * admins and four users, one of them disabled since a colleague left and
 * one, Lena, who has not chosen her own password yet.
 */
export function galleryUsers(): SettingsUser[] {
  return [
    { id: 'zan', nickname: 'Zan', email: 'zan@example.com', role: 'master', disabled: false, choosesPassword: false, lastActiveAt: ago(MINUTE) },
    { id: 'ana', nickname: 'Ana', email: 'ana@example.com', role: 'admin', disabled: false, choosesPassword: false, lastActiveAt: ago(3 * HOUR) },
    { id: 'mei', nickname: 'Mei', email: 'mei@example.com', role: 'admin', disabled: false, choosesPassword: false, lastActiveAt: ago(2 * DAY) },
    { id: 'bo', nickname: 'Bo', email: 'bo@example.com', role: 'user', disabled: false, choosesPassword: false, lastActiveAt: ago(25 * MINUTE) },
    { id: 'sam', nickname: 'Sam', email: 'sam@example.com', role: 'user', disabled: false, choosesPassword: false, lastActiveAt: ago(9 * DAY) },
    { id: 'lena', nickname: 'Lena', email: 'lena@example.com', role: 'user', disabled: false, choosesPassword: true, lastActiveAt: null },
    { id: 'joe', nickname: 'Joe', email: 'joe@example.com', role: 'user', disabled: true, choosesPassword: false, lastActiveAt: ago(40 * DAY) },
  ]
}

/** A specimen's accounts and whose page it is: the master's, or an admin's. */
export function createUsersState(viewerId: string) {
  return reactive({ users: galleryUsers(), viewerId })
}

export type UsersState = ReturnType<typeof createUsersState>

