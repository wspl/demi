package runnerproto

import (
	"encoding/json/jsontext"
	"errors"
	"regexp"

	"github.com/wspl/demi/go/commandservice"
)

var conversationPattern = regexp.MustCompile(`^[a-zA-Z0-9_-]+$`)

var gitStatusPattern = regexp.MustCompile(`^(?:[MTADRC][ MTDAR]| [MTDAR]|\?\?|DD|AU|UD|UA|DU|AA|UU)$`)

// jsonObject enforces the object-valued arguments of command invocations.
func jsonObject(v jsontext.Value) error {
	if v.Kind() != '{' {
		return errors.New("expected object")
	}
	return nil
}

//demi:union tag=type
//demi:msgpack
type Inbound interface{ inbound() }

//demi:variant conversation_release
type InboundConversationRelease struct {
	ID             string `json:"id"`
	ConversationID string `json:"conversationId" check:"chars=1..commandservice.ConversationNameChars,pattern=conversationPattern"`
}

func (InboundConversationRelease) inbound() {}

//demi:variant hello_ok
type InboundHelloOk struct {
	DeviceID string `json:"deviceId"`
}

func (InboundHelloOk) inbound() {}

//demi:variant claim_pending
type InboundClaimPending struct {
	ClaimToken string `json:"claimToken"`
}

func (InboundClaimPending) inbound() {}

//demi:variant claimed
type InboundClaimed struct {
	DeviceToken DeviceToken `json:"deviceToken" check:"func=deviceToken"`
}

func (InboundClaimed) inbound() {}

//demi:variant hello_error
type InboundHelloError struct {
	Code   HelloErrorCode `json:"code"`
	Reason string         `json:"reason"`
}

func (InboundHelloError) inbound() {}

//demi:variant ping
type InboundPing struct {
}

func (InboundPing) inbound() {}

//demi:variant sync
type InboundSync struct {
	ID string `json:"id"`
}

func (InboundSync) inbound() {}

//demi:variant volume_grown
type InboundVolumeGrown struct {
	ID     string     `json:"id"`
	Volume VolumeName `json:"volume"`
	Bytes  uint64     `json:"bytes" check:"range=1.."`
	Error  *string    `json:"error" check:"nullable"`
}

func (InboundVolumeGrown) inbound() {}

//demi:variant spawn
type InboundSpawn struct {
	SpawnID          string              `json:"spawnId"`
	Command          string              `json:"command"`
	Args             *[]string           `json:"args,omitzero"`
	Cwd              *string             `json:"cwd,omitzero"`
	Env              *map[string]*string `json:"env,omitzero"`
	InheritEnv       *bool               `json:"inheritEnv,omitzero"`
	KillProcessGroup *bool               `json:"killProcessGroup,omitzero"`
}

func (InboundSpawn) inbound() {}

//demi:variant spawn_stdin
type InboundSpawnStdin struct {
	SpawnID string `json:"spawnId"`
	Bytes   []byte `json:"bytes" msgpack:"bin"`
}

func (InboundSpawnStdin) inbound() {}

//demi:variant spawn_stdin_end
type InboundSpawnStdinEnd struct {
	SpawnID string `json:"spawnId"`
}

func (InboundSpawnStdinEnd) inbound() {}

//demi:variant spawn_kill
type InboundSpawnKill struct {
	SpawnID string  `json:"spawnId"`
	Signal  *Signal `json:"signal,omitzero"`
}

func (InboundSpawnKill) inbound() {}

//demi:variant job_start
type InboundJobStart struct {
	JobID        string                        `json:"jobId"`
	ManifestHash *string                       `json:"manifestHash,omitzero" check:"func=releaseDigest"`
	Context      commandservice.CommandContext `json:"context" check:"func=commandservice.Validate"`
	Script       string                        `json:"script"`
	Cwd          string                        `json:"cwd"`
	Env          map[string]string             `json:"env"`
	Stdin        *PipeRef                      `json:"stdin,omitzero"`
	Stdout       *PipeRef                      `json:"stdout,omitzero"`
}

