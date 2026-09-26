import type { z } from 'zod'
import { productStateSchema, type ProductState } from '../api/generated/web-api'

/**
 * A product state as `GET /state` answers it, for the page's tests: a master
 * of a shared instance with no providers, workspaces, devices, exposes or
 * conversations, without an expose domain, whose Cloud is not made yet. Each
 * of `parts` replaces a whole top-level field.
 */
export function productState(parts: Partial<z.input<typeof productStateSchema>> = {}): ProductState {
  return productStateSchema.parse({
    user: {
      id: 'user',
      email: 'test@example.test',
      nickname: 'Test',
      role: 'master',
      createdAt: '2026-09-10T00:00:00.000Z',
    },
    mode: 'shared',
    preferences: { appearance: {}, shortcuts: {} },
    providers: [],
    workspaces: [],
    devices: [],
    exposes: [],
    exposeDomain: null,
    publicUrl: 'http://127.0.0.1:3271/',
    conversations: [],
    cloud: {
      device: null,
      state: 'unallocated',
      operation: null,
      error: null,
      volumes: null,
      limits: { systemBytes: 16 * 1024 ** 3, homeBytes: 32 * 1024 ** 3 },
    },
    ...parts,
  })
}
