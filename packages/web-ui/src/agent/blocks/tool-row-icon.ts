import type { Component } from 'vue'
import { SquareTerminal, Zap } from '@lucide/vue'
import { toolRenderKind } from '../tool-rendering'

/**
 * The icon of a tool's row: a `shell` call, or a tool the runtime does not
 * have. A row that stands in for the call, such as the call being written or
 * the tail row it rolls into, shows the same.
 */
export function toolRowIcon(toolName: string): Component {
  switch (toolRenderKind(toolName)) {
    case 'shell':
      return SquareTerminal
    case 'generic':
      return Zap
  }
}
