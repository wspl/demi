package commandservice

// The limits of a job's edit record that its journal states.
const (
	// EditJobBytes is the most bytes the record copies in total.
	EditJobBytes = 64 * 1024 * 1024
	// EditJobFiles is the most files the record lists.
	EditJobFiles = 500
	// EditJobSegments is the most edit segments the record holds.
	EditJobSegments = 1000
)

// An EditContext says where an invoked command records its edits: the job's
// edit directory and the lock that serializes writers to it. Both paths are
// absolute.
//
//demi:wire
type EditContext struct {
	Directory string `json:"directory" check:"chars=1..,nonul,func=absolutePath"`
	Lock      string `json:"lock" check:"chars=1..,nonul,func=absolutePath"`
}

// EditCopies holds the copies of one edit segment: the file before and after
// it.
//
//demi:wire
type EditCopies struct {
	Original *string `json:"original,omitzero" check:"chars=1..,nonul"`
	Modified *string `json:"modified,omitzero" check:"chars=1..,nonul"`
}

// An EditKind says whether an edited file existed before the job.
type EditKind string

// The kinds of edited file.
const (
	EditAdded    EditKind = "added"
	EditModified EditKind = "modified"
)

// An EditFile is one edited file and its segments.
//
//demi:wire
type EditFile struct {
	Path  string       `json:"path" check:"chars=1..,nonul"`
	Kind  EditKind     `json:"kind" check:"oneof=added|modified"`
	Edits []EditCopies `json:"edits" check:"items=..EditJobSegments"`
}

// An EditJournal is a job's edit record as the command left it.
//
//demi:wire
type EditJournal struct {
	Files          []EditFile `json:"files" check:"items=..EditJobFiles"`
	BytesCopied    uint64     `json:"bytesCopied" check:"range=..EditJobBytes"`
	NextSegment    uint64     `json:"nextSegment" check:"range=..EditJobSegments"`
	FilesTruncated bool       `json:"filesTruncated"`
}
