package runnerwire

import (
	"encoding/json"

	"github.com/wspl/demi/internal/commandwire"
)

// A message from the backend to the runner.
// +demi:msgpack
// +demi:union tag=type
//
//sumtype:decl
type Inbound interface{ inbound() }

// Ends what the runner's services hold for the conversation
// (`resource-lifecycle.md` § Conversation release).
// +demi:variant Inbound conversation_release
type ConversationRelease struct {
	ID string `json:"id"`
	// +demi:pattern ^[A-Za-z0-9_-]{1,64}$
	ConversationID string `json:"conversationId"`
}

// +demi:variant Inbound hello_ok
type HelloOK struct {
	DeviceID string `json:"deviceId"`
}

// +demi:variant Inbound claim_pending
type ClaimPending struct {
	ClaimToken string `json:"claimToken"`
}

// +demi:variant Inbound claimed
type Claimed struct {
	DeviceToken DeviceToken `json:"deviceToken"`
}

// +demi:variant Inbound hello_error
type HelloError struct {
	Code   HelloErrorCode `json:"code"`
	Reason string         `json:"reason"`
}

// +demi:variant Inbound ping
type Ping struct {
}

// Flush writable filesystems before the guest is stopped; `sync_done`
// answers.
// +demi:variant Inbound sync
type Sync struct {
	ID string `json:"id"`
}

// The named backing image is now `bytes` large, or `error` says why it
// did not grow.
// +demi:variant Inbound volume_grown
type VolumeGrown struct {
	ID     string     `json:"id"`
	Volume VolumeName `json:"volume"`
	// +demi:range min=1
	Bytes uint64 `json:"bytes"`
	// +demi:nullable
	Error *string `json:"error"`
}

// +demi:variant Inbound spawn
type Spawn struct {
	SpawnID          string              `json:"spawnId"`
	Command          string              `json:"command"`
	Args             *[]string           `json:"args,omitempty"`
	CWD              *string             `json:"cwd,omitempty"`
	Env              *map[string]*string `json:"env,omitempty"`
	InheritEnv       *bool               `json:"inheritEnv,omitempty"`
	KillProcessGroup *bool               `json:"killProcessGroup,omitempty"`
}

// +demi:variant Inbound spawn_stdin
type SpawnStdin struct {
	SpawnID string    `json:"spawnId"`
	Bytes   WireBytes `json:"bytes"`
}

// +demi:variant Inbound spawn_stdin_end
type SpawnStdinEnd struct {
	SpawnID string `json:"spawnId"`
}

// +demi:variant Inbound spawn_kill
type SpawnKill struct {
	SpawnID string  `json:"spawnId"`
	Signal  *Signal `json:"signal,omitempty"`
}

// One job: `bash -c script` in `cwd` with exactly `env`. Its declared
// commands receive `context`; `stdin` and `stdout` attach the job's fd 0
// and fd 1 to pipes whose other ends are elsewhere.
// +demi:variant Inbound job_start
type JobStart struct {
	JobID string `json:"jobId"`
	// +demi:pattern ^[0-9a-f]{64}$
	ManifestHash *string                    `json:"manifestHash,omitempty"`
	Context      commandwire.CommandContext `json:"context"`
	Script       string                     `json:"script"`
	CWD          string                     `json:"cwd"`
	Env          map[string]string          `json:"env"`
	Stdin        *PipeRef                   `json:"stdin,omitempty"`
	Stdout       *PipeRef                   `json:"stdout,omitempty"`
}

// +demi:variant Inbound job_stdin
type JobStdin struct {
	JobID string    `json:"jobId"`
	Bytes WireBytes `json:"bytes"`
}

// +demi:variant Inbound job_stdin_end
type JobStdinEnd struct {
	JobID string `json:"jobId"`
}

// +demi:variant Inbound job_kill
type JobKill struct {
	JobID  string  `json:"jobId"`
	Signal *Signal `json:"signal,omitempty"`
}

// Starts or stops sending the job's output beyond each stream's first
// `JOB_VIEW_BYTES` (`runner.md` § Pipes and output). A job starts
// unfollowed.
// +demi:variant Inbound job_follow
type JobFollow struct {
	JobID  string `json:"jobId"`
	Follow bool   `json:"follow"`
}

