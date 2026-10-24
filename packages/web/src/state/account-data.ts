import { clearPluginCaches } from '@demicodes/web-ui/plugins/cache'
import { deleteDraftsBut } from '../conversation/drafts'
import { deleteAccountCache } from './offline'
import { heldAccount, holds, setOwner } from './account-owner'

/**
 * What the browser keeps of the signed-in account, and the one place that
 * names all of it, so that the browser holds one account's data at a time and
 * forgets it with the account:
 *
 * - IndexedDB `demi-offline`: the offline copy, the conversations, their nodes
 *   and blocks (`./offline`).
 * - IndexedDB `demi-drafts`: the drafts, by account (`../conversation/drafts`).
 * - IndexedDB `demi-plugin-cache`: what the plugins kept of the account's Hosts.
 * - `localStorage`: the folded projects (`demi.preferences.<account>`), a
 *   draft's last words (`demi.draft.<account>.<conversation>`), and the
 *   devices' measured paths and last directories (`demi.deviceMeasured.*`,
 *   `demi.devicePath.*`).
 * - `localStorage` `demi.account`: which account all of it is, which every
 *   write asks first (`./account-owner`).
 *
 * What belongs to the browser is not here and stays: the theme, the language,
 * the notification setting and the catalog of providers to add.
 */

/** The `localStorage` keys of the account's that name it, each followed by the account's id. */
const ACCOUNT_KEYS = ['demi.preferences.', 'demi.draft.']

/** Whether `key` is one of `ACCOUNT_KEYS` of the account `userId`. */
function accountKey(key: string, userId: string): boolean {
  return key === `demi.preferences.${userId}` || key.startsWith(`demi.draft.${userId}.`)
}
/**
 * The `localStorage` keys of the account's devices, each followed by a
 * device's id, which name no account and so are never known to be another's.
 */
const DEVICE_KEYS = ['demi.deviceMeasured.', 'demi.devicePath.']

/**
 * The account `userId` signed in: the browser's data becomes its. It owns the
 * data at once, so that no page of another account writes any more, and then
 * what is not its own goes: another account's, or, in a browser that names
 * no account, as before its first sign-in with this build, whatever cannot be
 * known to be its own. The drafts and last words that name it stay, so no
 * draft of the account is lost.
 */
export function takeAccount(userId: string): Promise<void> {
  // A sign-in and the copy's first write ask at once: both wait for the one
  // forgetting, so that no copy is written before it and then cleared.
  if (taking?.userId === userId) {
    return taking.run
  }
  if (holds(userId)) {
    return Promise.resolve()
  }
  setOwner(userId)
  const run = forget(userId).finally(() => {
    if (taking?.run === run) {
      taking = null
    }
  })
  taking = { userId, run }
  return run
}

/** The account the browser's data is becoming, while what is not its own goes. */
let taking: { userId: string; run: Promise<void> } | null = null

/** Resolves once what the sign-in of `userId` forgets is gone, at once when it is not under way. */
export function accountTaken(userId: string): Promise<void> {
  return taking?.userId === userId ? taking.run : Promise.resolve()
}

/**
 * The account `userId` signed out, or its session ended: the browser's data
 * stops being its, so that no page writes it any more, and then goes. Data
 * another account already holds is that account's and stays.
 */
export async function forgetAccount(userId: string): Promise<void> {
  const held = heldAccount()
  if (held !== null && held !== userId) {
    return
  }
  setOwner(null)
  await forget(null)
}

/** Forgets every account's data but the drafts and preferences of `keep`, every account's for none. */
async function forget(keep: string | null): Promise<void> {
  const own = (key: string) => keep !== null && accountKey(key, keep)
  try {
    for (const key of Object.keys(localStorage)) {
      const account = ACCOUNT_KEYS.some((prefix) => key.startsWith(prefix)) && !own(key)
      if (account || DEVICE_KEYS.some((prefix) => key.startsWith(prefix))) {
        localStorage.removeItem(key)
      }
    }
  } catch {
    // Storage the web browser refuses holds nothing.
  }
  const results = await Promise.allSettled([deleteAccountCache(), deleteDraftsBut(keep), clearPluginCaches()])
  const failed = results.find((result): result is PromiseRejectedResult => result.status === 'rejected')
  if (failed) {
    throw failed.reason
  }
}
