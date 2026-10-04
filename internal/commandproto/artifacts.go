package commandproto

// How an artifact is installed: one executable file, or a zip archive
// unpacked, whose entry is the file its user starts.
// +demi:union tag=kind
//
//sumtype:decl
type ArtifactForm interface{ artifactForm() }

// ArtifactFile installs one executable file.
// +demi:variant ArtifactForm file
type ArtifactFile struct{}

func (*ArtifactFile) artifactForm() {}

// ArtifactArchive unpacks an archive and selects its entry.
// +demi:variant ArtifactForm archive
// +demi:check validateArtifactArchive
type ArtifactArchive struct {
	Entry string `json:"entry"`
}

func (*ArtifactArchive) artifactForm() {}

// An artifact to install for `invocation`: its line's name and its version
// for the user, its bytes' SHA-256 and size, and its form.
type ArtifactInstall struct {
	// +demi:length chars min=1 max=200
	Invocation string `json:"invocation"`
	// +demi:length chars min=1 max=100
	Name string `json:"name"`
	// +demi:length chars min=1 max=100
	Version string `json:"version"`
	// +demi:pattern ^[0-9a-f]{64}$
	SHA256 string `json:"sha256"`
	// +demi:range min=1 max=9007199254740991
	Size uint64       `json:"size"`
	Form ArtifactForm `json:"form"`
}

// Which artifacts of the line `name` the Host has.
type ArtifactsInstalled struct {
	// +demi:length chars min=1 max=100
	Name string `json:"name"`
}

// One request a service writes as a standard output record: `id`, its own,
// unique among its requests in flight, and either an install or a question.
// +demi:check validateArtifactRequest
// +demi:root
type ArtifactRequest struct {
	ID        uint64              `json:"id"`
	Install   *ArtifactInstall    `json:"install,omitempty"`
	Installed *ArtifactsInstalled `json:"installed,omitempty"`
}

// One artifact of a line the Host has: its version and SHA-256, and the
// absolute path of the file or of the archive's entry.
type InstalledArtifact struct {
	// +demi:length chars min=1
	Version string `json:"version"`
	// +demi:pattern ^[0-9a-f]{64}$
	SHA256 string `json:"sha256"`
	// +demi:length chars min=1
	Path string `json:"path"`
}

// The answer to request `id`, one input chunk: a path, the line's
// artifacts, or why there is neither.
// +demi:check validateArtifactAnswer
// +demi:root
type ArtifactAnswer struct {
	ID uint64 `json:"id"`
	// +demi:length chars min=1
	Path      *string              `json:"path,omitempty"`
	Installed *[]InstalledArtifact `json:"installed,omitempty"`
	// +demi:length chars min=1
	Error *string `json:"error,omitempty"`
}