// Streams the job's kept output, as it stands, into `output`; the
// runner answers with `job_read` (`runner.md` § Pipes and output).
// +demi:variant Inbound job_read
type JobRead struct {
	ID     string  `json:"id"`
	JobID  string  `json:"jobId"`
	Output PipeRef `json:"output"`
}

// The backend has read what it needs of an ended job: its directory
// goes.
// +demi:variant Inbound job_release
type JobRelease struct {
	JobID string `json:"jobId"`
}

// +demi:variant Inbound rpc_stdin_pull
type RPCStdinPull struct {
	CallID string `json:"callId"`
}

// The call's pipe ends, sent before anything else for the call.
// +demi:variant Inbound rpc_pipes
type RPCPipes struct {
	CallID string   `json:"callId"`
	Stdin  *PipeRef `json:"stdin,omitempty"`
	Stdout PipeRef  `json:"stdout"`
}

// The call's standard error view; standard output is the pipe.
// +demi:variant Inbound rpc_output
type RPCOutput struct {
	CallID string    `json:"callId"`
	Bytes  WireBytes `json:"bytes"`
}

// Follows the stdout pipe's drain, so the process has written everything
// before it exits with the code.
// +demi:variant Inbound rpc_exit
type RPCExit struct {
	CallID   string `json:"callId"`
	ExitCode uint8  `json:"exitCode"`
}

// Open a TCP stream on the device's network between the socket and the
// two pipes.
// +demi:variant Inbound net_open
type NetOpen struct {
	StreamID string `json:"streamId"`
	Host     string `json:"host"`
	// +demi:range min=1
	Port   uint16  `json:"port"`
	Input  PipeRef `json:"input"`
	Output PipeRef `json:"output"`
}

// Open a user stream: invoke `operation` of `package` in its resident
// service.
// +demi:check validateServiceOpen
// +demi:variant Inbound service_open
type ServiceOpen struct {
	StreamID string                        `json:"streamId"`
	Context  commandwire.CommandContext    `json:"context"`
	Package  commandwire.PackageDescriptor `json:"package"`
	// +demi:length chars min=1
	Operation string           `json:"operation"`
	Args      *json.RawMessage `json:"args,omitempty"`
	JSON      *bool            `json:"json,omitempty"`
	// +demi:length chars min=1
	CWD    string  `json:"cwd"`
	Input  PipeRef `json:"input"`
	Output PipeRef `json:"output"`
}

// Read the Host's log: up to `limit` lines after `since`, oldest first,
// of one `source` when named.
// +demi:variant Inbound log_read
type LogRead struct {
	ID    string  `json:"id"`
	Since *uint64 `json:"since,omitempty"`
	// +demi:range min=1 max=1000
	Limit uint64 `json:"limit"`
	// +demi:length chars min=1
	Source *string `json:"source,omitempty"`
}

// The command manifest for the runner's cache, carried opaque; the
// runner verifies it as a [`crate::manifest::Manifest`].
// +demi:variant Inbound manifest
type ManifestMessage struct {
	Manifest json.RawMessage `json:"manifest"`
}

// +demi:variant Inbound artifact_location
type ArtifactLocation struct {
	ID       string                        `json:"id"`
	Location *commandwire.ArtifactLocation `json:"location,omitempty"`
	Error    *string                       `json:"error,omitempty"`
}

// The numbers a `numbers_reserve` asked for: the first of them, or why
// there are none (`native-runtime.md` § Conversation numbers).
// +demi:variant Inbound numbers_reserved
type NumbersReserved struct {
	ID string `json:"id"`
	// +demi:range min=1
	First *uint64 `json:"first,omitempty"`
	// +demi:length chars min=1
	Error *string `json:"error,omitempty"`
}

// Stream `length` bytes from `offset`, or to the end, into `output`.
// +demi:variant Inbound fs_readFile
type FSReadFile struct {
	ID     string  `json:"id"`
	Path   string  `json:"path"`
	CWD    *string `json:"cwd,omitempty"`
	Offset *uint64 `json:"offset,omitempty"`
	Length *uint64 `json:"length,omitempty"`
	Output PipeRef `json:"output"`
}

