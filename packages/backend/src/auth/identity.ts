/**
 * The auth module's current extent: a single stub identity. Every request
 * acts as this master user until the real login surface lands (M7); the
 * data model is multi-user-shaped from the start, so only this module
 * changes when it does.
 */
export const STUB_USER = { id: 'local', username: 'local', role: 'master' } as const

export type StubUser = typeof STUB_USER