func (InboundJobStart) inbound() {}

//demi:variant job_stdin
type InboundJobStdin struct {
	JobID string `json:"jobId"`
	Bytes []byte `json:"bytes" msgpack:"bin"`
}

func (InboundJobStdin) inbound() {}

//demi:variant job_stdin_end
type InboundJobStdinEnd struct {
	JobID string `json:"jobId"`
}

func (InboundJobStdinEnd) inbound() {}

//demi:variant job_kill
type InboundJobKill struct {
	JobID  string  `json:"jobId"`
	Signal *Signal `json:"signal,omitzero"`
}

func (InboundJobKill) inbound() {}

//demi:variant job_follow
type InboundJobFollow struct {
	JobID  string `json:"jobId"`
	Follow bool   `json:"follow"`
}

func (InboundJobFollow) inbound() {}

//demi:variant job_read
type InboundJobRead struct {
	ID     string  `json:"id"`
	JobID  string  `json:"jobId"`
	Output PipeRef `json:"output"`
}

func (InboundJobRead) inbound() {}

//demi:variant job_release
type InboundJobRelease struct {
	JobID string `json:"jobId"`
}

func (InboundJobRelease) inbound() {}

//demi:variant rpc_stdin_pull
type InboundRPCStdinPull struct {
	CallID string `json:"callId"`
}

func (InboundRPCStdinPull) inbound() {}

//demi:variant rpc_pipes
type InboundRPCPipes struct {
	CallID string   `json:"callId"`
	Stdin  *PipeRef `json:"stdin,omitzero"`
	Stdout PipeRef  `json:"stdout"`
}

func (InboundRPCPipes) inbound() {}

//demi:variant rpc_output
type InboundRPCOutput struct {
	CallID string `json:"callId"`
	Bytes  []byte `json:"bytes" msgpack:"bin"`
}

func (InboundRPCOutput) inbound() {}

//demi:variant rpc_exit
type InboundRPCExit struct {
	CallID   string `json:"callId"`
	ExitCode uint8  `json:"exitCode"`
}

func (InboundRPCExit) inbound() {}

//demi:variant net_open
type InboundNetOpen struct {
	StreamID string  `json:"streamId"`
	Host     string  `json:"host"`
	Port     uint16  `json:"port" check:"range=1.."`
	Input    PipeRef `json:"input"`
	Output   PipeRef `json:"output"`
}

func (InboundNetOpen) inbound() {}

//demi:variant service_open
type InboundServiceOpen struct {
	StreamID  string                           `json:"streamId"`
	Context   commandservice.CommandContext    `json:"context" check:"func=commandservice.Validate"`
	Package   commandservice.PackageDescriptor `json:"package" check:"func=commandservice.Validate"`
	Operation string                           `json:"operation" check:"chars=1.."`
	Args      *jsontext.Value                  `json:"args,omitzero" check:"func=jsonObject"`
	JSON      *bool                            `json:"json,omitzero"`
	Cwd       string                           `json:"cwd" check:"chars=1.."`
	Input     PipeRef                          `json:"input"`
	Output    PipeRef                          `json:"output"`
}

func (InboundServiceOpen) inbound() {}

//demi:variant log_read
type InboundLogRead struct {
	ID     string  `json:"id"`
	Since  *uint64 `json:"since,omitzero"`
	Limit  uint64  `json:"limit" check:"range=1..LogReadLines"`
	Source *string `json:"source,omitzero" check:"chars=1.."`
}

func (InboundLogRead) inbound() {}

//demi:variant manifest
type InboundManifest struct {
	Manifest jsontext.Value `json:"manifest"`
}

func (InboundManifest) inbound() {}

