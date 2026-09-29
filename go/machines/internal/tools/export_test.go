package tools

import "maps"

// WithProgram returns a copy of the tools that runs path for tool, so a test can
// stand a shell in for a program.
func WithProgram(t *Tools, tool Tool, path string) *Tools {
	paths := maps.Clone(t.paths)
	paths[tool] = path
	return &Tools{paths: paths}
}
