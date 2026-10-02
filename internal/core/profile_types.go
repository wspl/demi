package core

// Every field overrides what a child would inherit from its parent.
// +demi:root
type Profile struct {
	// The `--profile` value.
	Name string `json:"name"`
	// What the profile is for, listed in the spawn command's help.
	Description string `json:"description"`
	// Replaces the instructions in the child's system prompt.
	Instructions *string `json:"instructions,omitempty"`
	// The command paths, such as `["demi", "file"]`, the child keeps of its
	// parent's commands; every other command is left out.
	Commands *[][]string `json:"commands,omitempty"`
	// Whether the profile's children may spawn children of their own.
	CanSpawnSubagents bool `json:"canSpawnSubagents"`
	// A model used instead of the parent's, on a fork of the parent's
	// provider runtime.
	Model *ModelSelection `json:"model,omitempty"`
}
