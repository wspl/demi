package runner

//go:generate go run ../..

// +demi:union tag=type
// +demi:msgpack
// +demi:root
type Message interface{ isMessage() }

// +demi:timestamp
// +demi:msgpack
type Timestamp string

// +demi:msgpack
// +demi:root
type Environment map[string]*string

// +demi:msgpack
// +demi:variant Message ping
type Ping struct {
}

// +demi:msgpack
// +demi:variant Message hello_ok
type HelloOK struct {
	DeviceID string `json:"deviceId"`
}

// +demi:msgpack
// +demi:variant Message claim_pending
type ClaimPending struct {
	ClaimToken string `json:"claimToken"`
}

// +demi:msgpack
// +demi:variant Message hello_error
type HelloError struct {
	// +demi:enum unsupported_protocol unknown_device already_connected revoked internal
	Code   string `json:"code"`
	Reason string `json:"reason"`
}

// +demi:msgpack
// +demi:variant Message sync
type Sync struct {
	ID string `json:"id"`
}

// +demi:msgpack
// +demi:variant Message volume_grown
type VolumeGrown struct {
	ID string `json:"id"`
	// +demi:enum system home
	Volume string `json:"volume"`
	// +demi:range min=1
	Bytes uint64 `json:"bytes"`
	// +demi:nullable
	Error *string `json:"error"`
}

// +demi:msgpack
// +demi:variant Message spawn
type Spawn struct {
	SpawnID          string       `json:"spawnId"`
	Command          string       `json:"command"`
	Args             *[]string    `json:"args,omitempty"`
	Cwd              *string      `json:"cwd,omitempty"`
	Env              *Environment `json:"env,omitempty"`
	InheritEnv       *bool        `json:"inheritEnv,omitempty"`
	KillProcessGroup *bool        `json:"killProcessGroup,omitempty"`
}

// +demi:msgpack
// +demi:variant Message spawn_stdin
type SpawnStdin struct {
	SpawnID string `json:"spawnId"`
	Bytes   []byte `json:"bytes"`
}

// +demi:msgpack
// +demi:variant Message spawn_kill
type SpawnKill struct {
	SpawnID string `json:"spawnId"`
	// +demi:enum SIGTERM SIGKILL SIGINT SIGHUP SIGQUIT SIGUSR1 SIGUSR2 SIGSTOP SIGCONT
	Signal *string `json:"signal,omitempty"`
}

// +demi:msgpack
// +demi:variant Message job_follow
type JobFollow struct {
	JobID  string `json:"jobId"`
	Follow bool   `json:"follow"`
}

// +demi:msgpack
type PipeRef struct {
	ID  string `json:"id"`
	URL string `json:"url"`
}

// +demi:msgpack
// +demi:variant Message rpc_pipes
type RPCPipes struct {
	CallID string   `json:"callId"`
	Stdin  *PipeRef `json:"stdin,omitempty"`
	Stdout PipeRef  `json:"stdout"`
}

// +demi:msgpack
// +demi:variant Message rpc_exit
type RPCExit struct {
	CallID   string `json:"callId"`
	ExitCode uint8  `json:"exitCode"`
}

// +demi:msgpack
// +demi:variant Message fs_utimes
type FSUtimes struct {
	ID    string    `json:"id"`
	Path  string    `json:"path"`
	Atime Timestamp `json:"atime"`
	Mtime Timestamp `json:"mtime"`
	Cwd   *string   `json:"cwd,omitempty"`
}

// +demi:msgpack
// +demi:variant Message pong
type Pong struct {
	Jobs uint64 `json:"jobs"`
}

// +demi:msgpack
// +demi:variant Message sync_done
type SyncDone struct {
	ID    string  `json:"id"`
	Error *string `json:"error,omitempty"`
}

// +demi:msgpack
// +demi:variant Message job_output
type JobOutput struct {
	JobID string `json:"jobId"`
	// +demi:enum stdout stderr
	Stream string `json:"stream"`
	Offset uint64 `json:"offset"`
	Bytes  []byte `json:"bytes"`
}

// +demi:msgpack
// +demi:variant Message job_running_hint
type JobRunningHint struct {
	JobID        string `json:"jobId"`
	InvocationID string `json:"invocationId"`
	// +demi:nullable
	Hint *string `json:"hint"`
}

// +demi:msgpack
// +demi:variant Message volume_grow
type VolumeGrow struct {
	ID string `json:"id"`
	// +demi:enum system home
	Volume string `json:"volume"`
	// +demi:range min=1
	Bytes uint64 `json:"bytes"`
}

// +demi:msgpack
// +demi:variant Message spawn_output
type SpawnOutput struct {
	SpawnID string `json:"spawnId"`
	// +demi:enum stdout stderr
	Stream string `json:"stream"`
	Bytes  []byte `json:"bytes"`
}
