# Corrupt image isolation

All model-visible corruption notices use exactly `Image data is corrupted.`
They do not contain troubleshooting instructions, source paths, or additional
prompt rules.

## Boundaries

- Shell binary stdout is checked before it is attached as an image. A recognizable
  header alone is insufficient, including when the shell capture itself was not
  truncated (for example `browser screenshot | head -20`).
- Custom tool results are checked before their final content is committed.
- New user inputs are checked after reference resolution, before storage.
- New steering content is checked before delivery and storage.
- Invalid images become a text block. Other blocks and tool result status remain
  unchanged. Existing history is never scanned during inference, resume, or
  compaction. Corrupt legacy attachments require explicit one-off operational
  repair. Original shell artifacts remain available through the existing store.

## Validation and limits

The platform-neutral utility validates PNG chunk bounds, header fields and CRCs;
JPEG marker/segment/scan framing; GIF image and extension sub-blocks and trailer;
and WebP RIFF/chunk bounds, frame headers, and animated frame nesting. This detects
truncated containers and PNG byte corruption without sending media to a model.
It is a structural integrity check, not a full pixel decoder or a guarantee of
provider acceptance of every otherwise well-framed encoding.

Inline data URLs and base64 tool images are checked as bytes, including MIME
mismatches. Ordinary remote image URLs retain their existing provider-side fetch
semantics: the local layer does not fetch arbitrary URLs to validate them.

Validation runs at new-content boundaries without a content cache or historical
replay traversal. Provider errors do not trigger blanket image removal or hidden retries.

## Tests

- `packages/utils/src/__tests__/image-integrity.test.ts`: Pillow-generated PNG,
  JPEG, GIF and WebP fixtures, animated GIF/WebP, every truncated prefix, MIME
  mismatch, PNG CRC corruption, and a truncated IDAT with a fabricated IEND.
- `packages/agent/src/__tests__/tools.test.ts`: shell image attachment and a
  partial PNG under the capture limit.
- `packages/agent/src/__tests__/image-integrity.test.ts`: entry filtering,
  neighboring content, source preservation, malformed base64/data URLs, source
  mutation, historical replay without scanning, new user and tool storage,
  and direct in-flight steering. Providers are local stubs; no model calls.
