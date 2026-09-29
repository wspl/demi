package webapi

import (
	"github.com/wspl/demi/go/core"
	"github.com/wspl/demi/go/runnerproto"
)

// `{ path, home, entries }`: a directory as a Host lists it. `home` is the
// Host's home directory, null while it is unknown.
//
//demi:wire open
type Directory struct {
	Path    string           `json:"path"`
	Home    *string          `json:"home" check:"nullable"`
	Entries []DirectoryEntry `json:"entries"`
}

// One entry of a listing. An entry that disappears while the directory is
// listed is left out.
//
//demi:wire open
type DirectoryEntry struct {
	Name           string         `json:"name"`
	IsDirectory    bool           `json:"isDirectory"`
	IsSymbolicLink bool           `json:"isSymbolicLink"`
	Size           uint64         `json:"size" check:"range=..9007199254740991"`
	ModifiedAt     core.Timestamp `json:"modifiedAt" check:"func=core.Validate"`
}

// `POST .../fs { path }`: a directory to make, with its parents.
//
//demi:wire
type CreateDirectory struct {
	Path string `json:"path" check:"chars=1.."`
}

// `POST /devices/:id/fs { path }`: a directory to make on a device, named
// the way the device names it.
//
//demi:wire
type CreateDeviceDirectory struct {
	Path AbsolutePath `json:"path" check:"func=Validate"`
}

// `{ path }`: the directory a create made.
//
//demi:wire open
type CreatedDirectory struct {
	Path string `json:"path"`
}

// `GET .../fs/file`: a file's text.
//
//demi:wire open
type FileText struct {
	Path string `json:"path"`
	Text string `json:"text"`
}

// `GET .../changes`: the runner's list of uncommitted changes, and the
// directory its paths are relative to.
//
//demi:wire open
type WorkingTreeChanges struct {
	Root string `json:"root"`
	runnerproto.GitChanges
}

// One changed file's two sides: `original` as the last commit or the edit
// has it, empty for a new file; `modified` as the working tree or the edit
// has it, empty for a deleted one.
//
//demi:wire open
type ChangeSides struct {
	Original string `json:"original"`
	Modified string `json:"modified"`
}
