package file

import (
	"github.com/wspl/demi/internal/cmdpkg/file/fileop"
	"github.com/wspl/demi/internal/declare"
	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/plugin"
)

// Factory declares the native file commands.
type Factory struct{ commands *plugin.CommandPlugin }

// New constructs and validates the file command declarations.
func New() (*Factory, error) {
	read, err := readCommand()
	if err != nil {
		return nil, err
	}
	create, err := createCommand()
	if err != nil {
		return nil, err
	}
	edit, err := editCommand()
	if err != nil {
		return nil, err
	}
	patch, err := patchCommand()
	if err != nil {
		return nil, err
	}
	children := []host.Declared{read, create, edit, patch}
	commands, err := plugin.NewCommandPlugin(
		plugin.PlacementDemi,
		[]host.Declared{
			host.Group("file", "Read, create, edit, and patch workspace files (text, images, and video).", children...),
		},
	)
	if err != nil {
		return nil, err
	}
	return &Factory{commands: commands}, nil
}

// Manifest returns the plugin's declarations.
func (f *Factory) Manifest() plugin.Manifest {
	return plugin.Manifest{
		ID:          "file",
		Name:        "File commands",
		Description: "Reads, writes, edits and searches the conversation's files with `demi file`.",
		Commands:    f.commands.ManifestCommands(),
	}
}

// Instance returns the immutable native command dispatcher.
func (f *Factory) Instance() plugin.Plugin { return f.commands }

func readCommand() (host.Declared, error) {
	input, err := declare.NewSchema(fileop.ReadArgsJSONSchema())
	if err != nil {
		return host.Declared{}, err
	}
	return host.Leaf(declare.Leaf[declare.NativeOperation]{
		Name: "read",
		Summary: "Read a file. " +
			"Text files print as text; image and video files are shown to you as viewable media. " +
			"Output is the raw file bytes, so it also pipes cleanly into other commands (e.g. ffmpeg).",
		SuccessOutput: new(
			"writes the raw file bytes to stdout; an image or video result is presented to you as viewable media",
		),
		FailureOutput: new("writes the reason to stderr and exits non-zero if the path is missing or unreadable"),
		Input:         input,
		Positionals:   &[]string{"path"},
		Kind: &declare.Native[declare.NativeOperation]{
			Binding: declare.NativeOperation{Package: fileop.Package, Operation: "file.read"},
		},
	}, nil), nil
}

func createCommand() (host.Declared, error) {
	input, err := declare.NewSchema(fileop.CreateArgsJSONSchema())
	if err != nil {
		return host.Declared{}, err
	}
	return host.Leaf(declare.Leaf[declare.NativeOperation]{
		Name:          "create",
		Summary:       "Create a new file. Fails if the file exists.",
		SuccessOutput: new("writes \"Created <path>\" to stdout"),
		FailureOutput: new("writes the reason to stderr and exits non-zero without overwriting existing files"),
		Input:         input,
		Positionals:   &[]string{"path"},
		StdinField:    new("content"),
		Kind: &declare.Native[declare.NativeOperation]{
			Binding: declare.NativeOperation{Package: fileop.Package, Operation: "file.create"},
		},
	}, nil), nil
}

func editCommand() (host.Declared, error) {
	input, err := declare.NewSchema(fileop.EditArgsJSONSchema())
	if err != nil {
		return host.Declared{}, err
	}
	return host.Leaf(declare.Leaf[declare.NativeOperation]{
		Name:          "edit",
		Summary:       "Replace exact text in an existing file.",
		SuccessOutput: new("writes \"Edited <path>\" to stdout"),
		FailureOutput: new(
			"writes no-match, ambiguous-match, or write errors to stderr and exits non-zero without partial writes",
		),
		Input:       input,
		Positionals: &[]string{"path"},
		Kind: &declare.Native[declare.NativeOperation]{
			Binding: declare.NativeOperation{Package: fileop.Package, Operation: "file.edit"},
		},
	}, nil), nil
}

func patchCommand() (host.Declared, error) {
	input, err := declare.NewSchema(fileop.PatchArgsJSONSchema())
	if err != nil {
		return host.Declared{}, err
	}
	return host.Leaf(declare.Leaf[declare.NativeOperation]{
		Name:          "patch",
		Summary:       "Apply a unified diff patch to one or more files.",
		SuccessOutput: new("writes \"Patched <n> file(s)\" to stdout"),
		FailureOutput: new(
			"writes parse, validation, or write errors to stderr and exits non-zero " +
				"after rolling back partial writes when possible",
		),
		Input:      input,
		StdinField: new("patch"),
		Kind: &declare.Native[declare.NativeOperation]{
			Binding: declare.NativeOperation{Package: fileop.Package, Operation: "file.patch"},
		},
	}, nil), nil
}