// Fill the file from `input`.
// +demi:variant Inbound fs_writeFile
type FSWriteFile struct {
	ID            string  `json:"id"`
	Path          string  `json:"path"`
	CWD           *string `json:"cwd,omitempty"`
	CreateParents *bool   `json:"createParents,omitempty"`
	Input         PipeRef `json:"input"`
}

// +demi:variant Inbound fs_exists
type FSExists struct {
	ID   string  `json:"id"`
	Path string  `json:"path"`
	CWD  *string `json:"cwd,omitempty"`
}

// +demi:variant Inbound fs_stat
type FSStat struct {
	ID   string  `json:"id"`
	Path string  `json:"path"`
	CWD  *string `json:"cwd,omitempty"`
}

// +demi:variant Inbound fs_lstat
type FSLstat struct {
	ID   string  `json:"id"`
	Path string  `json:"path"`
	CWD  *string `json:"cwd,omitempty"`
}

// Lists a directory, each entry with its file type.
// +demi:variant Inbound fs_readdir
type FSReaddir struct {
	ID   string  `json:"id"`
	Path string  `json:"path"`
	CWD  *string `json:"cwd,omitempty"`
}

// +demi:variant Inbound fs_mkdir
type FSMkdir struct {
	ID        string  `json:"id"`
	Path      string  `json:"path"`
	CWD       *string `json:"cwd,omitempty"`
	Recursive *bool   `json:"recursive,omitempty"`
}

// +demi:variant Inbound fs_rm
type FSRm struct {
	ID        string  `json:"id"`
	Path      string  `json:"path"`
	CWD       *string `json:"cwd,omitempty"`
	Recursive *bool   `json:"recursive,omitempty"`
	Force     *bool   `json:"force,omitempty"`
}

// +demi:variant Inbound fs_cp
type FSCp struct {
	ID          string  `json:"id"`
	Path        string  `json:"path"`
	Destination string  `json:"destination"`
	CWD         *string `json:"cwd,omitempty"`
	Recursive   *bool   `json:"recursive,omitempty"`
}

// +demi:variant Inbound fs_mv
type FSMv struct {
	ID          string  `json:"id"`
	Path        string  `json:"path"`
	Destination string  `json:"destination"`
	CWD         *string `json:"cwd,omitempty"`
}

// +demi:variant Inbound fs_chmod
type FSChmod struct {
	ID   string  `json:"id"`
	Path string  `json:"path"`
	Mode uint32  `json:"mode"`
	CWD  *string `json:"cwd,omitempty"`
}

// +demi:variant Inbound fs_symlink
type FSSymlink struct {
	ID     string  `json:"id"`
	Target string  `json:"target"`
	Path   string  `json:"path"`
	CWD    *string `json:"cwd,omitempty"`
}

// +demi:variant Inbound fs_link
type FSLink struct {
	ID           string  `json:"id"`
	ExistingPath string  `json:"existingPath"`
	Path         string  `json:"path"`
	CWD          *string `json:"cwd,omitempty"`
}

// +demi:variant Inbound fs_readlink
type FSReadlink struct {
	ID   string  `json:"id"`
	Path string  `json:"path"`
	CWD  *string `json:"cwd,omitempty"`
}

// +demi:variant Inbound fs_realpath
type FSRealpath struct {
	ID   string  `json:"id"`
	Path string  `json:"path"`
	CWD  *string `json:"cwd,omitempty"`
}

// +demi:variant Inbound fs_utimes
type FSUtimes struct {
	ID    string    `json:"id"`
	Path  string    `json:"path"`
	Atime Timestamp `json:"atime"`
	Mtime Timestamp `json:"mtime"`
	CWD   *string   `json:"cwd,omitempty"`
}

// The working tree's changes under `root`.
// +demi:variant Inbound git_changes
type GitChangesMessage struct {
	ID   string `json:"id"`
	Root string `json:"root"`
}

// The file as the last commit has it, streamed whole into `output` after
// the reply.
// +demi:variant Inbound git_show
type GitShow struct {
	ID     string  `json:"id"`
	Root   string  `json:"root"`
	Path   string  `json:"path"`
	Output PipeRef `json:"output"`
}

// A message from the runner to the backend.
// +demi:msgpack
// +demi:union tag=type
//
//sumtype:decl
type Outbound interface{ outbound() }

