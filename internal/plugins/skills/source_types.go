package skills

import (
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/plugin"
)

// A source as its value keeps it.
// +demi:root
type source struct {
	// The repository, as the user wrote it.
	Origin string `json:"origin"`
	// Its place in the order the user added sources.
	Added uint64 `json:"added"`
	// +demi:nullable
	Commit *string `json:"commit,omitempty"`
	// +demi:nullable
	FetchedAt *core.Timestamp `json:"fetchedAt,omitempty"`
	Skills    []userSkill     `json:"skills"`
	Skipped   []Skipped       `json:"skipped"`
	// +demi:nullable
	Failure *Failure `json:"failure,omitempty"`
}

// A skill of a source's pinned commit.
// +demi:root
type userSkill struct {
	Name        string `json:"name"`
	Description string `json:"description"`
	// Its directory in the repository, empty at the root.
	Directory              string                 `json:"directory"`
	Files                  []plugin.DirectoryFile `json:"files"`
	Warnings               []string               `json:"warnings"`
	DisableModelInvocation bool                   `json:"disableModelInvocation"`
	Enabled                bool                   `json:"enabled"`
}