//demi:variant artifact_location
type InboundArtifactLocation struct {
	ID       string                           `json:"id"`
	Location *commandservice.ArtifactLocation `json:"location,omitzero" check:"func=commandservice.Validate"`
	Error    *string                          `json:"error,omitzero"`
}

func (InboundArtifactLocation) inbound() {}

//demi:variant numbers_reserved
type InboundNumbersReserved struct {
	ID    string  `json:"id"`
	First *uint64 `json:"first,omitzero" check:"range=1.."`
	Error *string `json:"error,omitzero" check:"chars=1.."`
}

func (InboundNumbersReserved) inbound() {}

//demi:variant fs_readFile
type InboundFSReadFile struct {
	ID     string  `json:"id"`
	Path   string  `json:"path"`
	Cwd    *string `json:"cwd,omitzero"`
	Offset *uint64 `json:"offset,omitzero"`
	Length *uint64 `json:"length,omitzero"`
	Output PipeRef `json:"output"`
}

func (InboundFSReadFile) inbound() {}

//demi:variant fs_writeFile
type InboundFSWriteFile struct {
	ID            string  `json:"id"`
	Path          string  `json:"path"`
	Cwd           *string `json:"cwd,omitzero"`
	CreateParents *bool   `json:"createParents,omitzero"`
	Input         PipeRef `json:"input"`
}

func (InboundFSWriteFile) inbound() {}

//demi:variant fs_exists
type InboundFSExists struct {
	ID   string  `json:"id"`
	Path string  `json:"path"`
	Cwd  *string `json:"cwd,omitzero"`
}

func (InboundFSExists) inbound() {}

//demi:variant fs_stat
type InboundFSStat struct {
	ID   string  `json:"id"`
	Path string  `json:"path"`
	Cwd  *string `json:"cwd,omitzero"`
}

func (InboundFSStat) inbound() {}

//demi:variant fs_lstat
type InboundFSLstat struct {
	ID   string  `json:"id"`
	Path string  `json:"path"`
	Cwd  *string `json:"cwd,omitzero"`
}

func (InboundFSLstat) inbound() {}

//demi:variant fs_readdir
type InboundFSReaddir struct {
	ID   string  `json:"id"`
	Path string  `json:"path"`
	Cwd  *string `json:"cwd,omitzero"`
}

func (InboundFSReaddir) inbound() {}

//demi:variant fs_mkdir
type InboundFSMkdir struct {
	ID        string  `json:"id"`
	Path      string  `json:"path"`
	Cwd       *string `json:"cwd,omitzero"`
	Recursive *bool   `json:"recursive,omitzero"`
}

func (InboundFSMkdir) inbound() {}

//demi:variant fs_rm
type InboundFSRm struct {
	ID        string  `json:"id"`
	Path      string  `json:"path"`
	Cwd       *string `json:"cwd,omitzero"`
	Recursive *bool   `json:"recursive,omitzero"`
	Force     *bool   `json:"force,omitzero"`
}

func (InboundFSRm) inbound() {}

//demi:variant fs_cp
type InboundFSCp struct {
	ID          string  `json:"id"`
	Path        string  `json:"path"`
	Destination string  `json:"destination"`
	Cwd         *string `json:"cwd,omitzero"`
	Recursive   *bool   `json:"recursive,omitzero"`
}

func (InboundFSCp) inbound() {}

//demi:variant fs_mv
type InboundFSMv struct {
	ID          string  `json:"id"`
	Path        string  `json:"path"`
	Destination string  `json:"destination"`
	Cwd         *string `json:"cwd,omitzero"`
}

func (InboundFSMv) inbound() {}

//demi:variant fs_chmod
type InboundFSChmod struct {
	ID   string  `json:"id"`
	Path string  `json:"path"`
	Mode uint32  `json:"mode"`
	Cwd  *string `json:"cwd,omitzero"`
}

func (InboundFSChmod) inbound() {}