// +demi:variant Outbound conversation_released
type ConversationReleased struct {
	ID    string  `json:"id"`
	Error *string `json:"error,omitempty"`
}

// Reserves `count` numbers of the conversation's `sequence` for a
// native service (`native-runtime.md` § Conversation numbers).
// +demi:variant Outbound numbers_reserve
type NumbersReserve struct {
	ID string `json:"id"`
	// +demi:pattern ^[A-Za-z0-9_-]{1,64}$
	ConversationID string                      `json:"conversationId"`
	Sequence       commandwire.ServiceSequence `json:"sequence"`
	// +demi:range min=1 max=16
	Count uint32 `json:"count"`
}

// Where to fetch an artifact, for the live work it serves.
// +demi:check validateArtifactResolve
// +demi:variant Outbound artifact_resolve
type ArtifactResolve struct {
	ID    string        `json:"id"`
	Owner ArtifactOwner `json:"owner"`
	// +demi:pattern ^[0-9a-f]{64}$
	SHA256 string `json:"sha256"`
	Target string `json:"target"`
}

// +demi:variant Outbound hello
type Hello struct {
	Protocol uint32 `json:"protocol"`
	// Absent on an unclaimed first start.
	DeviceToken *DeviceToken `json:"deviceToken,omitempty"`
	Runner      RunnerInfo   `json:"runner"`
}

// Liveness, with the count of running jobs the idle rule reads.
// +demi:variant Outbound pong
type Pong struct {
	Jobs uint64 `json:"jobs"`
}

// +demi:variant Outbound sync_done
type SyncDone struct {
	ID    string  `json:"id"`
	Error *string `json:"error,omitempty"`
}

// A writable volume is nearly full: the runner asks for this total size.
// +demi:variant Outbound volume_grow
type VolumeGrow struct {
	ID     string     `json:"id"`
	Volume VolumeName `json:"volume"`
	// +demi:range min=1
	Bytes uint64 `json:"bytes"`
}

// A failed fs call; `code` is the errno-style code when there is one.
// +demi:variant Outbound fs_error
type FSError struct {
	ID      string  `json:"id"`
	Code    *string `json:"code,omitempty"`
	Message string  `json:"message"`
}

// A failed working-tree call.
// +demi:variant Outbound git_error
type GitError struct {
	ID      string `json:"id"`
	Code    string `json:"code"`
	Message string `json:"message"`
}

// +demi:variant Outbound spawn_output
type SpawnOutput struct {
	SpawnID string       `json:"spawnId"`
	Stream  OutputStream `json:"stream"`
	Bytes   WireBytes    `json:"bytes"`
}

// A raw process's end: the code it exited with; else the name of the
// signal that ended it (`SIGKILL`), never other text; else, in
// `spawn_error`, why the runner has no status for it.
// +demi:variant Outbound spawn_exit
type SpawnExit struct {
	SpawnID string `json:"spawnId"`
	// +demi:nullable
	ExitCode   *int32      `json:"exitCode"`
	Signal     *string     `json:"signal,omitempty"`
	SpawnError *SpawnError `json:"spawnError,omitempty"`
}

// A stream's bytes from `offset` while the job runs (`runner.md`
// § Pipes and output). Beyond the stream's first `JOB_VIEW_BYTES`, an
// offset past the end of the stream's previous bytes says the runner
// left the bytes between out, and a message without bytes says only
// that the stream is `offset` bytes long.
// +demi:variant Outbound job_output
type JobOutput struct {
	JobID  string       `json:"jobId"`
	Stream OutputStream `json:"stream"`
	Offset uint64       `json:"offset"`
	Bytes  WireBytes    `json:"bytes"`
}

// A registered leaf's guidance while its invocation is active; none
// clears that invocation's.
// +demi:variant Outbound job_running_hint
type JobRunningHint struct {
	JobID        string `json:"jobId"`
	InvocationID string `json:"invocationId"`
	// +demi:nullable
	Hint *string `json:"hint"`
}

