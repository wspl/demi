package host

import (
	"context"

	"github.com/wspl/demi/internal/types"
)

// Key is the value identity of an execution target: equal keys mean the same Host.
type Key string

// Identity is the account a Host works as.
type Identity struct {
	UID, GID          uint32
	Hostname, HomeDir string
}

// Host is an execution target. Relative paths start at DefaultCWD, which is not a boundary.
type Host interface {
	Key() Key
	DefaultCWD() string
	Identity() Identity
	FS() FS
	Process() Process
}

// ErrorKind identifies why a Host operation failed.
type ErrorKind string

// The following constants name the supported variants.
const (
	Failed      ErrorKind = "failed"
	TooLarge    ErrorKind = "too_large"
	Offline     ErrorKind = "offline"
	Unavailable ErrorKind = "unavailable"
	Protocol    ErrorKind = "protocol"
	Interrupted ErrorKind = "interrupted"
)

// Error is a Host failure, including an operating system code when Kind is Failed.
type Error struct {
	Kind    ErrorKind
	Message string
	Code    string
}

// Error returns the failure message.
func (e *Error) Error() string { return e.Message }

// ByteStream delivers a file's bytes. EOF ends the stream; Close stops and releases it.
type ByteStream interface {
	Read(context.Context, []byte) (int, error)
	Close(context.Context) error
}

// ByteRange selects Length bytes from Offset, or the remainder when Length is nil.
type ByteRange struct {
	Offset uint64
	Length *uint64
}

// FileContents supplies bytes at once, or a cancellable stream when Stream is non-nil.
// WriteFile owns Stream: it closes it on success, failure and cancellation,
// and preserves its read error. Bytes is used only when Stream is nil.
type FileContents struct {
	Bytes  []byte
	Stream ByteStream
}

// FileKind describes a directory entry. Only Lstat and ReadDir report symlinks.
type FileKind string

// The following constants name the supported variants.
const (
	File            FileKind = "file"
	Directory       FileKind = "directory"
	Symlink         FileKind = "symlink"
	CharacterDevice FileKind = "character_device"
	FIFO            FileKind = "fifo"
	OtherFile       FileKind = "other"
)

// FileStat is a file's metadata.
type FileStat struct {
	Kind     FileKind
	Mode     uint32
	Size     uint64
	Modified types.Timestamp
}

// DirEntry is one directory entry.
type DirEntry struct {
	Name string
	Kind FileKind
}

// WriteOptions controls creation of parent directories.
type WriteOptions struct{ CreateParents bool }

// MkdirOptions controls recursive creation and acceptance of an existing directory.
type MkdirOptions struct{ Recursive bool }

// RmOptions controls recursive removal and acceptance of an absent path.
type RmOptions struct{ Recursive, Force bool }

// CpOptions controls recursive directory copying.
type CpOptions struct{ Recursive bool }

// FS is a Host's filesystem. WriteFile replaces a file atomically: errors or cancellation leave it unchanged.
// ReadStream opens the file before returning, so opening errors precede any bytes.
type FS interface {
	ReadFile(context.Context, string) ([]byte, error)
	ReadStream(context.Context, string, ByteRange) (ByteStream, error)
	WriteFile(context.Context, string, FileContents, WriteOptions) error
	Exists(context.Context, string) (bool, error)
	Stat(context.Context, string) (FileStat, error)
	Lstat(context.Context, string) (FileStat, error)
	ReadDir(context.Context, string) ([]DirEntry, error)
	Mkdir(context.Context, string, MkdirOptions) error
	Rm(context.Context, string, RmOptions) error
	Cp(context.Context, string, string, CpOptions) error
	Mv(context.Context, string, string) error
	Chmod(context.Context, string, uint32) error
	Symlink(context.Context, string, string) error
	Link(context.Context, string, string) error
	Readlink(context.Context, string) (string, error)
	Realpath(context.Context, string) (string, error)
	Utimes(context.Context, string, types.Timestamp, types.Timestamp) error
}

// Process starts programs outside a shell job. A failure to start is reported by the child's end.
type Process interface {
	Spawn(context.Context, SpawnRequest) (*StartedProcess, error)
}

// SpawnRequest describes a process to start. A nil CWD uses the Host's default.
type SpawnRequest struct {
	Command  string
	Args     []string
	CWD      *string
	Env      SpawnEnv
	Retained bool
}

// EnvMode selects inheritance, an exact environment, or changes to the inherited environment.
type EnvMode uint8

// The following constants name the supported variants.
const (
	Inherit EnvMode = iota
	Exactly
	Overlay
)

// SpawnEnv selects the child's environment. Nil overlay values remove variables.
type SpawnEnv struct {
	Mode   EnvMode
	Values map[string]*string
}

// StartedProcess owns a started process. Its caller must Close its control and join Wait.
// Wait is repeatable: each successful wait returns the same terminal result.
type StartedProcess struct {
	Output  ProcessStream
	Control ProcessControl
	Wait    func(context.Context) (ProcessEnd, error)
}

// ProcessStream delivers ordered stdout/stderr chunks until io.EOF.
type ProcessStream interface {
	Next(context.Context) (ProcessOutput, error)
}

// ProcessOutput is one read of a process's output.
type ProcessOutput struct {
	Stream types.StreamKind
	Bytes  []byte
}

// ProcessControl writes and signals a process. Operations after its end are no-ops.
// Close kills an unfinished process and releases control; Wait joins its completion.
type ProcessControl interface {
	WriteStdin(context.Context, []byte) error
	CloseStdin(context.Context) error
	Kill(context.Context, Signal) error
	Close(context.Context) error
}

// Signal selects termination or immediate killing.
type Signal string

// The following constants name the supported variants.
const (
	Terminate Signal = "SIGTERM"
	Kill      Signal = "SIGKILL"
)

// ProcessEndKind identifies how a process ended.
type ProcessEndKind uint8

// The following constants name the supported variants.
const (
	ProcessExited ProcessEndKind = iota
	ProcessSignalled
	ProcessNotStarted
	ProcessLost
)

// ProcessEnd reports an exit, signal, startup failure, or lost connection.
type ProcessEnd struct {
	Kind       ProcessEndKind
	ExitCode   int32
	Signal     string
	SpawnError *SpawnError
	Reason     string
}

// SpawnError describes why a process could not start.
type SpawnError struct {
	Kind   SpawnErrorKind
	Detail *string
}

// SpawnErrorKind is named the way the runner names the cause.
type SpawnErrorKind string

// The following constants name the supported variants.
const (
	ExecutableNotFound SpawnErrorKind = "executable_not_found"
	PermissionDenied   SpawnErrorKind = "permission_denied"
	CWDUnusable        SpawnErrorKind = "cwd_unusable"
	IsDirectory        SpawnErrorKind = "is_directory"
	OtherSpawnError    SpawnErrorKind = "other"
)
