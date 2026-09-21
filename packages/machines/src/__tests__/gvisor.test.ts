import { expect, test } from 'bun:test'
import { gvisorConfigFromEnv } from '../gvisor/config'
import { SlotPool } from '../gvisor/slots'
import { managedBootSchema } from '@demicodes/runner-protocol'

const environment = {
  DEMI_MANAGED_RUNSC: '/opt/gvisor/runsc', DEMI_MANAGED_IMAGE: '/opt/image',
  DEMI_MANAGED_BACKEND_URL: 'https://backend.example.com', DEMI_MANAGED_DNS: '1.1.1.1,8.8.8.8',
}

test('reject obsolete settings and malformed or insufficient network pools', () => {
  expect(gvisorConfigFromEnv(environment, '/var/lib/demi').backendUrl).toBe('https://backend.example.com/')
  for (const settings of [
    { DEMI_MANAGED_FIRECRACKER: '/old' }, { DEMI_MANAGED_SUBNET: '172.30.1.0/16' },
    { DEMI_MANAGED_SUBNET: '172.30.0.0/30' }, { DEMI_MANAGED_DNS: '127.0.0.1' },
    { DEMI_MANAGED_CPUS: '0' }, { DEMI_MANAGED_DNS: '1.1.1.1,' },
    { DEMI_MANAGED_RUNSC: 'runsc' }, { DEMI_MANAGED_BACKEND_URL: 'file:///tmp/backend' },
  ]) expect(() => gvisorConfigFromEnv({ ...environment, ...settings }, '/var/lib/demi')).toThrow()
})

test('slots have disjoint networks and are reusable after release', () => {
  const pool = new SlotPool('172.30.0.0/16', 3)
  const a = pool.take()
  const b = pool.take()
  expect(a).toMatchObject({ index: 0, gateway: '172.30.0.1', address: '172.30.0.2' })
  expect(b).toMatchObject({ index: 1, gateway: '172.30.0.5', address: '172.30.0.6' })
  expect(pool.slot(64)).toMatchObject({ gateway: '172.30.1.1', address: '172.30.1.2' })
  pool.take()
  expect(() => pool.take()).toThrow('All Cloud network slots')
  pool.release(b)
  expect(pool.take()).toEqual(b)
})

test('managed credentials reject injected fields and malformed values', () => {
  expect(managedBootSchema.parse({ backendUrl: 'https://backend.example.com', deviceToken: 'opaque' })).toBeDefined()
  for (const boot of [
    { backendUrl: 'ftp://example.com', deviceToken: 'opaque' },
    { backendUrl: 'https://example.com', deviceToken: 'secret\nargument' },
    { backendUrl: 'https://example.com', deviceToken: 'opaque', command: 'bad' },
  ]) expect(managedBootSchema.safeParse(boot).success).toBe(false)
})
