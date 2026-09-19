import { BROWSER_MAX_NODES, BROWSER_MAX_TIMEOUT_MS, browserDefaultTimeout, browserOperations, type BrowserOperation } from '@demicodes/browser-protocol'
import type { CommandGroup, NativeCommand } from '@demicodes/shell'

const summaries: Record<string, string> = {
  open: 'Open a URL in a new tab on this Host; starts the conversation’s browser when needed.',
  tabs: 'List the conversation’s live browser tabs without starting a browser.',
  info: 'Read a tab’s URL, title and viewport.',
  goto: 'Navigate a tab to a URL.',
  back: 'Navigate to the previous history entry.',
  forward: 'Navigate to the next history entry.',
  reload: 'Reload a tab.',
  history: 'Read the tab’s navigation history.',
  close: 'Close a tab. Closing the last tab ends this browser; the next open starts fresh.',
  inspect: `Read accessibility names, roles, values, states and references; at most ${BROWSER_MAX_NODES} nodes.`,
  find: `Find nodes by reference, role/name, associated label, visible text, test ID or CSS; at most ${BROWSER_MAX_NODES} nodes.`,
  read: 'Read a matched element’s text, HTML, value, attribute or visible/enabled/checked state.',
  probe: 'Find elements at viewport coordinates and optionally save an annotated screenshot.',
  drag: 'Drag through ordered viewport points, releasing input on every exit.',
  'select-text': 'Select rendered text or position its cursor; prefix and suffix disambiguate.',
  upload: 'Attach Host files through a file input or chooser.',
  download: 'Trigger and save a completed download on this Host.',
  'clipboard.write': 'Write finite raw stdin to the managed clipboard with the declared MIME type.',
  'clipboard.read': 'Read clipboard text or export supported MIME entries to Host files.',
  logs: 'Read console entries without clearing them; use the returned cursor to continue.',
  'cdp.targets': 'List this tab and its debuggable child targets.',
  'cdp.detach': 'Close your debugging connection to this tab, releasing its pauses, breakpoints and interceptions.',
  'cdp.send': 'Send a scoped CDP method with a JSON parameter object from stdin.',
  'cdp.events': 'Read buffered CDP events or wait for events after a cursor.',
  'content.fetch': 'Read up to ten URLs in temporary tabs sharing this browser’s login state.',
  'assets.list': 'Inventory observed page resources and inline SVGs.',
  'assets.export': 'Export selected assets from a current inventory to Host files.',
  'webmcp.list': 'List tools registered by this page and their input schemas.',
  'webmcp.call': 'Call a registered page tool with JSON arguments from stdin.',
  screenshot: 'Capture a tab as pure PNG stdout, or save a new PNG file with --output.',
  click: 'Click one actionable element or an explicit viewport coordinate.',
  move: 'Move the pointer to an element or viewport coordinate.',
  scroll: 'Scroll at an element or viewport coordinate.',
  fill: 'Replace an editable element’s contents with text.',
  type: 'Type characters into a target or the current focus, preserving selection.',
  key: 'Press --key at the current focus or focus a supplied target first; use a key name or a + joined combination, such as Space, Enter or ControlOrMeta+A.',
  check: 'Set a checkbox or radio to the requested checked value.',
  select: 'Select native select options by value, label or index.',
  wait: 'Wait for a URL glob, element state or current-document load state, within a bounded deadline.',
  eval: 'Evaluate a read-only JavaScript expression; Chrome rejects side effects.',
  'viewport.set': 'Set this tab’s viewport in CSS pixels and its pixel ratio (--scale), until the user picks another mode.',
  'viewport.reset': 'Return this tab to Web mode, where the user’s live view decides its size.',
  'dialog.inspect': 'Read the pending JavaScript dialog.',
  'dialog.accept': 'Accept the pending JavaScript dialog, optionally supplying prompt text.',
  'dialog.dismiss': 'Dismiss the pending JavaScript dialog.',
  'content.read': 'Read or save the page’s text or HTML content.',
  capabilities: 'Report the browser’s available observation and evaluation capabilities.',
} satisfies Record<BrowserOperation, string>

/** Browser leaves derive their arguments and JSON results from the browser contract. */
export function createBrowserGroup(): CommandGroup {
  const root: CommandGroup = {
    name: 'browser',
    summary: 'Operate the conversation’s persistent browser tabs on the Host running this shell. Use inspect to obtain node references; never guess them.',
    subcommands: [],
  }
  for (const [name, schema] of Object.entries(browserOperations)) {
    const summary = summaries[name]
    if (!summary) throw new Error(`Missing browser command description: ${name}`)
    const path = name.split('.')
    const leaf: NativeCommand = {
      name: path.at(-1)!,
      kind: 'native',
      binding: { package: 'demi.builtin', operation: `browser.${name}` },
      summary,
      input: {
        ...schema.input.shape,
        timeout: schema.input.shape.timeout.describe(`Whole operation deadline in milliseconds; default ${browserDefaultTimeout(name)}, maximum ${BROWSER_MAX_TIMEOUT_MS}.`),
      },
      positionals: name === 'open' ? ['url'] : name === 'goto' ? ['tab', 'url'] : name === 'cdp.send' ? ['tab', 'method'] : name === 'webmcp.call' ? ['tab', 'tool'] : 'tab' in schema.input.shape ? ['tab'] : [],
      stdinField: name === 'eval' ? 'expression' : name === 'find' ? 'body' : name === 'cdp.send' ? 'params' : name === 'webmcp.call' ? 'arguments' : undefined,
      output: { json: schema.result },
      successOutput: name === 'screenshot' ? 'pure PNG bytes, or file metadata with --output; --json requires --output' : 'readable page results; one validated JSON value with --json',
      failureOutput: 'a browser error on stderr with nonzero exit status; an action is never replayed automatically',
    }
    if (path.length === 1) {
      root.subcommands.push(leaf)
    } else {
      const groupName = path[0]!
      let group = root.subcommands.find((command): command is CommandGroup => 'subcommands' in command && command.name === groupName)
      if (!group) {
        group = { name: groupName, summary: `Browser ${groupName} operations.`, subcommands: [] }
        root.subcommands.push(group)
      }
      group.subcommands.push(leaf)
    }
  }
  return root
}
