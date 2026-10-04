package browserproto

import (
	"encoding/base64"
	"strconv"
)

// Package is the package's id, which its release descriptor names and the coding
// agent's commands and the backend's `browser` user stream bind to.
const Package = "demi.browser"

// Prefix is the prefix of every browser operation's name, such as `browser.open`.
const Prefix = "browser."

// TimeoutMS is the deadline of an operation whose input names none, in milliseconds.
const TimeoutMS = 30_000

// MaxTimeoutMS is the longest deadline an input may name, and `open`'s default: a cold start
// installs and launches Chrome first.
const MaxTimeoutMS = 300_000

// MaxNodes is the most nodes, entries or matches one result lists.
const MaxNodes = 1_000

// DefaultNodes is the number of entries a result lists when the input names no limit.
const DefaultNodes = 100

// InlineBytes is the largest result written to stdout; a larger one is shortened or fails.
const InlineBytes = 64 * 1024

// StdinBytes is the longest text an input carries, such as typed text or an expression,
// in Unicode scalar values.
const StdinBytes = 1024 * 1024

// ClipboardPNGBytes is the largest PNG a clipboard holds, in bytes.
const ClipboardPNGBytes = 16 * 1024 * 1024

// ClipboardPNGPixels is the most pixels a clipboard PNG holds.
const ClipboardPNGPixels = 16_000_000

// FetchURLs is the most URLs one `content.fetch` reads.
const FetchURLs = 10

// ConsoleEntries is the maximum number of console entries a tab retains.
const ConsoleEntries = 1000

// ConsoleBytes is the most bytes of console entries a tab retains.
const ConsoleBytes = 1024 * 1024

// CDPEvents is the maximum number of CDP events a debugging connection retains.
const CDPEvents = 10000

// CDPBytes is the most bytes of CDP events a debugging connection retains.
const CDPBytes = 8 * 1024 * 1024

// LocatorLength is the longest locator, URL, path or name an input carries, in Unicode
// scalar values.
const LocatorLength = 4096

// LiveOperation is the declared operation that serves a view (`native-runtime.md` § User
// streams).
const LiveOperation = "browser.live"

// ControlFrame is a frame's kind, the byte after its length.
const ControlFrame = 1

// VideoFrame is a video frame: a [VideoHeader], then H.264 Annex B data.
const VideoFrame = 2

// VideoCodec is the video frames' codec as WebCodecs names it: H.264 High profile (`64`),
// no constraint flags (`00`), level 5.1 (`33`). The capture extension
// encodes with it, and the page asks the user's browser for a decoder
// of it before it opens a view.
const VideoCodec = "avc1.640033"

// FileFrame is a chosen file's bytes: a [FileHeader], then the data.
const FileFrame = 3

// MaxFrameBytes is the largest frame after its length: a paste's text and HTML, or a key frame.
const MaxFrameBytes = 16 * 1024 * 1024

// FileChunkBytes is a file frame's largest data.
const FileChunkBytes = 64 * 1024

// HeartbeatMS is the maximum interval between module messages, so a still page is told from a stall.
const HeartbeatMS = 250

// StallMS is silence after which the page shows the stream as stalled and stops sending input.
const StallMS = 1000

// CaptureUnavailable is a notice's code when this Host cannot capture the watched tab.
const CaptureUnavailable = "capture_unavailable"

// CaptureFailed is a notice's code when the watched tab's capture failed; the next picture ends it.
const CaptureFailed = "capture_failed"

// ControlTokenPattern describes a control token: a UUID in lowercase, as
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

// DefaultLoad and the other default choices are used when an operation omits an optional choice.
const (
	// DefaultLoad is the load milestone used when none is requested.
	DefaultLoad = LoadDOMContentLoaded
	// DefaultInspectView is the inspection view used when none is requested.
	DefaultInspectView = InspectViewAccessibility
	// DefaultMouseButton is the mouse button used when none is requested.
	DefaultMouseButton = MouseButtonLeft
	// DefaultElementState is the element state used when none is requested.
	DefaultElementState = ElementStateVisible
	// DefaultClipboardMIME is the clipboard MIME type used when none is requested.
	DefaultClipboardMIME = ClipboardMIMETextPlain
	// DefaultContentFormat is the content format used when none is requested.
	DefaultContentFormat = ContentFormatText
	// DefaultViewportMode is the viewport mode used when none is requested.
	DefaultViewportMode = ViewportModeWeb
)