//demi:variant fs_symlink
type InboundFSSymlink struct {
	ID     string  `json:"id"`
	Target string  `json:"target"`
	Path   string  `json:"path"`
	Cwd    *string `json:"cwd,omitzero"`
}

func (InboundFSSymlink) inbound() {}

//demi:variant fs_link
type InboundFSLink struct {
	ID           string  `json:"id"`
	ExistingPath string  `json:"existingPath"`
	Path         string  `json:"path"`
	Cwd          *string `json:"cwd,omitzero"`
}

func (InboundFSLink) inbound() {}

//demi:variant fs_readlink
type InboundFSReadlink struct {
	ID   string  `json:"id"`
	Path string  `json:"path"`
	Cwd  *string `json:"cwd,omitzero"`
}

func (InboundFSReadlink) inbound() {}

//demi:variant fs_realpath
type InboundFSRealpath struct {
	ID   string  `json:"id"`
	Path string  `json:"path"`
	Cwd  *string `json:"cwd,omitzero"`
}

func (InboundFSRealpath) inbound() {}

//demi:variant fs_utimes
type InboundFSUtimes struct {
	ID    string  `json:"id"`
	Path  string  `json:"path"`
	Atime int64   `json:"atime" msgpack:"timestamp"`
	Mtime int64   `json:"mtime" msgpack:"timestamp"`
	Cwd   *string `json:"cwd,omitzero"`
}

func (InboundFSUtimes) inbound() {}

//demi:variant git_changes
type InboundGitChanges struct {
	ID   string `json:"id"`
	Root string `json:"root"`
}

func (InboundGitChanges) inbound() {}

//demi:variant git_show
type InboundGitShow struct {
	ID     string  `json:"id"`
	Root   string  `json:"root"`
	Path   string  `json:"path"`
	Output PipeRef `json:"output"`
}

func (InboundGitShow) inbound() {}

//demi:union tag=type
//demi:msgpack
type Outbound interface{ outbound() }

//demi:variant conversation_released
type OutboundConversationReleased struct {
	ID    string  `json:"id"`
	Error *string `json:"error,omitzero"`
}

func (OutboundConversationReleased) outbound() {}

//demi:variant numbers_reserve
type OutboundNumbersReserve struct {
	ID             string                  `json:"id"`
	ConversationID string                  `json:"conversationId" check:"chars=1..commandservice.ConversationNameChars,pattern=conversationPattern"`
	Sequence       commandservice.Sequence `json:"sequence" check:"func=commandservice.Validate"`
	Count          uint32                  `json:"count" check:"range=1..commandservice.MaxNumbers"`
}

func (OutboundNumbersReserve) outbound() {}

//demi:variant artifact_resolve
type OutboundArtifactResolve struct {
	ID     string        `json:"id"`
	Owner  ArtifactOwner `json:"owner"`
	SHA256 string        `json:"sha256" check:"func=releaseDigest"`
	Target string        `json:"target" check:"func=commandservice.ValidateTarget"`
}

func (OutboundArtifactResolve) outbound() {}

//demi:variant hello
type OutboundHello struct {
	Protocol    uint32       `json:"protocol"`
	DeviceToken *DeviceToken `json:"deviceToken,omitzero" check:"func=deviceToken"`
	Runner      RunnerInfo   `json:"runner"`
}

func (OutboundHello) outbound() {}

//demi:variant pong
type OutboundPong struct {
	Jobs uint64 `json:"jobs"`
}

func (OutboundPong) outbound() {}

//demi:variant sync_done
type OutboundSyncDone struct {
	ID    string  `json:"id"`
	Error *string `json:"error,omitzero"`
}

func (OutboundSyncDone) outbound() {}

//demi:variant volume_grow
type OutboundVolumeGrow struct {
	ID     string     `json:"id"`
	Volume VolumeName `json:"volume"`
	Bytes  uint64     `json:"bytes" check:"range=1.."`
}

func (OutboundVolumeGrow) outbound() {}

