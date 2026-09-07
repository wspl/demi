# Settings row alignment

`web-ui/settings/SettingsRow.vue` owns alignment for both product settings and
gallery examples. Leading icons center on the 20px title line.

The optional `controlsAlign` prop accepts `start` or `center`. When omitted,
controls align with the title if the `detail` slot is present, and center on the
text block otherwise. This uses content structure, not measured wrapped lines.
Callers can explicitly choose `start` for other dense content.

For example, an MCP row with title, target and tool tags uses first-line controls;
a Skills source with title and origin uses centered controls. MCP rows without
tool tags also center their controls. In narrow cards, controls still move below
the text and align right.
