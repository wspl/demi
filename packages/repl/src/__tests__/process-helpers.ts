import { spawn } from 'node:child_process'

export class ProcessCapture {
  readonly closed: Promise<number | null>
  private stdoutText = ''
  private stderrText = ''
  private readonly waiters: Waiter[] = []

  constructor(
    private readonly child: ReturnType<typeof spawn>,
  ) {
    child.stdout?.on('data', (chunk) => {
      this.stdoutText += Buffer.from(chunk).toString('utf8')
      this.resolveMatchingWaiters()
    })
    child.stderr?.on('data', (chunk) => {
      this.stderrText += Buffer.from(chunk).toString('utf8')
    })
    this.closed = new Promise<number | null>((resolve) => {
      child.once('close', (code) => {
        this.rejectPendingWaiters()
        resolve(code)
      })
      child.once('error', () => {
        this.rejectPendingWaiters()
        resolve(null)
      })
    })
  }

  stdout(): string {
    return this.stdoutText
  }

  stderr(): string {
    return this.stderrText
  }

  waitForStdout(expected: string, timeoutMs: number): Promise<void> {
    if (this.stdoutText.includes(expected)) return Promise.resolve()
    return new Promise((resolve, reject) => {
      const waiter: Waiter = {
        expected,
        resolve,
        reject,
        timer: setTimeout(() => {
          this.removeWaiter(waiter)
          reject(new Error(`Timed out waiting for stdout ${JSON.stringify(expected)}\nstdout:\n${this.stdoutText}\nstderr:\n${this.stderrText}`))
        }, timeoutMs),
      }
      this.waiters.push(waiter)
    })
  }

  private resolveMatchingWaiters(): void {
    for (const waiter of [...this.waiters]) {
      if (!this.stdoutText.includes(waiter.expected)) continue
      this.removeWaiter(waiter)
      waiter.resolve()
    }
  }

  private rejectPendingWaiters(): void {
    for (const waiter of [...this.waiters]) {
      if (this.stdoutText.includes(waiter.expected)) {
        this.removeWaiter(waiter)
        waiter.resolve()
        continue
      }
      this.removeWaiter(waiter)
      waiter.reject(new Error(`Process closed before stdout ${JSON.stringify(waiter.expected)}\nstdout:\n${this.stdoutText}\nstderr:\n${this.stderrText}`))
    }
  }

  private removeWaiter(waiter: Waiter): void {
    clearTimeout(waiter.timer)
    const index = this.waiters.indexOf(waiter)
    if (index !== -1) this.waiters.splice(index, 1)
  }
}

interface Waiter {
  expected: string
  resolve(): void
  reject(error: Error): void
  timer: ReturnType<typeof setTimeout>
}
