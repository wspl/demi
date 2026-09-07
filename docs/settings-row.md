# Settings row alignment

`web-ui/settings/SettingsRow.vue` owns alignment for both product settings and
gallery examples. Leading icons and checkboxes center on the 20px title line.
Single-line rows center the leading slot vertically with the title, including
when minimum row height or taller controls add space. Multiline rows anchor it
to the first line.

The optional `controlsAlign` prop accepts `start` or `center`. When omitted,
controls align with the title if the `detail` slot is present, and center on the
text block otherwise. This uses content structure, not measured wrapped lines.
Callers can explicitly choose `start` for other dense content.

For example, an MCP row with title, target and tool tags uses first-line controls;
a Skills source with title and origin uses centered controls. MCP rows without
tool tags also center their controls. In narrow cards, controls still move below
the text and align right.

Interactive rows emit clicks from the row surface. With `isolateControls`, the
trailing control area stops click propagation, including button gaps. Model rows use
this to toggle enabled state from the title, tags and surrounding space.
Their checkbox handles its own click and keyboard input and stops click
propagation so it toggles only once.
