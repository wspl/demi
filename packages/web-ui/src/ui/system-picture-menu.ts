/**
 * Whether the system shows a menu of its own for a picture's long press, with
 * Save to Photos, Copy and Share, as iOS and iPadOS do. A page cannot stop
 * that menu but by taking it away (`-webkit-touch-callout: none`), so there a
 * picture's long press is the system's (`context-menu.ts`), and a reply's
 * picture is no link, whose own menu the system would show instead
 * (`markdown/render.ts`).
 */
export const SYSTEM_PICTURE_MENUS = typeof CSS !== 'undefined' && CSS.supports('-webkit-touch-callout', 'none')
