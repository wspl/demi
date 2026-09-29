// Package shell defines the Host, command-system and shell-environment contracts.
// Control state belongs to its owning shard or environment goroutine. Byte
// streams may cross that boundary; their receiver must close them when done.
package shell

import (
	"context"
	"io"

	"github.com/wspl/demi/go/core"
)

// HostKey is an execution target's value identity: equal keys name one Host.
type HostKey string

type HostIdentity struct {
	UID      uint32
	GID      uint32
	Hostname string
	HomeDir  string
}

// Host paths are relative to DefaultCwd, which is not a permission boundary.
type Host interface {
	Key() HostKey
	DefaultCwd() string
	Identity() HostIdentity
	FS() HostFS
	Process() HostProcess
}

type HostErrorKind string

const (
	HostFailed      HostErrorKind = "failed"
	HostTooLarge    HostErrorKind = "too_large"
	HostOffline     HostErrorKind = "offline"
	HostUnavailable HostErrorKind = "unavailable"
	HostProtocol    HostErrorKind = "protocol"
	HostInterrupted HostErrorKind = "interrupted"
)

type HostError struct {
	Kind    HostErrorKind
	Message string
	Code    string // OS name, such as ENOENT, for HostFailed; empty when absent.
}

func (e *HostError) Error() string { return e.Message }

type ByteRange struct {
	Offset uint64
	Length *uint64
}
type FileKind string

const (
	FileRegular         FileKind = "file"
	FileDirectory       FileKind = "directory"
	FileSymlink         FileKind = "symlink"
	FileCharacterDevice FileKind = "character_device"
	FileFIFO            FileKind = "fifo"
	FileOther           FileKind = "other"
)

type FileStat struct {
	Kind     FileKind
	Mode     uint32
	Size     uint64
	Modified core.Timestamp
}
type DirEntry struct {
	Name string
	Kind FileKind
}
type WriteOptions struct{ CreateParents bool }
type MkdirOptions struct{ Recursive bool }
type RmOptions struct{ Recursive, Force bool }
type CpOptions struct{ Recursive bool }

// HostFS opens a read before returning its stream. Closing that stream stops
// the read. WriteFile owns contents through completion: a failure or cancellation
// leaves the destination unchanged and preserves the input stream's own error.
// Implementations close contents on cancellation to interrupt blocked input.
type HostFS interface {
	ReadFile(context.Context, string) ([]byte, error)
	ReadStream(context.Context, string, ByteRange) (io.ReadCloser, error)
	WriteFile(context.Context, string, io.ReadCloser, WriteOptions) error
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
	Utimes(context.Context, string, core.Timestamp, core.Timestamp) error
}

type HostProcess interface {
	Spawn(context.Context, SpawnRequest) (*Process, error)
}
type SpawnRequest struct {
	Command  string
	Args     []string
	Cwd      *string
	Env      SpawnEnv
	Retained bool
}

// A nil Values map inherits. A nonnil map replaces the environment unless
// Inherit is true; nil overlay values remove inherited variables.
type SpawnEnv struct {
	Values  map[string]*string
	Inherit bool
}

// Process has one reader for output and one waiter for its end. Close explicitly
// relinquishes control, asking the Host to kill a still-running process.
type Process struct {
	Output  ProcessOutputReader
	Control ProcessControl
	Wait    func(context.Context) (ProcessEnd, error)
}

func (p *Process) Close() error { return p.Control.Close() }

type ProcessOutputReader interface {
	Next(context.Context) (ProcessOutput, error)
}
type ProcessOutput struct {
	Stream core.StreamKind
	Bytes  []byte
}
type ProcessControl interface {
	WriteStdin(context.Context, []byte) error
	CloseStdin(context.Context) error
	Kill(context.Context, Signal) error
	Close() error
}
type Signal string

const (
	SignalTerminate Signal = "SIGTERM"
	SignalKill      Signal = "SIGKILL"
)

type ProcessEndKind string

const (
	ProcessExited     ProcessEndKind = "exited"
	ProcessSignalled  ProcessEndKind = "signalled"
	ProcessNotStarted ProcessEndKind = "not_started"
	ProcessLost       ProcessEndKind = "lost"
)

type ProcessEnd struct {
	Kind     ProcessEndKind
	ExitCode int32
	Signal   string
	Error    *SpawnError
	Reason   string
}
type SpawnError struct {
	Kind   SpawnErrorKind
	Detail *string
}
type SpawnErrorKind string

const (
	SpawnExecutableNotFound SpawnErrorKind = "executable_not_found"
	SpawnPermissionDenied   SpawnErrorKind = "permission_denied"
	SpawnCwdUnusable        SpawnErrorKind = "cwd_unusable"
	SpawnIsDirectory        SpawnErrorKind = "is_directory"
	SpawnOther              SpawnErrorKind = "other"
)
