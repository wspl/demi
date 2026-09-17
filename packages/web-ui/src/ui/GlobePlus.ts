import { createLucideIcon } from '@lucide/vue'

// Lucide has no GlobePlus. Match its GlobeCheck badge geometry with a plus.
export const GlobePlus = createLucideIcon('globe-plus', [
  ['path', { d: 'M16 5h6M19 2v6' }],
  ['path', { d: 'M2 12h20A10 10 0 1 1 12 2a14.5 14.5 0 0 0 0 20 14.5 14.5 0 0 0 4-10' }],
])
