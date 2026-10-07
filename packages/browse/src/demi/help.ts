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
    demi.up(...servers)                  start backend, web, gallery (backend and web by default), sign in
    demi.down(...servers)                stop those, or all and the browser; demi.down({ wipe: true }) also removes the data and the runner
    demi.stop()                          close the browser
    demi.shot(name, { element, region, pad, zoom, full, now })   save a PNG at the page's real size; answers its path
    demi.timeline(action, [0, 100, 1500], { region, name })      frames painted those ms after the action
    demi.pixel(x, y)                     the colour at a point
    demi.net.latency(ms) .bandwidth(kbps | null) .offline() .unreachable() .online() .cut(pattern) .reset() .status()
    demi.ime(text, { into, commit })     compose text through an input method
    demi.emulate({ viewport, scale, theme, device, locale, timeZone, reset })   without reloading
    demi.grant(permission, { origin })   such as 'clipboard' or 'local-network-access'
    demi.gallery(path)                   open a gallery page, such as '/session?view=blocks'
    demi.message(text, { wait, timeout })   send in the open conversation and wait for its turn
    demi.turn({ timeout })               wait for the open conversation's turn to end
    demi.runner({ fresh }) .stop() .start()   the slot's runner, paired through Add Device
    demi.log.console() .network() .sockets() ({ all, modules }), demi.log.mark(name)
    demi.help()                          this text

--headed shows the browser's window.`
