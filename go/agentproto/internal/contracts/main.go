// contracts compares the available Go browser roots with the Rust judge output.
// Temporary scalar metadata lives here until L5 accepts opaque schema declarations.
package main

import (
	"bytes"
	"flag"
	"fmt"
	"os"
	"path/filepath"

	"github.com/wspl/demi/go/core"
	"github.com/wspl/demi/go/internal/wiregen"
)

func main() {
	judge := flag.String("judge", "", "Rust protocol contracts.ts to compare")
	output := flag.String("out", "", "output file for available protocol roots")
	webOutput := flag.String("web-out", "", "output web API TypeScript")
	webJudge := flag.String("web-judge", "", "Rust web-api.ts to compare")
	tablesOutput := flag.String("tables-out", "", "output protocol tables")
	tablesJudge := flag.String("tables-judge", "", "Rust tables.ts to compare")
	flag.Parse()
	if *tablesOutput != "" || *tablesJudge != "" {
		if err := runTables(*tablesJudge, *tablesOutput); err != nil {
			fmt.Fprintln(os.Stderr, err)
			os.Exit(1)
		}
		return
	}
	if *webOutput != "" || *webJudge != "" {
		if err := runWeb(*webJudge, *webOutput); err != nil {
			fmt.Fprintln(os.Stderr, err)
			os.Exit(1)
		}
		return
	}
	if err := run(*judge, *output); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}
func run(judge, output string) error {
	packages, options, err := loadBrowser("core", "agentproto")
	if err != nil {
		return err
	}
	roots := []wiregen.BrowserRoot{
		{Type: wiregen.BrowserType{Package: "agentproto", Name: "ServerFrame"}, Receives: true},
		{Type: wiregen.BrowserType{Package: "agentproto", Name: "ClientFrame"}},
	}
	for _, name := range []string{"Block", "ProviderModelList", "AuthState", "RuntimeState", "AccountInfo", "LoginPending", "QuotaSnapshot", "WireAPI", "PreviewType"} {
		roots = append(roots, wiregen.BrowserRoot{Type: wiregen.BrowserType{Package: "core", Name: name}, Receives: true})
	}
	data, err := wiregen.GenerateBrowserTypeScript(packages, roots, options)
	if err != nil {
		return err
	}
	if output != "" {
		if err := os.MkdirAll(filepath.Dir(output), 0755); err != nil {
			return err
		}
		if err := os.WriteFile(output, data, 0644); err != nil {
			return err
		}
	}
	if judge != "" {
		expected, err := os.ReadFile(judge)
		if err != nil {
			return err
		}
		if !bytes.Equal(data, expected) {
			return fmt.Errorf("protocol TypeScript differs: Go %d bytes, Rust %d bytes; LiveModuleMessage and LiveViewerMessage are unavailable in builtinproto", len(data), len(expected))
		}
	}
	return nil
}

func loadBrowser(names ...string) (map[string]*wiregen.Package, wiregen.BrowserOptions, error) {
	packages := map[string]*wiregen.Package{}
	options := wiregen.BrowserOptions{Extends: map[wiregen.BrowserType]wiregen.BrowserType{}, Aliases: map[wiregen.BrowserType]*wiregen.Type{}, Names: map[wiregen.BrowserType]string{}, Docs: map[wiregen.BrowserType]string{}, ScalarSchemas: map[wiregen.BrowserType]wiregen.BrowserScalarSchema{}, External: map[wiregen.BrowserType]string{}}
	for _, name := range names {
		pkg, metadata, err := Browser(name)
		if err != nil {
			return nil, options, err
		}
		packages[name] = pkg
		for ref, value := range metadata.Extends {
			options.Extends[ref] = value
		}
		for ref, value := range metadata.Aliases {
			options.Aliases[ref] = value
		}
		for ref, value := range metadata.Names {
			options.Names[ref] = value
		}
		for ref, value := range metadata.Docs {
			options.Docs[ref] = value
		}
		for ref, value := range metadata.ScalarSchemas {
			options.ScalarSchemas[ref] = value
		}
	}
	return packages, options, nil
}
func runWeb(judge, output string) error {
	packages, options, err := loadBrowser("core", "agentproto", "webapi", "builtinproto", "runnerproto", "commandservice")
	if err != nil {
		return err
	}
	protocol := []wiregen.BrowserRoot{{Type: wiregen.BrowserType{Package: "agentproto", Name: "ServerFrame"}, Receives: true}, {Type: wiregen.BrowserType{Package: "agentproto", Name: "ClientFrame"}}}
	for _, name := range []string{"Block", "ProviderModelList", "AuthState", "RuntimeState", "AccountInfo", "LoginPending", "QuotaSnapshot", "WireAPI", "PreviewType"} {
		protocol = append(protocol, wiregen.BrowserRoot{Type: wiregen.BrowserType{Package: "core", Name: name}, Receives: true})
	}
	// These existing builtin types belong to protocol through its pending live
	// roots. The web reuses their owner's declarations and imports their schemas.
	protocol = append(protocol, wiregen.BrowserRoot{Type: wiregen.BrowserType{Package: "builtinproto", Name: "BrowserCreatedBy"}, Receives: true})
	definitions, err := wiregen.BrowserDefinitions(packages, protocol, options)
	if err != nil {
		return err
	}
	for _, ref := range definitions {
		options.External[ref] = "@demicodes/protocol"
	}
	data, err := wiregen.GenerateBrowserTypeScript(packages, webRoots, options)
	if err != nil {
		return err
	}
	if output != "" {
		if err := os.MkdirAll(filepath.Dir(output), 0755); err != nil {
			return err
		}
		if err := os.WriteFile(output, data, 0644); err != nil {
			return err
		}
	}
	if judge != "" {
		expected, err := os.ReadFile(judge)
		if err != nil {
			return err
		}
		return compareWeb(data, expected, options)
	}
	return nil
}

func runTables(judge, output string) error {
	live, err := wiregen.Load("builtinproto")
	if err != nil {
		return err
	}
	data, waiting, err := wiregen.GenerateBrowserTables(core.PreviewTypes, core.AttachmentFileExtensions, core.VideoFileExtensions, live)
	if err != nil {
		return err
	}
	if output != "" {
		if err := os.MkdirAll(filepath.Dir(output), 0755); err != nil {
			return err
		}
		if err := os.WriteFile(output, data, 0644); err != nil {
			return err
		}
	}
	if judge != "" {
		expected, err := os.ReadFile(judge)
		if err != nil {
			return err
		}
		if !bytes.Equal(data, expected) {
			return fmt.Errorf("protocol tables differ; waiting on builtinproto constants: %v", waiting)
		}
	}
	return nil
}
