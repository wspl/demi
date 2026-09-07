# Button icon rotation

`Button` and `IconButton` share `ui/button-icon-spin.ts` in `web-ui`.

- `spinOnClick` starts one 600ms revolution on an enabled click.
- `spinning` keeps the icon rotating while work is active. Setting it to false
  finishes the current revolution before stopping. Both props can be combined
  so even an immediately completed action has visible feedback.
- `spinEnd` fires after the final revolution. A caller that removes the button
  on success can keep it mounted until this event, as the MCP restart row does.

Only direct SVG children rotate; labels and button surfaces remain still. Keep
the same icon mounted during work instead of replacing it with a spinner.
Repeated clicks do not reset an active revolution. Unmount cancels animation;
reduced-motion preferences preserve the lifecycle without visible rotation.

Model catalog refresh and skill source update use the shared rotation. The
Primitives gallery demonstrates click feedback and controlled start/stop.
