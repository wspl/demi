package remaining

import "encoding/json"

//go:generate go run ../.. .

// +demi:root
// +demi:msgpack
// +demi:union tag=type
type Message interface{ message() }

// A time in milliseconds since the Unix epoch, the precision of the Host
// contract's JavaScript `Date` values. It travels as the MessagePack
// timestamp extension (type -1) in its shortest form.
// +demi:schema
// +demi:timestamp
type Timestamp int64

// +demi:variant Message manifest
type Manifest struct {
	Manifest json.RawMessage `json:"manifest"`
}

// +demi:variant Message fs_utimes
type UTimes struct {
	ID    string    `json:"id"`
	Path  string    `json:"path"`
	ATime Timestamp `json:"atime"`
	MTime Timestamp `json:"mtime"`
	CWD   *string   `json:"cwd,omitempty"`
}

// A successful fs call's reply.
// +demi:schema
// +demi:variant Message fs_ok
type FsOK struct {
	ID string `json:"id"`
	// +demi:flatten
	Result FsResult
}

// The result of each fs operation; operations that only succeed or fail
// carry none.
// +demi:union tag=op content=result
type FsResult interface{ fsResult() }

// A successful working-tree call's reply.
// +demi:variant Message git_ok
type GitOK struct {
	ID string `json:"id"`
	// +demi:flatten
	Result GitResult
}

// +demi:union tag=op content=result
type GitResult interface{ gitResult() }

// +demi:variant GitResult changes
type Changes struct {
	Value GitChanges `json:"value"`
}

// The file streams into the call's output pipe after the reply.
// +demi:variant GitResult show
type Show struct{}

// A file's metadata, as `stat` or `lstat` reports it.
type FileStat struct {
	IsFile            bool      `json:"isFile"`
	IsDirectory       bool      `json:"isDirectory"`
	IsSymbolicLink    bool      `json:"isSymbolicLink"`
	Mode              uint32    `json:"mode"`
	Size              uint64    `json:"size"`
	MTime             Timestamp `json:"mtime"`
	UID               *uint32   `json:"uid,omitempty"`
	GID               *uint32   `json:"gid,omitempty"`
	Ino               *uint64   `json:"ino,omitempty"`
	Dev               *uint64   `json:"dev,omitempty"`
	NLink             *uint64   `json:"nlink,omitempty"`
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
	Status string `json:"status"`
	Kind   string `json:"kind"`
	// The path before a rename.
	From    *string `json:"from,omitempty"`
	Added   uint64  `json:"added"`
	Removed uint64  `json:"removed"`
}

// One line of the Host's log: when it was written, which source wrote it,
// the conversation the work belonged to when there was one, and the text.
type LogLine struct {
	At             Timestamp `json:"at"`
	Source         string    `json:"source"`
	ConversationID *string   `json:"conversationId,omitempty"`
	Text           string    `json:"text"`
}

// +demi:variant Message log_lines
type LogLines struct {
	ID    string    `json:"id"`
	Lines []LogLine `json:"lines"`
	Next  uint64    `json:"next"`
}

// +demi:variant Message rpc_call
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

// Open a user stream: invoke `operation` of `package` in its resident
// service.
// +demi:variant Message service_open
type ServiceOpen struct {
	StreamID  string           `json:"streamId"`
	Context   json.RawMessage  `json:"context"`
	Package   json.RawMessage  `json:"package"`
	Operation string           `json:"operation"`
	Args      *json.RawMessage `json:"args,omitempty"`
	JSON      *bool            `json:"json,omitempty"`
	CWD       string           `json:"cwd"`
	Input     PipeRef          `json:"input"`
	Output    PipeRef          `json:"output"`
}

type PipeRef struct {
	ID  string `json:"id"`
	URL string `json:"url"`
}

// +demi:variant FsResult readFile
type ReadFile struct{}

// +demi:variant FsResult writeFile
type WriteFile struct{}

// +demi:variant FsResult exists
type Exists struct {
	Value bool `json:"value"`
}

// +demi:variant FsResult stat
type Stat struct {
	Value FileStat `json:"value"`
}

// +demi:variant FsResult lstat
type Lstat struct {
	Value FileStat `json:"value"`
}

// +demi:variant FsResult readdir
type Readdir struct {
	Value []DirEntry `json:"value"`
}

// +demi:variant FsResult mkdir
type Mkdir struct{}

// +demi:variant FsResult rm
type Rm struct{}

// +demi:variant FsResult cp
type Cp struct{}

// +demi:variant FsResult mv
type Mv struct{}

// +demi:variant FsResult chmod
type Chmod struct{}

// +demi:variant FsResult symlink
type Symlink struct{}

// +demi:variant FsResult link
type Link struct{}

// +demi:variant FsResult readlink
type Readlink struct {
	Value string `json:"value"`
}

// +demi:variant FsResult realpath
type Realpath struct {
	Value string `json:"value"`
}

// +demi:variant FsResult utimes
type Utimes struct{}
