import { expect, test } from 'bun:test'
import { SETTINGS_SECTIONS, withUsersSection } from '../sections'
import type { SettingsUser, UserRole } from '../types'
import { assignableRoles, userActions } from '../users'

// Cost: pure functions; under a millisecond.
function account(id: string, role: UserRole, disabled = false): SettingsUser {
  return { id, nickname: id, email: `${id}@example.com`, role, disabled, lastActiveAt: null }
}

const master = account('zan', 'master')
const admin = account('ana', 'admin')
const peer = account('mei', 'admin')
const user = account('bo', 'user')

test('the master resets, changes the role of, signs out and disables a lower account', () => {
  expect(userActions(master, admin)).toEqual(['resetPassword', 'changeRole', 'signOutEverywhere', 'disable'])
})

test('an admin changes no role: a user offers reset, sign out and disable', () => {
  expect(userActions(admin, user)).toEqual(['resetPassword', 'signOutEverywhere', 'disable'])
})

test('no menu on the user’s own row, on a peer or on a higher role', () => {
  expect(userActions(master, master)).toEqual([])
  expect(userActions(admin, admin)).toEqual([])
  expect(userActions(admin, peer)).toEqual([])
  expect(userActions(admin, master)).toEqual([])
  expect(userActions(user, user)).toEqual([])
})

test('a disabled account offers Enable Account in place of sign out and disable', () => {
  expect(userActions(admin, account('joe', 'user', true))).toEqual(['resetPassword', 'enable'])
})

test('each role may give only the roles below its own', () => {
  expect(assignableRoles('master')).toEqual(['admin', 'user'])
  expect(assignableRoles('admin')).toEqual(['user'])
  expect(assignableRoles('user')).toEqual([])
})

test('the rail lists Users after Devices for the master and admins, not for a user', () => {
  const ids = (role: UserRole) =>
    withUsersSection(SETTINGS_SECTIONS, role).flatMap((group) => group.items.map((item) => item.id))
  expect(ids('master')).toContain('users')
  expect(ids('admin')).toContain('users')
  expect(ids('user')).not.toContain('users')
  const rail = ids('master')
  expect(rail[rail.indexOf('devices') + 1]).toBe('users')
})
