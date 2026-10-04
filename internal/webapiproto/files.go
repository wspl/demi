package webapiproto

import (
	"github.com/wspl/demi/internal/runnerproto"
	"github.com/wspl/demi/internal/types"
)

// `{ path, home, entries }`: a directory as a Host lists it. `home` is the
// Host's home directory, null while it is unknown.
// +demi:root direction=receive output=web
// +demi:tolerant
type Directory struct {
	Path string `json:"path"`
	// +demi:nullable
	Home    *string          `json:"home"`
	Entries []DirectoryEntry `json:"entries"`
}

// One entry of a listing. An entry that disappears while the directory is
// listed is left out.
// +demi:tolerant
type DirectoryEntry struct {
	Name           string `json:"name"`
	IsDirectory    bool   `json:"isDirectory"`
	IsSymbolicLink bool   `json:"isSymbolicLink"`
	// +demi:range max=9007199254740991
	Size       uint64          `json:"size"`
	ModifiedAt types.Timestamp `json:"modifiedAt"`
}

// `POST .../fs { path }`: a directory to make, with its parents.
// +demi:root direction=send output=web
type CreateDirectory struct {
	// +demi:length chars min=1
	Path string `json:"path"`
}

// `POST /devices/:id/fs { path }`: a directory to make on a device, named
// the way the device names it.
// +demi:root direction=send output=web
type CreateDeviceDirectory struct {
	Path AbsolutePath `json:"path"`
}

// `{ path }`: the directory a create made.
// +demi:root direction=receive output=web
// +demi:tolerant
type CreatedDirectory struct {
	Path string `json:"path"`
}

// `GET .../fs/file`: a file's text.
// +demi:root direction=receive output=web
// +demi:tolerant
type FileText struct {
	Path string `json:"path"`
	Text string `json:"text"`
}

// `GET .../changes`: the runner's list of uncommitted changes, and the
// directory its paths are relative to.
// +demi:root direction=receive output=web
// +demi:tolerant
type WorkingTreeChanges struct {
	Root string `json:"root"`
	runnerproto.GitChanges
}

// One changed file's two sides: `original` as the last commit or the edit
// has it, empty for a new file; `modified` as the working tree or the edit
// has it, empty for a deleted one.
// +demi:root direction=receive output=web
// +demi:tolerant
type ChangeSides struct {
	Original string `json:"original"`
	Modified string `json:"modified"`
}

// `?path=` naming a directory to list: the Host's starting directory when
// omitted. Queries are the backend's alone and are not emitted.
// +demi:root
// +demi:tolerant
type DirectoryQuery struct {
	// +demi:nullable
	Path *NonEmptyPath `json:"path,omitempty"`
}

// `?path=` of a device listing: absolute on the device, its home when
// omitted.
// +demi:root
// +demi:tolerant
type DeviceDirectoryQuery struct {
	// +demi:nullable
	Path *AbsolutePath `json:"path,omitempty"`
}

// `?path=` naming one file.
// +demi:root
// +demi:tolerant
type FileQuery struct {
	Path NonEmptyPath `json:"path"`
}

// `?path=` of a delete: absolute, so what it would take with it can be
// told.
// +demi:root
// +demi:tolerant
type RemoveQuery struct {
	Path AbsolutePath `json:"path"`
}

// `?path=&version=&download=` of the raw file route.
// +demi:root
// +demi:tolerant
type RawFileQuery struct {
	Path NonEmptyPath `json:"path"`
	// The ETag the request expects the file to still have.
	// +demi:nullable
	Version  *NonEmptyPath `json:"version,omitempty"`
	Download StrictBool    `json:"download,omitempty"`
}

// `?path=&replace=` of an upload.
// +demi:root
// +demi:tolerant
type FileUploadQuery struct {
	Path    NonEmptyPath `json:"path"`
	Replace StrictBool   `json:"replace,omitempty"`
}

// `?path=` of a working tree file.
// +demi:root
// +demi:tolerant
type TreeFileQuery struct {
	Path TreePath `json:"path"`
}

// `?path=&download=` of the committed side of a working tree file.
// +demi:root
// +demi:tolerant
type CommittedFileQuery struct {
	Path     TreePath   `json:"path"`
	Download StrictBool `json:"download,omitempty"`
}

// A path on a Host that names its root: `/…` on Unix, `C:\…` on Windows.
// The Host decides what it names, so the meaning never depends on where a
// runner stands.
// +demi:root
// +demi:id
// +demi:check validateAbsolutePath
type AbsolutePath string

// A path under a working tree's root: nonempty, not absolute, and without
// a `..` segment, so it stays inside the root.
// +demi:root
// +demi:id
// +demi:check validateTreePath
type TreePath string

// A nonempty query text, such as a path or an ETag.
// +demi:root
// +demi:id
// +demi:length chars min=1
type NonEmptyPath string
