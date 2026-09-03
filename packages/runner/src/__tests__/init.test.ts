import { expect, test } from 'bun:test'
import { guestBootConfig, parseKernelCmdline } from '../init/cmdline'
import { BlockHomeImage, DEFAULT_GROWTH_POLICY, growthWanted, parseDf, sectorsWritten } from '../init/home-image'
import { GUEST_LAYOUT, initPlan, resolvConf, runInit, type InitStep } from '../init/plan'

// PID 1 without a kernel: the command line, the plan of spawns the boot
// is, the home image's untouched report and growth decision, each over
// recorded commands.

const CMDLINE = 'console=ttyS0 init=/demi-runner panic=1 reboot=k demi.backend=https://demi.example.com demi.token=tok-123 demi.ip=172.16.5.2/30 demi.gw=172.16.5.1 demi.dns=1.1.1.1,8.8.8.8 quiet\n'

test('the kernel command line: words, key=value pairs, quoted values', () => {
  const params = parseKernelCmdline('quiet root=/dev/vda demi.name="two words" x=')
  expect(params.get('quiet')).toBe('')
  expect(params.get('root')).toBe('/dev/vda')
  expect(params.get('demi.name')).toBe('two words')
  expect(params.get('x')).toBe('')
})

test('the guest configuration: backend, token, network; backend or token missing is fatal', () => {
  const config = guestBootConfig(CMDLINE)
  expect(config.backendUrl).toBe('https://demi.example.com')
  expect(config.deviceToken).toBe('tok-123')
  expect(config.network).toEqual({ address: '172.16.5.2/30', gateway: '172.16.5.1', dns: ['1.1.1.1', '8.8.8.8'] })
  expect(config.firstBoot).toBe(false)
  expect(guestBootConfig('demi.backend=http://b demi.token=t').network).toBeNull()
  expect(guestBootConfig('demi.backend=http://b demi.token=t demi.firstboot=1').firstBoot).toBe(true)
  expect(() => guestBootConfig('demi.token=t')).toThrow('demi.backend')
  expect(() => guestBootConfig('demi.backend=http://b')).toThrow('demi.token')
  expect(resolvConf(config.network!)).toBe('nameserver 1.1.1.1\nnameserver 8.8.8.8\n')
})

test('the plan: kernel filesystems, the upper pivoted over /, the home, the network — in that order', () => {
  const plan = initPlan(GUEST_LAYOUT, guestBootConfig(CMDLINE).network)
  const lines = plan.map((step) => `${step.command} ${step.args.join(' ')}`)
  const at = (needle: string) => lines.findIndex((line) => line.includes(needle))
  expect(lines[0]).toBe('mount -t proc proc /proc')
  expect(at('-t overlay')).toBeGreaterThan(at('-t tmpfs upper /run/upper'))
  expect(at('pivot_root /run/newroot /run/newroot/oldroot')).toBeGreaterThan(at('--move /proc /run/newroot/proc'))
  expect(at('-t ext4 /dev/vdb /home')).toBeGreaterThan(at('pivot_root'))
  expect(at('resize2fs /dev/vdb')).toBe(at('-t ext4 /dev/vdb /home') + 1)
  expect(at('ip addr add 172.16.5.2/30 dev eth0')).toBeGreaterThan(at('-t ext4'))
  expect(lines[lines.length - 1]).toBe('ip route add default via 172.16.5.1 dev eth0')
  expect(plan.filter((step) => step.tolerated).map((step) => step.args.join(' '))).toEqual(['-t devtmpfs dev /dev', '/dev/vdb'])
})

test('runInit stops at the first fatal failure and skips a tolerated one', async () => {
  const ran: string[] = []
  const logged: string[] = []
  const runner = (failing: string) => async (command: string, args: string[]) => {
    ran.push(`${command} ${args.join(' ')}`)
    return args.join(' ') === failing ? { code: 32, stderr: 'mount: unknown filesystem\n' } : { code: 0, stderr: '' }
  }
  const plan: InitStep[] = [
    { command: 'mount', args: ['a'] },
    { command: 'mount', args: ['b'], tolerated: true },
    { command: 'mount', args: ['c'] },
  ]
  await runInit(plan, runner('b'), (line) => logged.push(line))
  expect(ran).toEqual(['mount a', 'mount b', 'mount c'])
  expect(logged).toEqual(['init: mount b exited 32: mount: unknown filesystem'])
  ran.length = 0
  await expect(runInit(plan, runner('c'), () => {})).rejects.toThrow('init: mount c exited 32')
  expect(ran).toEqual(['mount a', 'mount b', 'mount c'])
})

const DISKSTATS = [
  ' 254       0 vda 1200 0 96000 300 0 0 0 0 0 200 300 0 0 0 0 0 0',
  ' 254      16 vdb 40 0 800 10 12 3 96 20 0 30 30 0 0 0 0 0 0',
].join('\n')

test('the home image: untouched while the sectors written stand at the baseline', async () => {
  let written = 96
  const ran: string[] = []
  const io = {
    run: async (command: string, args: string[]) => {
      ran.push(`${command} ${args.join(' ')}`)
      return { code: 0, stdout: new TextEncoder().encode(command === 'df' ? `Filesystem 1-blocks Used Available Capacity Mounted on\n/dev/vdb ${1024 ** 3} 0 ${available} 0% /home\n` : '') }
    },
    readFile: async () => new TextEncoder().encode(DISKSTATS.replace('12 3 96', `12 3 ${written}`)),
  }
  let available = 900 * 1024 ** 2
  expect(sectorsWritten(DISKSTATS, 'vdb')).toBe(96)
  expect(sectorsWritten(DISKSTATS, 'vdc')).toBeNull()
  const home = new BlockHomeImage(io, '/dev/vdb', '/home')
  await home.baseline()
  expect(await home.sync()).toEqual({ untouched: true })
  expect(ran).toEqual(['sync -f /home'])
  written = 104
  expect(await home.sync()).toEqual({ untouched: false })

  expect(await home.wanted()).toBeNull()
  available = 100 * 1024 ** 2
  expect(await home.wanted()).toBe(2 * 1024 ** 3)
  await home.grown(2 * 1024 ** 3)
  expect(ran[ran.length - 1]).toBe('resize2fs /dev/vdb')
})

test('df parsing and the growth reserve', () => {
  expect(parseDf('Filesystem 1-blocks Used Available Capacity Mounted on\n/dev/vdb 1000 400 600 40% /home\n')).toEqual({ totalBytes: 1000, availableBytes: 600 })
  expect(parseDf('garbage')).toBeNull()
  // A tenth of a large image is more than the fixed reserve; the fixed reserve rules a small one.
  const large = 100 * 1024 ** 3
  expect(growthWanted({ totalBytes: large, availableBytes: 11 * 1024 ** 3 })).toBeNull()
  expect(growthWanted({ totalBytes: large, availableBytes: 9 * 1024 ** 3 })).toBe(2 * large)
  expect(growthWanted({ totalBytes: 1024 ** 3, availableBytes: DEFAULT_GROWTH_POLICY.reserveBytes })).toBeNull()
  expect(growthWanted({ totalBytes: 1024 ** 3, availableBytes: DEFAULT_GROWTH_POLICY.reserveBytes - 1 })).toBe(2 * 1024 ** 3)
})