//demi:variant fs_error
type OutboundFSError struct {
	ID      string  `json:"id"`
	Code    *string `json:"code,omitzero"`
	Message string  `json:"message"`
}

func (OutboundFSError) outbound() {}

//demi:variant git_error
type OutboundGitError struct {
	ID      string `json:"id"`
	Code    string `json:"code"`
	Message string `json:"message"`
}

func (OutboundGitError) outbound() {}

//demi:variant spawn_output
type OutboundSpawnOutput struct {
	SpawnID string       `json:"spawnId"`
	Stream  OutputStream `json:"stream"`
	Bytes   []byte       `json:"bytes" msgpack:"bin"`
}

func (OutboundSpawnOutput) outbound() {}

//demi:variant spawn_exit
type OutboundSpawnExit struct {
	SpawnID    string      `json:"spawnId"`
	ExitCode   *int32      `json:"exitCode" check:"nullable"`
	Signal     *string     `json:"signal,omitzero"`
	SpawnError *SpawnError `json:"spawnError,omitzero"`
}

func (OutboundSpawnExit) outbound() {}

//demi:variant job_output
type OutboundJobOutput struct {
	JobID  string       `json:"jobId"`
	Stream OutputStream `json:"stream"`
	Offset uint64       `json:"offset"`
	Bytes  []byte       `json:"bytes" msgpack:"bin"`
}

func (OutboundJobOutput) outbound() {}

//demi:variant job_running_hint
type OutboundJobRunningHint struct {
	JobID        string  `json:"jobId"`
	InvocationID string  `json:"invocationId"`
	Hint         *string `json:"hint" check:"nullable"`
}

func (OutboundJobRunningHint) outbound() {}

//demi:variant job_exit
type OutboundJobExit struct {
	JobID          string          `json:"jobId"`
	ExitCode       *int32          `json:"exitCode" check:"nullable"`
	Signal         *string         `json:"signal,omitzero"`
	SpawnError     *SpawnError     `json:"spawnError,omitzero"`
	Cwd            *string         `json:"cwd,omitzero"`
	Output         *OutputLengths  `json:"output,omitzero"`
	Files          []JobFileChange `json:"files" check:"items=..commandservice.EditJobFiles"`
	FilesTruncated bool            `json:"filesTruncated"`
}

func (OutboundJobExit) outbound() {}

//demi:variant job_read
type OutboundJobRead struct {
	ID    string  `json:"id"`
	Error *string `json:"error,omitzero"`
}

func (OutboundJobRead) outbound() {}

//demi:variant rpc_call
type OutboundRPCCall struct {
	JobID  string            `json:"jobId"`
	CallID string            `json:"callId"`
	Root   string            `json:"root"`
	Path   []string          `json:"path"`
	Argv   []string          `json:"argv"`
	Args   jsontext.Value    `json:"args" check:"func=jsonObject"`
	JSON   bool              `json:"json"`
	Cwd    string            `json:"cwd"`
	Env    map[string]string `json:"env"`
	Stdin  bool              `json:"stdin"`
}

func (OutboundRPCCall) outbound() {}

//demi:variant rpc_stdin
type OutboundRPCStdin struct {
	CallID string `json:"callId"`
	Bytes  []byte `json:"bytes" msgpack:"bin"`
}

func (OutboundRPCStdin) outbound() {}

//demi:variant rpc_stdin_end
type OutboundRPCStdinEnd struct {
	CallID string `json:"callId"`
}

func (OutboundRPCStdinEnd) outbound() {}

//demi:variant rpc_cancel
type OutboundRPCCancel struct {
	CallID string `json:"callId"`
}

func (OutboundRPCCancel) outbound() {}

//demi:variant pipe_done
type OutboundPipeDone struct {
	PipeID string  `json:"pipeId"`
	Ok     bool    `json:"ok"`
	Error  *string `json:"error,omitzero"`
}

