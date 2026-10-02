package commandwire

// ArtifactForm describes how downloaded bytes are installed.
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

// ArtifactInstall requests installation for an invocation.
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

// ArtifactsInstalled asks for the installed artifacts of a release line.
type ArtifactsInstalled struct {
	// +demi:length chars min=1 max=100
	Name string `json:"name"`
}

// ArtifactRequest carries exactly one installation or inventory request.
// +demi:check validateArtifactRequest
type ArtifactRequest struct {
	ID        uint64              `json:"id"`
	Install   *ArtifactInstall    `json:"install,omitempty"`
	Installed *ArtifactsInstalled `json:"installed,omitempty"`
}

// InstalledArtifact identifies installed bytes and their entry path.
type InstalledArtifact struct {
	// +demi:length chars min=1
	Version string `json:"version"`
	// +demi:pattern ^[0-9a-f]{64}$
	SHA256 string `json:"sha256"`
	// +demi:length chars min=1
	Path string `json:"path"`
}

// ArtifactAnswer carries exactly one path, inventory, or failure.
// +demi:check validateArtifactAnswer
type ArtifactAnswer struct {
	ID uint64 `json:"id"`
	// +demi:length chars min=1
	Path      *string              `json:"path,omitempty"`
	Installed *[]InstalledArtifact `json:"installed,omitempty"`
	// +demi:length chars min=1
	Error *string `json:"error,omitempty"`
}
