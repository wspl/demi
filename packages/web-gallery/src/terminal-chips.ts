import { useSessionPanels } from '@demicodes/web-ui/agent/useSessionPanels'
import type { ChipKind, TerminalRecord } from '@demicodes/web-ui/agent/terminals'
import { productWould } from './product-would'

/**
 * The state of a specimen's Running and Service chips, as the product's
 * dock keeps it (`useSessionPanels`): the window opens on the chip's oldest
 * command, moves to it while it shows the other chip's, and closes when the
 * chip that owns it is pressed. The window itself is the product's, so a
 * press says in a toast what the product would show.
 */
export function useGalleryTerminalChips(terminals: () => readonly TerminalRecord[]) {
  const { activeTerminalId, toggleTerminals } = useSessionPanels(
    () => [],
    terminals,
    () => 'gallery-terminal-chips',
  )
  function press(kind: ChipKind): void {
    toggleTerminals(kind)
    const shown = terminals().find((terminal) => terminal.id === activeTerminalId.value)
    productWould(shown ? `Show ${shown.title} in the Terminals Window` : 'Close the Terminals Window')
  }
  return { activeId: activeTerminalId, press }
}
