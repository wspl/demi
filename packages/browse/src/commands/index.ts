// The commands of `bun browse`, one file each, in the order `help` lists
// them. A command's module loads only when it runs.
import type { Context } from '../command'

export interface CommandEntry {
  name: string
  usage: string
  summary: string
  load: () => Promise<{ run: (context: Context, argv: string[]) => Promise<void> }>
}

export const COMMANDS: CommandEntry[] = [
  {
    name: 'up',
    usage: 'up [backend] [web] [gallery]',
    summary: 'Start the slot\'s servers (backend and web by default), wait until they answer, sign the browser in',
    load: () => import('./up'),
  },
  {
    name: 'down',
    usage: 'down [backend] [web] [gallery] [runner] | down --wipe',
    summary: 'Stop the servers, runner and browser this slot started, and delete the .env up copied; named servers alone, the rest kept; --wipe also removes the backend\'s data and the runner',
    load: () => import('./down'),
  },
  {
    name: 'open',
    usage: 'open <address>',
    summary: 'Open /path in the web app, gallery:/path in the gallery, or a URL',
    load: () => import('./open'),
  },
  {
    name: 'click',
    usage: 'click <locator|x,y> [--right | --middle] [--double] [--modifiers Meta,Shift]',
    summary: 'Click an element or a point',
    load: () => import('./click'),
  },
  {
    name: 'fill',
    usage: 'fill <locator> <text>',
    summary: 'Replace a field\'s text',
    load: () => import('./fill'),
  },
  {
    name: 'type',
    usage: 'type <text> [--into <locator>] [--delay <ms>]',
    summary: 'Type text key by key into the focused element, or into one',
    load: () => import('./type'),
  },
  {
    name: 'press',
    usage: 'press <key>... [--into <locator>]',
    summary: 'Press keys or shortcuts, such as Enter, Escape or Meta+Comma',
    load: () => import('./press'),
  },
  {
    name: 'hover',
    usage: 'hover <locator|x,y>',
    summary: 'Move the pointer over an element or a point',
    load: () => import('./hover'),
  },
  {
    name: 'select',
    usage: 'select <locator> <option>...',
    summary: 'Choose options of a <select> by value or label',
    load: () => import('./select'),
  },
  {
    name: 'upload',
    usage: 'upload <locator> <file>...',
    summary: 'Give files to a file input, or to the chooser a control opens',
    load: () => import('./upload'),
  },
  {
    name: 'drag',
    usage: 'drag <from locator|x,y> <to locator|x,y> [--steps <n>] [--hold] | drag --release [<to locator|x,y>]',
    summary: 'Drag from one element or point to another; --hold keeps the button down until drag --release',
    load: () => import('./drag'),
  },
  {
    name: 'scroll',
    usage: 'scroll <locator|x,y> [dx] [dy]',
    summary: 'Scroll an element into view, or turn the wheel by dx,dy over it (negative up or left)',
    load: () => import('./scroll'),
  },
  {
    name: 'ime',
    usage: 'ime <text> [--into <locator>] [--commit enter|none]',
    summary: 'Compose text through an input method and commit it, as Chinese or Japanese input does',
    load: () => import('./ime'),
  },
  {
    name: 'wait',
    usage: 'wait <locator> | gone <locator> | text <text> | url <pattern> | js <expression> | turn [--timeout <s>]',
    summary: 'Wait for an element to appear or go, text, an address, a condition, or the end of the agent\'s turn',
    load: () => import('./wait'),
  },
  {
    name: 'focus',
    usage: 'focus [--expect <locator>]',
    summary: 'Print the element that has the keyboard focus, or fail unless it is the one named',
    load: () => import('./focus'),
  },
  {
    name: 'shot',
    usage: 'shot [name|path.png] [--element <locator>] [--region x,y,w,h] [--pad <px>] [--zoom <n>] [--full] [--now]',
    summary: 'Save a PNG at the page\'s real size and pixel ratio, once its transitions end, and print its path',
    load: () => import('./shot'),
  },
  {
    name: 'timeline',
    usage: 'timeline \'<command>\' --at <ms>,<ms>,... [--region x,y,w,h] [--name <prefix>]',
    summary: 'Run one command and keep the frames painted at those milliseconds after it, or an area of them',
    load: () => import('./timeline'),
  },
  {
    name: 'download',
    usage: 'download \'<command>\' [--timeout <s>]',
    summary: 'Run one command and keep the download it starts in the browser, with its name and size',
    load: () => import('./download'),
  },
  {
    name: 'pixel',
    usage: 'pixel <x> <y>',
    summary: 'Print the colour the page shows at a point',
    load: () => import('./pixel'),
  },
  {
    name: 'emulate',
    usage: 'emulate [--size WxH] [--scale <n>] [--theme light|dark] [--device <name>] [--locale <id>] [--timezone <id>] [--reset]',
    summary: 'Set the viewport, pixel ratio, theme, a phone, locale or time zone',
    load: () => import('./emulate'),
  },
  {
    name: 'net',
    usage: 'net [offline | unreachable | online | latency <ms> | bandwidth <kbit/s>|off | cut <pattern> | reset]',
    summary: 'Take the page offline, make the server unreachable, add latency, limit bandwidth, or cut its sockets without a close',
    load: () => import('./net'),
  },
  {
    name: 'grant',
    usage: 'grant <permission>... [--origin <url>] | grant --clear',
    summary: 'Grant permissions such as clipboard or local-network-access',
    load: () => import('./grant'),
  },
  {
    name: 'log',
    usage: 'log console|network|sockets [--all] [--modules] | log mark [name]',
    summary: 'Print what the page logged, requested or sent on sockets since the last mark',
    load: () => import('./log'),
  },
  {
    name: 'message',
    usage: 'message <text> [--no-wait] [--timeout <s>]',
    summary: 'Send a message in the open conversation and wait for its turn to end',
    load: () => import('./message'),
  },
  {
    name: 'runner',
    usage: 'runner [stop | start] [--new]',
    summary: 'Start the slot\'s runner, paired through Add Device the first time; stop and start it as a device that goes away and comes back; --new pairs a new one',
    load: () => import('./runner'),
  },
  {
    name: 'eval',
    usage: 'eval <javascript>',
    summary: 'Run JavaScript in the page and print the result',
    load: () => import('./eval'),
  },
  {
    name: 'cdp',
    usage: 'cdp <method> [params as JSON]',
    summary: 'Send a DevTools protocol command to the page and print the answer',
    load: () => import('./cdp'),
  },
  {
    name: 'script',
    usage: 'script <file> [args...]',
    summary: 'Run a module\'s default export with the page, its context and a CDP session',
    load: () => import('./script'),
  },
  {
    name: 'stop',
    usage: 'stop',
    summary: 'Close the slot\'s browser and end its daemon',
    load: () => import('./stop'),
  },
]

export function findCommand(name: string): CommandEntry | undefined {
  return COMMANDS.find((command) => command.name === name)
}

/** What `bun browse help` prints. */
export function helpText(): string {
  const width = Math.max(...COMMANDS.map((command) => command.name.length))
  const lines = COMMANDS.map((command) => `  ${command.name.padEnd(width)}  ${command.summary}`)
  return [
    'bun browse <command> [arguments] [--headed]: product checks in the slot\'s browser (docs/delivery/browse.md)',
    '',
    ...lines,
    '',
    'bun browse help <command> prints its usage. A locator is Playwright\'s: role=button[name="Reload"], text=… (the',
    'whole text), text~=… (a part of it), label=…, testid=…, CSS, chained with >>; a point is x,y in CSS pixels.',
    '--headed shows the browser\'s window.',
  ].join('\n')
}
