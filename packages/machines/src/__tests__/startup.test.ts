import { expect, test } from 'bun:test'
import { firecrackerConfigFromEnv } from '../firecracker/config'
import { machinesStartupFromEnv } from '../startup'

const base = {
  DEMI_MANAGED_FIRECRACKER: '/fc',
  DEMI_MANAGED_KERNEL: '/kernel',
  DEMI_MANAGED_ROOTFS: '/rootfs',
}

test('machine environment rejects invalid and inconsistent VM configuration without launching it', () => {
  for (const key of ['VCPUS', 'MEM_MIB', 'SYSTEM_MIB', 'HOME_MIB', 'SLOTS']) {
    for (const value of [
      '',
      ' ',
      '0',
      '-1',
      '1.5',
      'NaN',
      '9007199254740992',
      '0x10',
    ]) {
      expect(() =>
        firecrackerConfigFromEnv(
          { ...base, [`DEMI_MANAGED_${key}`]: value },
          '/data',
        ),
      ).toThrow(`DEMI_MANAGED_${key}`)
    }
  }
  for (const [key, value] of [
    ['FIRECRACKER', ''],
    ['KERNEL', ' '],
    ['ROOTFS', ''],
    ['DNS', ''],
    ['DNS', '1.1.1.1,'],
    ['DNS', '999.1.1.1'],
    ['SUBNET', '172.16.0.1/16'],
    ['SUBNET', '172.16.0.0/33'],
    ['SUBNET', '172.16.0.0/-1'],
    ['SUBNET', '172..0.0/16'],
    ['LAUNCH', ''],
    ['JAILER', '/jailer'],
  ]) {
    expect(() =>
      firecrackerConfigFromEnv(
        { ...base, [`DEMI_MANAGED_${key}`]: value },
        '/data',
      ),
    ).toThrow(`DEMI_MANAGED_${key}`)
  }
  expect(() =>
    firecrackerConfigFromEnv({ DEMI_MANAGED_SLOTS: '5' }, '/data'),
  ).toThrow('DEMI_MANAGED_FIRECRACKER')
  expect(() =>
    firecrackerConfigFromEnv(
      { ...base, DEMI_MANAGED_SUBNET: '172.16.0.0/30' },
      '/data',
    ),
  ).toThrow('slots')
  expect(
    firecrackerConfigFromEnv(
      {
        ...base,
        DEMI_MANAGED_SUBNET: '172.16.0.0/30',
        DEMI_MANAGED_SLOTS: '1',
      },
      '/data',
    ),
  ).toMatchObject({ slots: 1, subnet: '172.16.0.0/30' })
  expect(() => machinesStartupFromEnv(base, '/data')).toThrow(
    'DEMI_MACHINES_SOCKET',
  )
  expect(() =>
    machinesStartupFromEnv({ ...base, DEMI_MACHINES_SOCKET: ' ' }, '/data'),
  ).toThrow('DEMI_MACHINES_SOCKET')
  expect(() =>
    machinesStartupFromEnv(
      { ...base, DEMI_MACHINES_SOCKET: '/sock', DEMI_MACHINES_DATA: '' },
      '/data',
    ),
  ).toThrow('DEMI_MACHINES_DATA')
  expect(
    machinesStartupFromEnv({ ...base, DEMI_MACHINES_SOCKET: '/sock' }, '/data'),
  ).toMatchObject({ socketPath: '/sock', dataDir: '/data' })
})
