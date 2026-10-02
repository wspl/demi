package browserop

import (
	"encoding/base64"
	"strconv"
)

// The package's id, which its release descriptor names and the coding
// agent's commands and the backend's `browser` user stream bind to.
const Package = "demi.browser"

// The prefix of every browser operation's name, such as `browser.open`.
const Prefix = "browser."

// The deadline of an operation whose input names none, in milliseconds.
const TimeoutMS = 30_000

// The longest deadline an input may name, and `open`'s default: a cold start
// installs and launches Chrome first.
const MaxTimeoutMS = 300_000

// The most nodes, entries or matches one result lists.
const MaxNodes = 1_000

// How many a result lists when the input names no limit.
const DefaultNodes = 100

// The largest result written to stdout; a larger one is shortened or fails.
const InlineBytes = 64 * 1024

// The longest text an input carries, such as typed text or an expression,
// in Unicode scalar values.
const StdinBytes = 1024 * 1024

// The largest PNG a clipboard holds, and its most pixels.
const ClipboardPNGBytes = 16 * 1024 * 1024

// The most pixels a clipboard PNG holds.
const ClipboardPNGPixels = 16_000_000

// The most URLs one `content.fetch` reads.
const FetchURLs = 10

// The console entries a tab retains, and their most bytes.
const ConsoleEntries = 1000

// The most bytes of console entries a tab retains.
const ConsoleBytes = 1024 * 1024

// The CDP events a debugging connection retains, and their most bytes.
const CDPEvents = 10000

// The most bytes of CDP events a debugging connection retains.
const CDPBytes = 8 * 1024 * 1024

// The longest locator, URL, path or name an input carries, in Unicode
// scalar values.
const LocatorLength = 4096

// The declared operation that serves a view (`native-runtime.md` § User
// streams).
const LiveOperation = "browser.live"

// A frame's kind, the byte after its length.
const ControlFrame = 1

// A video frame: [`VideoHeader`], then H.264 Annex B data.
const VideoFrame = 2

// The video frames' codec as WebCodecs names it: H.264 High profile (`64`),
// no constraint flags (`00`), level 5.1 (`33`). The capture extension
// encodes with it, and the page asks the user's browser for a decoder
// of it before it opens a view.
const VideoCodec = "avc1.640033"

// A chosen file's bytes: [`FileHeader`], then the data.
const FileFrame = 3

// The largest frame after its length: a paste's text and HTML, or a key frame.
const MaxFrameBytes = 16 * 1024 * 1024

// A file frame's largest data.
const FileChunkBytes = 64 * 1024

// How often the module speaks at least, so a still page is told from a stall.
const HeartbeatMS = 250

// Silence after which the page shows the stream as stalled and stops sending input.
const StallMS = 1000

// A notice's code when this Host cannot capture the watched tab.
const CaptureUnavailable = "capture_unavailable"

// A notice's code when the watched tab's capture failed; the next picture ends it.
const CaptureFailed = "capture_failed"

// What a control token looks like: a UUID in lowercase, as
// `crypto.randomUUID` makes it. It is both the check and the schema.
const ControlTokenPattern = "^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$"

// Handle returns prefix, an underscore and URL-safe base64 of 16 random bytes.
func Handle(prefix string, random [16]byte) string {
	return prefix + "_" + base64.RawURLEncoding.EncodeToString(random[:])
}

// NumberedTabID returns the checked public identity of a tab number.
func NumberedTabID(number uint64) (TabID, error) {
	return ParseTabID("t" + strconv.FormatUint(number, 10))
}

// NumberedNodeRef returns the checked public reference of a node number.
func NumberedNodeRef(number uint64) (NodeRef, error) {
	return ParseNodeRef("e" + strconv.FormatUint(number, 10))
}

// Defaults used when an operation omits an optional choice.
const (
	DefaultLoad          = LoadDomContentLoaded
	DefaultInspectView   = InspectViewAccessibility
	DefaultMouseButton   = MouseButtonLeft
	DefaultElementState  = ElementStateVisible
	DefaultClipboardMime = ClipboardMimeTextPlain
	DefaultContentFormat = ContentFormatText
	DefaultViewportMode  = ViewportModeWeb
)
