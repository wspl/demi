// `bun browse help` and `demi.help()`: what a script has, and the helpers
// `demi` adds (browse.md).
import type { Tool } from '../tool'

export function help(tool: Tool): void {
  tool.print(HELP)
}

export const HELP = `bun browse: product checks as Playwright scripts in the slot's browser (docs/delivery/browse.md)

  bun browse [--headed] [--limit <seconds|Nm>] <<'JS' … JS   run a script from standard input
  bun browse [--headed] [--limit <seconds|Nm>] <file>          run a script file
  bun browse up [backend] [web] [gallery] | down [servers] [--wipe] | stop | help

A script is the body of an async function. It prints what it console.logs and
what it returns, then the page's state; a call ends after ten minutes unless
--limit says otherwise. It has:
  page      the slot's Playwright Page, open between calls; page.goto('/chat/c-1') opens the web app
  context   its BrowserContext; an action waits 10 s for its element unless it says { timeout }
  expect    Playwright's expect, whose assertions wait until they hold
  cdp       a DevTools protocol session on the page
  keep      an object whose properties outlive the call, for a later call
  demi      the project's helpers:
    demi.up(...servers, { publicHost, built }) start backend, web, gallery (backend and web by default), sign in;
                                         publicHost 'localhost' lets a page at localhost make passkeys
                                         built serves the web app's build through vite preview
    demi.down(...servers)                stop those, or all and the browser; demi.down({ wipe: true }) also removes the data and the runner
    demi.stop()                          close the browser
    demi.shot(name, { page, element, region, pad, zoom, full, now })   save a PNG at the page's real size, of page (such as demi.webkit()'s) or the slot's; answers its path
    demi.timeline(action, [0, 100, 1500], { region, name })      frames painted those ms after the action
    demi.frames(probe)                   run probe in the page at every frame it draws; .stop() answers what each read, with when
    demi.focusRings({ root, name, shots })   Tab through the page and focus what Tab skips; report each focus indicator an ancestor clips or an element covers, with px per side, and photograph it
    demi.focusRings.of(locator)           the same for the indicator an element shows now, such as a selected capsule's outline
    demi.pixel(x, y)                     the colour at a point
    demi.net.latency(ms) .bandwidth(kbps | null) .offline() .unreachable() .online() .cut(pattern) .reset() .status()
    demi.ime(text, { into, commit })     compose text through an input method, in the order of into's engine; commit 'enter' | 'escape' | 'none'
    demi.webkit()                        a WebKit (Safari) page for this call, signed in; its goto takes app addresses
    demi.emulate({ viewport, scale, theme, device, locale, timeZone, reset })   without reloading
    demi.hide({ page }) .show({ page })  hide the slot's page or page behind another tab, so it sees visibilityState 'hidden' and visibilitychange, and bring it back; the call's end shows it
    demi.grant(permission, { origin })   such as 'clipboard' or 'local-network-access'
    demi.gallery(path)                   open a gallery page, such as '/session?view=blocks'
    demi.message(text, { files, wait, timeout })   send in the open conversation, with files from this machine, and wait for its turn, not for subagents; fails at once, with the page's words, when it is not delivered or the turn fails, and before writing when the draft holds files
    demi.turn({ timeout })               wait until the open conversation no longer runs, from a message on its way to the subagents and background jobs it started
    demi.runner({ fresh, backend }) .stop() .start({ backend })   the slot's runner, paired through Add Device; backend: another address to reach the backend at, such as a proxy
    demi.log.console() .network() .sockets() ({ all, modules }), demi.log.mark(name)
    demi.help()                          this text

--headed shows the browser's window.`
