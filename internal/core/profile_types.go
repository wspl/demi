package core

// Profile overrides the configuration a child inherits from its parent.
type Profile struct {
	Name              string          `json:"name"`
	Description       string          `json:"description"`
	Instructions      *string         `json:"instructions,omitempty"`
	Commands          *[][]string     `json:"commands,omitempty"`
	CanSpawnSubagents bool            `json:"canSpawnSubagents"`
	Model             *ModelSelection `json:"model,omitempty"`
}
