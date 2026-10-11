import type { Component } from 'vue'
import { Bell, Bot, Brain, SquareTerminal, Zap } from '@lucide/vue'
import { toolRenderKind } from '../tool-rendering'
import type { WorkStep } from '../work-groups'

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

/**
 * The icon of a step's own row, which a row that stands for the step, such
 * as the run's row while the step is its newest or the tail row it rolls
 * through, shows too; `key` stays put while the icon does, so it does not
 * roll with the label.
 */
export function stepIcon(step: WorkStep): { icon: Component, key: string } {
  switch (step.type) {
    case 'thinking':
      return { icon: Brain, key: 'thinking' }
    case 'tool_call':
    case 'pending_call': {
      const tool = step.type === 'tool_call' ? step.toolName : step.call.toolName
      return { icon: toolRowIcon(tool), key: toolRenderKind(tool) }
    }
    case 'wakeup':
      return { icon: Bell, key: 'notice' }
    case 'agent_message':
      return { icon: Bot, key: 'receipt' }
  }
}
