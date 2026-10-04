package machinewire

import (
	"github.com/wspl/demi/internal/commandwire"
	"github.com/wspl/demi/internal/runnerwire"
)

// The manifest's schema version.
// +demi:root
// +demi:range min=1 max=1
type FormatVersion uint32

// +demi:root
// +demi:enum linux
type OS string

// A Cloud image's CPU architecture. Persisted state never moves between
// architectures.
// +demi:root
// +demi:enum amd64 arm64
type Architecture string

// The name of the base archive in the release directory.
// +demi:root
// +demi:enum rootfs.tar.zst
type RootfsFile string

// The base archive: its SHA-256, its byte size and its file.
type RootfsArchive struct {
	// +demi:pattern ^[0-9a-f]{64}$
	SHA256 string `json:"sha256"`
	// +demi:range min=1
	Size uint64     `json:"size"`
	File RootfsFile `json:"file"`
}

// A system package the image's dpkg database lists.
type InstalledPackage struct {
	// +demi:length chars min=1
	Name string `json:"name"`
	// +demi:length chars min=1
	Version string `json:"version"`
}

// A standalone tool the build installed, such as uv or Chrome.
type StandaloneTool struct {
	// +demi:length chars min=1
	Name string `json:"name"`
	// +demi:length chars min=1
	Version string `json:"version"`
	// +demi:pattern ^[0-9a-f]{64}$
	SHA256 string `json:"sha256"`
}

// A Cloud image release's manifest.
// +demi:root
// +demi:check validateManifest
type CloudImageManifest struct {
	FormatVersion FormatVersion `json:"formatVersion"`
	OS            OS            `json:"os"`
	Architecture  Architecture  `json:"architecture"`
	Rootfs        RootfsArchive `json:"rootfs"`
	// The Ubuntu release of the base.
	// +demi:length chars min=1
	Ubuntu   string             `json:"ubuntu"`
	Packages []InstalledPackage `json:"packages"`
	// The executables the image embeds, by absolute path under `/usr` or
	// `/opt`, with their size and SHA-256.
	Executables map[string]commandwire.PackageArtifact `json:"executables"`
	// The command package releases whose artifacts the image embeds.
	Releases []commandwire.PackageDescriptor `json:"releases"`
	// The runner release of `/usr/bin/demi-runner`.
	Runner runnerwire.Release `json:"runner"`
	Tools  []StandaloneTool   `json:"tools"`
}
