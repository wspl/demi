package filebrowser

import "github.com/wspl/demi/internal/plugin"

// Factory declares the plugin's identity and page package.
type Factory struct{}

// New constructs the plugin's factory.
func New() *Factory { return &Factory{} }

// Manifest returns the plugin's declarations.
func (*Factory) Manifest() plugin.Manifest {
	return plugin.Manifest{
		ID:          "file-browser",
		Name:        "File browser",
		Description: "File browser",
		Page:        &plugin.Page{Package: "@demicodes/plugin-file-browser"},
	}
}

// Instance creates an identity-only plugin.
func (*Factory) Instance() plugin.Plugin { return plugin.NoRequests{} }
