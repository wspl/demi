/** Shared ANSI fixtures so gallery and product show the same colored jobs. */

const reset = '\x1b[0m'
const bold = '\x1b[1m'
const dim = '\x1b[2m'
const red = '\x1b[31m'
const green = '\x1b[32m'
const yellow = '\x1b[33m'
const blue = '\x1b[34m'
const magenta = '\x1b[35m'
const cyan = '\x1b[36m'
const brightRed = '\x1b[91m'
const brightGreen = '\x1b[92m'
const brightYellow = '\x1b[93m'
export const DEMO_BUN_TEST = [
  `${dim}$${reset} bun test src/auth.test.ts`,
  '',
  `${bold}bun test${reset} ${dim}v1.3.14${reset}`,
  '',
  `${cyan}src/auth.test.ts:${reset}`,
  `${brightGreen}(pass)${reset} writes the session cookie ${dim}[4.12ms]${reset}`,
  `${brightGreen}(pass)${reset} sets the session header ${dim}[1.08ms]${reset}`,
  `${brightYellow}(skip)${reset} expired cookie ${dim}[pending]${reset}`,
  `${brightRed}(fail)${reset} reads the old cookie name ${dim}[2.01ms]${reset}`,
  `  ${red}error${reset}: expect(received).toBe(expected)`,
  `  Expected: ${green}"session"${reset}`,
  `  Received: ${red}"sid"${reset}`,
  '',
  ` ${red}1 fail${reset}  ${green}2 pass${reset}  ${yellow}1 skip${reset}  ${dim}3 expect() calls${reset}`,
].join('\n')

export const DEMO_RG = [
  `${dim}$${reset} rg sid packages/web/src`,
  '',
  `${magenta}packages/web/src/auth.test.ts${reset}`,
  `${green}18${reset}:    expect(cookie.name).toBe("${bold}${red}sid${reset}")`,
  `${green}42${reset}:    // legacy ${bold}${red}sid${reset} header`,
  `${magenta}packages/web/src/cookie.ts${reset}`,
  `${green}11${reset}:    // writes session, not ${bold}${red}sid${reset}`,
].join('\n')

export const DEMO_GIT_DIFF = [
  `${dim}$${reset} git diff --stat && git diff packages/web/src/auth.test.ts`,
  '',
  ` packages/web/src/auth.test.ts | 12 ${green}++++++${reset}${red}------${reset}`,
  ` packages/web/src/cookie.ts    |  4 ${green}++++${reset}`,
  ` ${bold}2 files changed, 10 insertions(+), 6 deletions(-)${reset}`,
  '',
  `${bold}diff --git a/packages/web/src/auth.test.ts b/packages/web/src/auth.test.ts${reset}`,
  `${red}--- a/packages/web/src/auth.test.ts${reset}`,
  `${green}+++ b/packages/web/src/auth.test.ts${reset}`,
  `${cyan}@@ -15,7 +15,7 @@${reset} ${dim}describe("cookie", () => {${reset}`,
  `     const cookie = readSessionCookie()`,
  `${red}-    expect(cookie.name).toBe("sid")${reset}`,
  `${green}+    expect(cookie.name).toBe("session")${reset}`,
  `     expect(cookie.httpOnly).toBe(true)`,
  `${blue}warning:${reset} keep the helper on ${magenta}session${reset}; do not write ${yellow}sid${reset}`,
].join('\n')
