// A development sign-in for the web front end against a real backend: opens
// the backend's control database directly and issues a session cookie for the
// account named by `DEMI_DEV_EMAIL`, so nobody has to type a password. With
// `--reset-password` it first sets that account's password to
// `DEMI_DEV_PASSWORD`, for signing in by hand. Both variables live in the
// repository's `.env`; the data directory is `DEMI_BACKEND_DATA`, the same
// default as the backend's. Run it with
// `bun run --conditions development packages/backend/src/dev-session.ts`
// while the backend is up: the new session is visible to it at once.
import { homedir } from 'node:os'
import { join } from 'node:path'
import process from 'node:process'
import { hashPassword } from './auth/passwords'
import { WebSessions } from './auth/sessions'
import { SESSION_COOKIE } from './http/cookies'
import { LocalControlService } from './storage/control'
import { openSqliteDatabase } from './storage/database'

async function main(): Promise<void> {
  const dataDir = process.env.DEMI_BACKEND_DATA ??
    join(homedir(), '.demi', 'backend')
  const email = process.env.DEMI_DEV_EMAIL
  if (!email)
    throw new Error('DEMI_DEV_EMAIL names the account to sign in as (set it in .env)')
  const resetPassword = process.argv.includes('--reset-password')
  const password = process.env.DEMI_DEV_PASSWORD
  if (resetPassword && !password)
    throw new Error('--reset-password needs DEMI_DEV_PASSWORD (set it in .env)')

  const db = openSqliteDatabase(join(dataDir, 'control.sqlite'))
  try {
    const control = new LocalControlService(db)
    const user = await control.findUserByEmail(email.trim().toLowerCase())
    if (!user)
      throw new Error(`no account ${email} in ${dataDir}`)
    if (resetPassword && password) {
      await control.setUserPassword(user.id, await hashPassword(password))
      console.log(`password of ${user.email} reset to DEMI_DEV_PASSWORD`)
    }
    const session = await new WebSessions(control).open(user.id)
    console.log(`signed in as ${user.email} until ${session.expiresAt.toISOString()}`)
    console.log(`${SESSION_COOKIE}=${session.token}`)
  } finally {
    db.close()
  }
}

void main()