// A job's end, its status given as `spawn_exit` gives a process's.
// +demi:variant Outbound job_exit
type JobExit struct {
	JobID string `json:"jobId"`
	// +demi:nullable
	ExitCode   *int32      `json:"exitCode"`
	Signal     *string     `json:"signal,omitempty"`
	SpawnError *SpawnError `json:"spawnError,omitempty"`
	// The directory the script ended in; absent when bash never ran it.
	CWD *string `json:"cwd,omitempty"`
	// Absent when bash never ran the script.
	Output *OutputLengths `json:"output,omitempty"`
	// +demi:length max=500
	Files          []JobFileChange `json:"files"`
	FilesTruncated bool            `json:"filesTruncated"`
}

// The answer to `job_read`: the kept output flows through the pipe; an
// error says why none does.
// +demi:variant Outbound job_read
type JobReadReply struct {
	ID    string  `json:"id"`
	Error *string `json:"error,omitempty"`
}

// An `rpc` command invoked on the target. It names only its job; `stdin`
// says whether the process has a pipe on fd 0.
// +demi:check validateRPCCall
// +demi:variant Outbound rpc_call
type RPCCall struct {
	JobID  string            `json:"jobId"`
	CallID string            `json:"callId"`
	Root   string            `json:"root"`
	Path   []string          `json:"path"`
	Argv   []string          `json:"argv"`
	Args   json.RawMessage   `json:"args"`
	JSON   bool              `json:"json"`
	CWD    string            `json:"cwd"`
	Env    map[string]string `json:"env"`
	Stdin  bool              `json:"stdin"`
}

// +demi:variant Outbound rpc_stdin
type RPCStdin struct {
	CallID string    `json:"callId"`
	Bytes  WireBytes `json:"bytes"`
}

// +demi:variant Outbound rpc_stdin_end
type RPCStdinEnd struct {
	CallID string `json:"callId"`
}

// +demi:variant Outbound rpc_cancel
type RPCCancel struct {
	CallID string `json:"callId"`
}

// This runner's end of a pipe closed: its HTTP exchange completed, or why
// it did not.
// +demi:variant Outbound pipe_done
type PipeDone struct {
	PipeID string  `json:"pipeId"`
	Ok     bool    `json:"ok"`
	Error  *string `json:"error,omitempty"`
}

// +demi:variant Outbound net_opened
type NetOpened struct {
	StreamID string `json:"streamId"`
}

// +demi:variant Outbound net_error
type NetError struct {
	StreamID string       `json:"streamId"`
	Code     NetErrorCode `json:"code"`
	Message  string       `json:"message"`
}

// +demi:variant Outbound service_opened
type ServiceOpened struct {
	StreamID string `json:"streamId"`
}

// +demi:variant Outbound service_error
type ServiceError struct {
	StreamID string           `json:"streamId"`
	Code     ServiceErrorCode `json:"code"`
	Message  string           `json:"message"`
}

// A user stream's invocation completed: its exit code and the bounded
// tail of its standard error.
// +demi:variant Outbound service_done
type ServiceDone struct {
	StreamID string `json:"streamId"`
	ExitCode uint8  `json:"exitCode"`
	// +demi:length chars max=16384
	Stderr string `json:"stderr"`
}

// +demi:variant Outbound log_lines
type LogLines struct {
	ID string `json:"id"`
	// +demi:length max=1000
	Lines []LogLine `json:"lines"`
	Next  uint64    `json:"next"`
}

// +demi:variant Outbound log_error
type LogError struct {
	ID      string `json:"id"`
	Message string `json:"message"`
}

// Every command package install in progress on this runner, whenever
// the list changes (`native-runtime.md` § Installation progress).
// +demi:variant Outbound installs
type Installs struct {
	// +demi:length max=64
	Installs []Install `json:"installs"`
}

// One artifact being installed for a command package: its line's name and
// its version, such as `Chrome for Testing` and `153.0.8010.36`, and how
// far it is, in bytes of its size.
// +demi:root
// +demi:check validateInstall
type Install struct {
	// The package, such as `demi.browser`.
	// +demi:length chars min=1 max=200
	Package string `json:"package"`
	// +demi:length chars min=1 max=100
	Name string `json:"name"`
	// +demi:length chars min=1 max=100
	Version string       `json:"version"`
	Phase   InstallPhase `json:"phase"`
	// +demi:range max=9007199254740991
	Done uint64 `json:"done"`
	// +demi:range min=1 max=9007199254740991
	Total uint64 `json:"total"`
}