func (OutboundPipeDone) outbound() {}

//demi:variant net_opened
type OutboundNetOpened struct {
	StreamID string `json:"streamId"`
}

func (OutboundNetOpened) outbound() {}

//demi:variant net_error
type OutboundNetError struct {
	StreamID string       `json:"streamId"`
	Code     NetErrorCode `json:"code"`
	Message  string       `json:"message"`
}

func (OutboundNetError) outbound() {}

//demi:variant service_opened
type OutboundServiceOpened struct {
	StreamID string `json:"streamId"`
}

func (OutboundServiceOpened) outbound() {}

//demi:variant service_error
type OutboundServiceError struct {
	StreamID string           `json:"streamId"`
	Code     ServiceErrorCode `json:"code"`
	Message  string           `json:"message"`
}

func (OutboundServiceError) outbound() {}

//demi:variant service_done
type OutboundServiceDone struct {
	StreamID string `json:"streamId"`
	ExitCode uint8  `json:"exitCode"`
	Stderr   string `json:"stderr" check:"chars=..ServiceStderrChars"`
}

func (OutboundServiceDone) outbound() {}

//demi:variant log_lines
type OutboundLogLines struct {
	ID    string    `json:"id"`
	Lines []LogLine `json:"lines" check:"items=..LogReadLines"`
	Next  uint64    `json:"next"`
}

func (OutboundLogLines) outbound() {}

//demi:variant log_error
type OutboundLogError struct {
	ID      string `json:"id"`
	Message string `json:"message"`
}

func (OutboundLogError) outbound() {}

//demi:wire
type PipeRef struct {
	ID  string `json:"id"`
	URL string `json:"url"`
}

//demi:variant
type JobArtifactOwner struct {
	JobID        string `json:"jobId"`
	ManifestHash string `json:"manifestHash" check:"func=releaseDigest"`
}

//demi:variant
type StreamArtifactOwner struct {
	StreamID string `json:"streamId"`
}

//demi:wire
type RunnerInfo struct {
	Name         string         `json:"name"`
	Platform     RunnerPlatform `json:"platform"`
	Version      string         `json:"version"`
	NativeTarget *string        `json:"nativeTarget,omitzero" check:"func=commandservice.ValidateTarget"`
	Identity     HostIdentity   `json:"identity"`
	Managed      *bool          `json:"managed,omitzero"`
}

//demi:wire
type HostIdentity struct {
	UID      uint32 `json:"uid"`
	GID      uint32 `json:"gid"`
	Hostname string `json:"hostname"`
	HomeDir  string `json:"homeDir"`
}

//demi:wire
//demi:msgpack
type FileStat struct {
	IsFile            bool    `json:"isFile"`
	IsDirectory       bool    `json:"isDirectory"`
	IsSymbolicLink    bool    `json:"isSymbolicLink"`
	Mode              uint32  `json:"mode"`
	Size              uint64  `json:"size"`
	Mtime             int64   `json:"mtime" msgpack:"timestamp"`
	UID               *uint32 `json:"uid,omitzero"`
	GID               *uint32 `json:"gid,omitzero"`
	Ino               *uint64 `json:"ino,omitzero"`
	Dev               *uint64 `json:"dev,omitzero"`
	Nlink             *uint64 `json:"nlink,omitzero"`
	IsCharacterDevice *bool   `json:"isCharacterDevice,omitzero"`
	IsFIFO            *bool   `json:"isFIFO,omitzero"`
}

//demi:wire
//demi:msgpack
type DirEntry struct {
	Name           string `json:"name"`
	IsFile         bool   `json:"isFile"`
	IsDirectory    bool   `json:"isDirectory"`
	IsSymbolicLink bool   `json:"isSymbolicLink"`
}

//demi:wire
//demi:msgpack
type GitChanges struct {
	Repository bool        `json:"repository"`
	Head       *string     `json:"head" check:"nullable"`
	Files      []GitChange `json:"files"`
	Truncated  bool        `json:"truncated"`
	Watched    bool        `json:"watched"`
}

