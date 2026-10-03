//revive:disable:exported
// Contract names are the schema titles and the TypeScript export names the page
// imports; contract doc comments are the schemas' descriptions.

package skills

import "github.com/wspl/demi/internal/core"

//go:generate go run github.com/wspl/demi/tools/contractgen

// `add_source { origin }`.
// +demi:root direction=send output=plugin-skills
// +demi:schema
type AddSource struct {
	Origin string `json:"origin"`
}

// What `add_source` answers: the new source's id.
// +demi:root direction=receive output=plugin-skills
// +demi:schema
// +demi:tolerant
type AddedSource struct {
	Source string `json:"source"`
}

// `update_source { source }` and `remove_source { source }`.
// +demi:root direction=send output=plugin-skills
// +demi:schema
type SourceCall struct {
	Source string `json:"source"`
}

// `set_enabled { source, skill, enabled }`.
// +demi:root direction=send output=plugin-skills
// +demi:schema
type SetEnabled struct {
	Source  string `json:"source"`
	Skill   string `json:"skill"`
	Enabled bool   `json:"enabled"`
}

// `set_source_enabled { source, enabled }`.
// +demi:root direction=send output=plugin-skills
// +demi:schema
type SetSourceEnabled struct {
	Source  string `json:"source"`
	Enabled bool   `json:"enabled"`
}

// A `SKILL.md` that is not a skill.
// +demi:root direction=receive output=plugin-skills
// +demi:tolerant
type Skipped struct {
	// Its path in the repository.
	Path   string `json:"path"`
	Reason string `json:"reason"`
}

// The last fetch's failure.
// +demi:root direction=receive output=plugin-skills
// +demi:tolerant
type Failure struct {
	At      core.Timestamp `json:"at"`
	Message string         `json:"message"`
}

// The plugin's state for the user's pages.
// +demi:root direction=receive output=plugin-skills
// +demi:schema
// +demi:tolerant
type SkillsState struct {
	// Every source, in the order the user added them.
	Sources []SourceState `json:"sources"`
}

// A source as the page shows it.
// +demi:root direction=receive output=plugin-skills
// +demi:tolerant
type SourceState struct {
	ID     string `json:"id"`
	Origin string `json:"origin"`
	// +demi:nullable
	Commit *string `json:"commit,omitempty"`
	// +demi:nullable
	FetchedAt *core.Timestamp `json:"fetchedAt,omitempty"`
	Fetching  bool            `json:"fetching"`
	// +demi:nullable
	Failure *Failure     `json:"failure,omitempty"`
	Skills  []SkillState `json:"skills"`
	Skipped []Skipped    `json:"skipped"`
}

// A user skill as the page shows it.
// +demi:root direction=receive output=plugin-skills
// +demi:tolerant
type SkillState struct {
	Name        string   `json:"name"`
	Description string   `json:"description"`
	Warnings    []string `json:"warnings"`
	Enabled     bool     `json:"enabled"`
	// The skill is never offered to the agent.
	DisableModelInvocation bool `json:"disableModelInvocation"`
}
