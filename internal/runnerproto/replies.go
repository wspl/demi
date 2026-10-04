package runnerproto

// A successful fs call's reply.
// +demi:variant Outbound fs_ok
type FSOK struct {
	ID string `json:"id"`
	// +demi:flatten
	Result FSResult
}

// The result of each fs operation; operations that only succeed or fail
// carry none.
// +demi:union tag=op content=result
//
//sumtype:decl
type FSResult interface {
	fsResult()
	Op() string
}

// +demi:variant FSResult readFile
type FSReadFileResult struct{}

// Op returns the operation's name on the wire.
func (*FSReadFileResult) Op() string { return "readFile" }

// +demi:variant FSResult writeFile
type FSWriteFileResult struct{}

// Op returns the operation's name on the wire.
func (*FSWriteFileResult) Op() string { return "writeFile" }

// +demi:variant FSResult exists
type FSExistsResult struct {
	Value bool `json:"value"`
}

// Op returns the operation's name on the wire.
func (*FSExistsResult) Op() string { return "exists" }

// +demi:variant FSResult stat
type FSStatResult struct {
	Value FileStat `json:"value"`
}

// Op returns the operation's name on the wire.
func (*FSStatResult) Op() string { return "stat" }

// +demi:variant FSResult lstat
type FSLstatResult struct {
	Value FileStat `json:"value"`
}

// Op returns the operation's name on the wire.
func (*FSLstatResult) Op() string { return "lstat" }

// +demi:variant FSResult readdir
type FSReaddirResult struct {
	Value []DirEntry `json:"value"`
}

// Op returns the operation's name on the wire.
func (*FSReaddirResult) Op() string { return "readdir" }

// +demi:variant FSResult mkdir
type FSMkdirResult struct{}

// Op returns the operation's name on the wire.
func (*FSMkdirResult) Op() string { return "mkdir" }

// +demi:variant FSResult rm
type FSRmResult struct{}

// Op returns the operation's name on the wire.
func (*FSRmResult) Op() string { return "rm" }

// +demi:variant FSResult cp
type FSCpResult struct{}

// Op returns the operation's name on the wire.
func (*FSCpResult) Op() string { return "cp" }

// +demi:variant FSResult mv
type FSMvResult struct{}

// Op returns the operation's name on the wire.
func (*FSMvResult) Op() string { return "mv" }

// +demi:variant FSResult chmod
type FSChmodResult struct{}

// Op returns the operation's name on the wire.
func (*FSChmodResult) Op() string { return "chmod" }

// +demi:variant FSResult symlink
type FSSymlinkResult struct{}

// Op returns the operation's name on the wire.
func (*FSSymlinkResult) Op() string { return "symlink" }

// +demi:variant FSResult link
type FSLinkResult struct{}

// Op returns the operation's name on the wire.
func (*FSLinkResult) Op() string { return "link" }

// +demi:variant FSResult readlink
type FSReadlinkResult struct {
	Value string `json:"value"`
}

// Op returns the operation's name on the wire.
func (*FSReadlinkResult) Op() string { return "readlink" }

// +demi:variant FSResult realpath
type FSRealpathResult struct {
	Value string `json:"value"`
}

// Op returns the operation's name on the wire.
func (*FSRealpathResult) Op() string { return "realpath" }

// +demi:variant FSResult utimes
type FSUtimesResult struct{}

// Op returns the operation's name on the wire.
func (*FSUtimesResult) Op() string { return "utimes" }

// A successful working-tree call's reply.
// +demi:variant Outbound git_ok
type GitOK struct {
	ID string `json:"id"`
	// +demi:flatten
	Result GitResult
}

// +demi:union tag=op content=result
//
//sumtype:decl
type GitResult interface {
	gitResult()
	Op() string
}

// +demi:variant GitResult changes
type GitChangesResult struct {
	Value GitChanges `json:"value"`
}

// Op returns the operation's name on the wire.
func (*GitChangesResult) Op() string { return "changes" }

// The file streams into the call's output pipe after the reply.
// +demi:variant GitResult show
type GitShowResult struct{}

// Op returns the operation's name on the wire.
func (*GitShowResult) Op() string { return "show" }