// Where an install is: downloading the artifact, or unpacking a
// resource's archive.
// +demi:enum download unpack
type InstallPhase string

// Values of InstallPhase.
const (
	InstallPhaseDownload InstallPhase = "download"
	InstallPhaseUnpack   InstallPhase = "unpack"
)

// Why a hello was refused. `already_connected` is the one outcome a runner
// retries: the token's live connection may be a half-open socket.
// +demi:enum unsupported_protocol unknown_device already_connected revoked internal
type HelloErrorCode string

// Values of HelloErrorCode.
const (
	HelloErrorCodeUnsupportedProtocol HelloErrorCode = "unsupported_protocol"
	HelloErrorCodeUnknownDevice       HelloErrorCode = "unknown_device"
	HelloErrorCodeAlreadyConnected    HelloErrorCode = "already_connected"
	HelloErrorCodeRevoked             HelloErrorCode = "revoked"
	HelloErrorCodeInternal            HelloErrorCode = "internal"
)

// A managed guest's writable volume.
// +demi:enum system home
type VolumeName string

// Values of VolumeName.
const (
	VolumeNameSystem VolumeName = "system"
	VolumeNameHome   VolumeName = "home"
)

// Why a `net_open` stream could not connect.
// +demi:enum refused unreachable resolve_failed timeout
type NetErrorCode string

// Values of NetErrorCode.
const (
	NetErrorCodeRefused       NetErrorCode = "refused"
	NetErrorCodeUnreachable   NetErrorCode = "unreachable"
	NetErrorCodeResolveFailed NetErrorCode = "resolve_failed"
	NetErrorCodeTimeout       NetErrorCode = "timeout"
)

// Why a `service_open` stream never opened: the package lacks the operation,
// its service could not start, or the runner turned the stream away.
// +demi:enum unknown_operation service_failed refused
type ServiceErrorCode string

// Values of ServiceErrorCode.
const (
	ServiceErrorCodeUnknownOperation ServiceErrorCode = "unknown_operation"
	ServiceErrorCodeServiceFailed    ServiceErrorCode = "service_failed"
	ServiceErrorCodeRefused          ServiceErrorCode = "refused"
)

// One end of a pipe: the id the runner reports `pipe_done` under, and the
// origin-relative URL its end `PUT`s to or `GET`s from.
type PipeRef struct {
	ID  string `json:"id"`
	URL string `json:"url"`
}

// The live work an artifact serves, which authorizes it: a job and the
// manifest it runs with, or a user stream.
// +demi:union untagged
//
//sumtype:decl
type ArtifactOwner interface{ artifactOwner() }

// +demi:variant
type JobArtifactOwner struct {
	JobID string `json:"jobId"`
	// +demi:pattern ^[0-9a-f]{64}$
	ManifestHash string `json:"manifestHash"`
}

// +demi:variant
type StreamArtifactOwner struct {
	StreamID string `json:"streamId"`
}

// What a runner says about itself in its hello.
// +demi:check validateRunnerInfo
type RunnerInfo struct {
	Name         string         `json:"name"`
	Platform     RunnerPlatform `json:"platform"`
	Version      string         `json:"version"`
	NativeTarget *string        `json:"nativeTarget,omitempty"`
	// Read at shell creation, so it arrives before any Host use.
	Identity HostIdentity `json:"identity"`
	// A runner booted as a managed host's init: it presents its token or is
	// refused, never paired.
	Managed *bool `json:"managed,omitempty"`
}

// The account the runner works as.
type HostIdentity struct {
	UID      uint32 `json:"uid"`
	GID      uint32 `json:"gid"`
	Hostname string `json:"hostname"`
	HomeDir  string `json:"homeDir"`
}

// A file's metadata, as `stat` or `lstat` reports it.
type FileStat struct {
	IsFile            bool      `json:"isFile"`
	IsDirectory       bool      `json:"isDirectory"`
	IsSymbolicLink    bool      `json:"isSymbolicLink"`
	Mode              uint32    `json:"mode"`
	Size              uint64    `json:"size"`
	Mtime             Timestamp `json:"mtime"`
	UID               *uint32   `json:"uid,omitempty"`
	GID               *uint32   `json:"gid,omitempty"`
	Ino               *uint64   `json:"ino,omitempty"`
	Dev               *uint64   `json:"dev,omitempty"`
	Nlink             *uint64   `json:"nlink,omitempty"`
	IsCharacterDevice *bool     `json:"isCharacterDevice,omitempty"`
	IsFIFO            *bool     `json:"isFIFO,omitempty"`
}

