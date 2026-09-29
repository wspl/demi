package commandservice

import "time"

// Version is the command wire's protocol version, which a descriptor and a
// service's catalog declare.
const Version = 1

// The paths of the wire's requests.
const (
	InfoPath         = "/v1/info"
	InvokePath       = "/v1/invoke"
	ConversationPath = "/v1/conversation"
	NumbersPath      = "/v1/numbers"
	ShutdownPath     = "/v1/shutdown"
)

// The wire's limits (docs/execution/native-runtime.md § Validation and flow
// control). They are fixed: the peer's SDK holds the same values.
const (
	// MaxMetadataBytes is the most bytes of invocation metadata JSON, and of
	// any one JSON document of the wire.
	MaxMetadataBytes = 256 * 1024
	// MaxRecordBytes is the most bytes of one response record's payload or
	// one input chunk.
	MaxRecordBytes = 64 * 1024
	// maxHeaderBytes is the setting that puts exactly 16 KiB, the most bytes of
	// an HTTP/2 header list, on the wire: net/http adds 320 bytes (ten fields of
	// 32 bytes of overhead each) to it when it derives SETTINGS_MAX_HEADER_LIST_SIZE.
	maxHeaderBytes = 16*1024 - 320
	// outputQueueRecords is the most records an invocation queues.
	outputQueueRecords = 4
	// maxHTTP2Window is the largest flow-control window HTTP/2 allows.
	maxHTTP2Window = 1<<31 - 1
	// http2InitialWindow is the connection window every HTTP/2 connection
	// starts with, before any WINDOW_UPDATE.
	http2InitialWindow = 65535
)

// The wire's time limits.
const (
	// handshakeTimeout is how long the HTTP/2 handshake may take.
	handshakeTimeout = 10 * time.Second
	// metadataTimeout is how long a request has to send its metadata.
	metadataTimeout = 10 * time.Second
	// cancellationGrace is how long a handler has to stop once its
	// invocation is cancelled.
	cancellationGrace = 5 * time.Second
)

// MaxNumbers is the most numbers one request of the numbers stream reserves.
const MaxNumbers = 16

// ConversationNameChars is the longest conversation name.
const ConversationNameChars = 64

// A TargetTriple names a platform a package carries an executable for.
type TargetTriple string

// The targets a published release carries.
const (
	TargetDarwinArm64  TargetTriple = "aarch64-apple-darwin"
	TargetDarwinAmd64  TargetTriple = "x86_64-apple-darwin"
	TargetLinuxArm64   TargetTriple = "aarch64-unknown-linux-musl"
	TargetLinuxAmd64   TargetTriple = "x86_64-unknown-linux-musl"
	TargetWindowsArm64 TargetTriple = "aarch64-pc-windows-msvc"
	TargetWindowsAmd64 TargetTriple = "x86_64-pc-windows-msvc"
)
