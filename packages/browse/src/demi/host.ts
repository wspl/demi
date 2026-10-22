// `demi.host(name)`: has the open conversation run on the device `name`, which the page's
// header menu chooses as a person does, by the hook `HostMenu` gives its button, so it works in
// any language. A device runs the commands of its conversations with its own environment, such
// as a paired runner's SDKs and simulators, which the Cloud's stand-in of the development setup
// does not have.
//
// A conversation that has messages asks before it moves, a question the call does not answer:
// choose the device before the first message, as `demi.runner()` pairs it before one.
import type { Page } from 'playwright'
import { Failure, type Tool } from '../tool'

/** How long the page may take to show the device it was asked to choose. */
const CHOOSE_MS = 10_000

export async function host(tool: Tool, name: string): Promise<void> {
  return hostOn(await tool.browser.page(), name)
}

/** `demi.host` on `page`, which shows a conversation of the web app. */
export async function hostOn(page: Page, name: string): Promise<void> {
  const trigger = page.getByRole('button').and(page.locator('[data-host-menu]'))
  await trigger.click()
  // A row of the menu is a Host's name with its state under it; the name alone is a text of its own.
  const choice = page.getByRole('menuitem').filter({ has: page.getByText(name, { exact: true }) })
  if (await choice.count() === 0) {
    const listed = (await page.getByRole('menuitem').allInnerTexts()).map((row) => row.split('\n')[0].trim())
    await page.keyboard.press('Escape')
    throw new Failure(`No Host is named ${name}; the menu lists ${listed.join(', ')}`)
  }
  if (await choice.getAttribute('aria-disabled') === 'true') {
    await page.keyboard.press('Escape')
    throw new Failure(`${name} is not online, so the conversation cannot run on it; demi.runner.start() starts the slot's runner`)
  }
  await choice.click()
  // The button names the Host the conversation runs on once the page moved it.
  const moved = await trigger.getByText(name, { exact: true }).waitFor({ timeout: CHOOSE_MS }).then(() => true, () => false)
  if (!moved) {
    throw new Failure(
      `The conversation does not run on ${name} after choosing it: it shows ${(await trigger.innerText()).trim()}. `
      + 'A conversation with messages asks before it moves; choose the device before its first message',
    )
  }
}