// One entry of a directory listing, with its file type.
type DirEntry struct {
	Name           string `json:"name"`
	IsFile         bool   `json:"isFile"`
	IsDirectory    bool   `json:"isDirectory"`
	IsSymbolicLink bool   `json:"isSymbolicLink"`
}

// A working tree's changes, as `git status` lists them. The web app
// receives it too, beside the directory it lists (`web-api.md` § File text
// and working tree changes).
// +demi:root
type GitChanges struct {
	Repository bool `json:"repository"`
	// Always written, null before the first commit.
	// +demi:nullable
	Head      *string     `json:"head"`
	Files     []GitChange `json:"files"`
	Truncated bool        `json:"truncated"`
	Watched   bool        `json:"watched"`
}

// One path `git status` lists in a working tree.
type GitChange struct {
	Path string `json:"path"`
	// git's two status letters: the index against HEAD, then the working
	// tree against the index; `??` for an untracked file.
	// +demi:pattern ^(?:[MTADRC][ MTDAR]| [MTDAR]|\?\?|DD|AU|UD|UA|DU|AA|UU)$
	Status string     `json:"status"`
	Kind   ChangeKind `json:"kind"`
	// The path before a rename.
	From *string `json:"from,omitempty"`
	// +demi:range max=9007199254740991
	Added uint64 `json:"added"`
	// +demi:range max=9007199254740991
	Removed uint64 `json:"removed"`
}

// +demi:enum added modified deleted renamed
type ChangeKind string

// Values of ChangeKind.
const (
	ChangeKindAdded    ChangeKind = "added"
	ChangeKindModified ChangeKind = "modified"
	ChangeKindDeleted  ChangeKind = "deleted"
	ChangeKindRenamed  ChangeKind = "renamed"
)

// Why the runner has no status for a process or job: it could not start, or
// the runner failed it before its end was known, which is `Other`. `detail`
// holds the runner's words.
type SpawnError struct {
	Kind   SpawnErrorKind `json:"kind"`
	Detail *string        `json:"detail,omitempty"`
}

// +demi:enum executable_not_found permission_denied cwd_unusable is_directory other
type SpawnErrorKind string

// Values of SpawnErrorKind.
const (
	SpawnErrorKindExecutableNotFound SpawnErrorKind = "executable_not_found"
	SpawnErrorKindPermissionDenied   SpawnErrorKind = "permission_denied"
	SpawnErrorKindCwdUnusable        SpawnErrorKind = "cwd_unusable"
	SpawnErrorKindIsDirectory        SpawnErrorKind = "is_directory"
	SpawnErrorKindOther              SpawnErrorKind = "other"
)

// Each stream's length when a job ended. The backend reads what it did not
// receive of them from the job's kept output (`runner.md` § Pipes and
// output).
type OutputLengths struct {
	StdoutBytes uint64 `json:"stdoutBytes"`
	StderrBytes uint64 `json:"stderrBytes"`
}

// A file a job changed: its edit record entry and its line counts.
// +demi:check validateJobFileChange
type JobFileChange struct {
	// +demi:length chars min=1
	Path string               `json:"path"`
	Kind commandwire.EditKind `json:"kind"`
	// +demi:length max=1000
	Edits   []commandwire.EditCopies `json:"edits"`
	Added   uint64                   `json:"added"`
	Removed uint64                   `json:"removed"`
}

// One line of the Host's log: when it was written, which source wrote it,
// the conversation the work belonged to when there was one, and the text.
type LogLine struct {
	At Timestamp `json:"at"`
	// +demi:length chars min=1
	Source string `json:"source"`
	// +demi:length chars min=1
	ConversationID *string `json:"conversationId,omitempty"`
	Text           string  `json:"text"`
}

func (*JobArtifactOwner) artifactOwner()    {}
func (*StreamArtifactOwner) artifactOwner() {}
