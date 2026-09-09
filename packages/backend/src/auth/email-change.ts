import { createHmac, randomInt } from 'node:crypto'
import { createId } from '@demicodes/utils'
import type { ControlService } from '../storage/control'
import { verifyPassword } from './passwords'

/** Deployment supplies mail delivery; tests capture messages without sending email. */
export interface AccountMailSender {
  sendVerification(message: { email: string; code: string; expiresAt: number }): Promise<void>
}

export class EmailChanges {
  constructor(
    private readonly control: ControlService,
    private readonly secret: Uint8Array,
    private readonly sender: AccountMailSender | undefined,
    private readonly now: () => number = Date.now,
  ) {}

  async start(userId: string, email: string, password: string) {
    if (!this.sender) return { code: 'mail_unavailable' } as const
    const user = await this.control.getUser(userId)
    const account = user ? await this.control.findUserByEmail(user.email) : null
    if (!account || !(await verifyPassword(password, account.passwordHash))) return { code: 'invalid_credentials' } as const
    if (await this.control.findUserByEmail(email)) return { code: 'email_taken' } as const
    const id = createId()
    const code = randomInt(1_000_000).toString().padStart(6, '0')
    const sentAt = this.now()
    const expiresAt = sentAt + 10 * 60_000
    const issued = await this.control.issueEmailChallenge({
      userId, id, email, passwordHash: account.passwordHash,
      codeHash: this.codeHash(id, code), sentAt, expiresAt,
    })
    if (!issued) return { code: 'too_many_attempts' } as const
    try {
      await this.sender.sendVerification({ email, code, expiresAt })
    } catch {
      // A failed delivery cannot leave a usable code or block the user's retry.
      await this.control.deleteEmailChallenge(userId, id)
      return { code: 'mail_failed' } as const
    }
    return { id, email, expiresAt } as const
  }

  confirm(userId: string, id: string, code: string) {
    return this.control.confirmEmailChallenge(userId, id, this.codeHash(id, code), this.now())
  }

  private codeHash(id: string, code: string): string {
    return createHmac('sha256', this.secret).update(`email-change:${id}:${code}`).digest('hex')
  }
}
