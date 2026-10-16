import { expect, test } from 'bun:test'
import { settingsSections, withUsersSection } from '../sections'
import type { SettingsUser, UserRole } from '../types'
import { assignableRoles, parseCloudGb, userActions } from '../users'

// Cost: pure functions; under a millisecond.
function account(id: string, role: UserRole, disabled = false): SettingsUser {
  return {
    id,
    nickname: id,
    email: `${id}@example.com`,
    role,
    disabled,
    choosesPassword: false,
    lastActiveAt: null,
    cloudStorage: { systemGb: 16, homeGb: 16 },
  }
}

const master = account('alex', 'master')
const admin = account('ana', 'admin')
const peer = account('mei', 'admin')
const user = account('bo', 'user')

test('the master resets, changes the role of, sets the Cloud storage of, signs out and disables a lower account', () => {
  expect(userActions(master, admin)).toEqual([
    'resetPassword',
    'changeRole',
    'cloudStorage',
    'signInCode',
    'signOutEverywhere',
    'disable',
  ])
})

test('an admin changes no role: a user offers reset, Cloud storage, sign-in code, sign out and disable', () => {
  expect(userActions(admin, user)).toEqual(['resetPassword', 'cloudStorage', 'signInCode', 'signOutEverywhere', 'disable'])
})

test('an administrator’s own row offers Cloud storage alone; a peer, a higher role and a user’s own row nothing', () => {
  expect(userActions(master, master)).toEqual(['cloudStorage'])
  expect(userActions(admin, admin)).toEqual(['cloudStorage'])
  expect(userActions(admin, peer)).toEqual([])
  expect(userActions(admin, master)).toEqual([])
  expect(userActions(user, user)).toEqual([])
})

test('a disabled account offers Enable Account in place of sign-in code, sign out and disable', () => {
  expect(userActions(admin, account('joe', 'user', true))).toEqual(['resetPassword', 'cloudStorage', 'enable'])
})

test('a Cloud disk is a whole number of GB from 1 to 1024', () => {
  expect(parseCloudGb(' 64 ')).toBe(64)
  expect(parseCloudGb('1024')).toBe(1024)
  for (const text of ['0', '1025', '1.5', '-1', '', '16 GB']) {
    expect(parseCloudGb(text)).toBeNull()
  }
})

test('each role may give only the roles below its own', () => {
  expect(assignableRoles('master')).toEqual(['admin', 'user'])
  expect(assignableRoles('admin')).toEqual(['user'])
  expect(assignableRoles('user')).toEqual([])
})

test('the rail lists Users after Devices for the master and admins, not for a user', () => {
  const ids = (role: UserRole) =>
    withUsersSection(settingsSections(), role).flatMap((group) => group.items.map((item) => item.id))
  expect(ids('master')).toContain('users')
  expect(ids('admin')).toContain('users')
  expect(ids('user')).not.toContain('users')
  const rail = ids('master')
  expect(rail[rail.indexOf('devices') + 1]).toBe('users')
})