//demi:wire
type GitChange struct {
	Path    string     `json:"path"`
	Status  string     `json:"status" check:"pattern=gitStatusPattern"`
	Kind    ChangeKind `json:"kind"`
	From    *string    `json:"from,omitzero"`
	Added   uint64     `json:"added" check:"range=..9007199254740991"`
	Removed uint64     `json:"removed" check:"range=..9007199254740991"`
}

//demi:wire
type SpawnError struct {
	Kind   SpawnErrorKind `json:"kind"`
	Detail *string        `json:"detail,omitzero"`
}

//demi:wire
type OutputLengths struct {
	StdoutBytes uint64 `json:"stdoutBytes"`
	StderrBytes uint64 `json:"stderrBytes"`
}

//demi:wire
type JobFileChange struct {
	Path    string                      `json:"path" check:"nonul,chars=1.."`
	Kind    commandservice.EditKind     `json:"kind" check:"func=commandservice.Validate"`
	Edits   []commandservice.EditCopies `json:"edits" check:"each(func=commandservice.Validate),items=..commandservice.EditJobSegments"`
	Added   uint64                      `json:"added"`
	Removed uint64                      `json:"removed"`
}

//demi:wire
type LogLine struct {
	At             int64   `json:"at" msgpack:"timestamp"`
	Source         string  `json:"source" check:"chars=1.."`
	ConversationID *string `json:"conversationId,omitzero" check:"chars=1.."`
	Text           string  `json:"text"`
}

//demi:union untagged
type ArtifactOwner interface{ artifactOwner() }

func (JobArtifactOwner) artifactOwner() {}

func (StreamArtifactOwner) artifactOwner() {}

//demi:enum
type HelloErrorCode string

const (
	HelloErrorCodeUnsupportedProtocol HelloErrorCode = "unsupported_protocol"
	HelloErrorCodeUnknownDevice       HelloErrorCode = "unknown_device"
	HelloErrorCodeAlreadyConnected    HelloErrorCode = "already_connected"
	HelloErrorCodeRevoked             HelloErrorCode = "revoked"
	HelloErrorCodeInternal            HelloErrorCode = "internal"
)

//demi:enum
type VolumeName string

const (
	VolumeNameSystem VolumeName = "system"
	VolumeNameHome   VolumeName = "home"
)

//demi:enum
type NetErrorCode string

const (
	NetErrorCodeRefused       NetErrorCode = "refused"
	NetErrorCodeUnreachable   NetErrorCode = "unreachable"
	NetErrorCodeResolveFailed NetErrorCode = "resolve_failed"
	NetErrorCodeTimeout       NetErrorCode = "timeout"
)

//demi:enum
type ServiceErrorCode string

const (
	ServiceErrorCodeUnknownOperation ServiceErrorCode = "unknown_operation"
	ServiceErrorCodeServiceFailed    ServiceErrorCode = "service_failed"
	ServiceErrorCodeRefused          ServiceErrorCode = "refused"
)

//demi:enum
type ChangeKind string

const (
	ChangeKindAdded    ChangeKind = "added"
	ChangeKindModified ChangeKind = "modified"
	ChangeKindDeleted  ChangeKind = "deleted"
	ChangeKindRenamed  ChangeKind = "renamed"
)

//demi:enum
type SpawnErrorKind string

const (
	SpawnErrorKindExecutableNotFound SpawnErrorKind = "executable_not_found"
	SpawnErrorKindPermissionDenied   SpawnErrorKind = "permission_denied"
	SpawnErrorKindCwdUnusable        SpawnErrorKind = "cwd_unusable"
	SpawnErrorKindIsDirectory        SpawnErrorKind = "is_directory"
	SpawnErrorKindOther              SpawnErrorKind = "other"
)
