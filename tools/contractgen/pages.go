package main

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"reflect"
	"sort"
	"strings"

	"github.com/wspl/demi/internal/contract"
	"github.com/wspl/demi/tools/contractgen/pagemeta"
)

const pageScope = "@demicodes/"

func readPages(ctx context.Context, repository string) ([]pagemeta.Page, error) {
	cmd := exec.CommandContext(ctx, "go", "run", "./tools/contractgen/manifests")
	cmd.Dir = repository
	cmd.Env = append(os.Environ(), "CGO_ENABLED=0", "GOFLAGS=-mod=readonly")
	var stderr bytes.Buffer
	cmd.Stderr = &stderr
	data, err := cmd.Output()
	if err != nil {
		return nil, fmt.Errorf("page manifests: %w\n%s", err, &stderr)
	}
	return pagemeta.Decode(data)
}

// pageTypes collects manifest names and lets receipt win when a type is used both ways.
func pageTypes(page pagemeta.Page, protocol map[string]bool) (map[string]bool, error) {
	names := map[string]bool{}
	definitions := map[string]any{}
	for _, schema := range page.Schemas {
		var root map[string]json.RawMessage
		if err := json.Unmarshal(schema.Value, &root); err != nil {
			return nil, fmt.Errorf("plugin %s: schema: %w", page.ID, err)
		}
		var defs map[string]json.RawMessage
		if raw, ok := root["$defs"]; ok {
			if err := json.Unmarshal(raw, &defs); err != nil || defs == nil {
				return nil, fmt.Errorf("plugin %s: a schema's $defs is not an object: %s", page.ID, raw)
			}
			delete(root, "$defs")
		}
		collected := make([]string, 0, len(defs)+1)
		for name, definition := range defs {
			if err := insertPageDefinition(page.ID, definitions, name, definition); err != nil {
				return nil, err
			}
			collected = append(collected, name)
		}
		if string(root["type"]) != `"null"` {
			var title string
			if err := json.Unmarshal(root["title"], &title); err != nil || title == "" {
				return nil, fmt.Errorf("plugin %s: a manifest schema is not a named type: %s", page.ID, schema.Value)
			}
			delete(root, "title")
			// Tool metadata is not a wire value: member order is irrelevant here.
			definition, err := json.Marshal(root)
			if err != nil {
				return nil, err
			}
			if err := insertPageDefinition(page.ID, definitions, title, definition); err != nil {
				return nil, err
			}
			collected = append(collected, title)
		}
		for _, name := range collected {
			if !protocol[name] {
				names[name] = names[name] || schema.Direction == "receive"
			}
		}
	}
	return names, nil
}

// insertPageDefinition enforces Rust's one-schema-per-name rule across a page.
func insertPageDefinition(plugin string, definitions map[string]any, name string, data []byte) error {
	var definition any
	decoder := json.NewDecoder(bytes.NewReader(data))
	decoder.UseNumber()
	if err := decoder.Decode(&definition); err != nil {
		return fmt.Errorf("plugin %s: type %s: %w", plugin, name, err)
	}
	if existing, ok := definitions[name]; ok && !reflect.DeepEqual(existing, definition) {
		return fmt.Errorf("plugin %s: two schemas of %s differ", plugin, name)
	}
	definitions[name] = definition
	return nil
}

func checkPage(page pagemeta.Page, actual, protocol map[string]bool) error {
	expected, err := pageTypes(page, protocol)
	if err != nil {
		return err
	}
	names := make([]string, 0, len(expected)+len(actual))
	for name := range expected {
		names = append(names, name)
	}
	for name := range actual {
		if _, ok := expected[name]; !ok {
			names = append(names, name)
		}
	}
	sort.Strings(names)
	for _, name := range names {
		want, declared := expected[name]
		got, emitted := actual[name]
		switch {
		case !declared:
			return fmt.Errorf("plugin %s: type %s is extra in generated output", page.ID, name)
		case !emitted:
			return fmt.Errorf("plugin %s: type %s is missing from generated output", page.ID, name)
		case want != got:
			return fmt.Errorf("plugin %s: type %s has wrong direction (receive=%t, manifest receive=%t)", page.ID, name, got, want)
		}
	}
	return nil
}

func (g *generator) pageSources(pages []pagemeta.Page, sources map[string][]byte) error {
	protocol := map[string]bool{}
	for name := range g.tsExports["protocol"] {
		protocol[name] = true
	}
	owned := map[string]bool{}
	for _, page := range pages {
		if !strings.HasPrefix(page.Package, pageScope) {
			return fmt.Errorf("the page package %s is not a workspace package of %s", page.Package, pageScope)
		}
		output := strings.TrimPrefix(page.Package, pageScope)
		owned[output] = true
		if err := checkPage(page, g.tsExports[output], protocol); err != nil {
			return err
		}
		// A page with no schemas (changes and file-browser) still owns a module.
		source := sources[output]
		if source == nil {
			if len(page.Schemas) != 0 {
				return fmt.Errorf("plugin %s: page package %s has no matching output", page.ID, page.Package)
			}
			source = []byte(tsHeader)
		}
		var suffix strings.Builder
		suffix.WriteString("\n/** The plugin this package is the page of. */\n")
		fmt.Fprintf(&suffix, "export const PLUGIN = %s\n", q(page.ID))
		for _, constant := range page.Constants {
			text := strings.ReplaceAll(constant.Description, "*/", "*\\/")
			lines := strings.Split(strings.TrimSuffix(text, "\n"), "\n")
			if text == "" {
				lines = nil
			}
			if len(lines) == 1 {
				fmt.Fprintf(&suffix, "/** %s */\n", strings.TrimSuffix(lines[0], "\r"))
			} else {
				suffix.WriteString("/**\n")
				for _, line := range lines {
					line = strings.TrimSuffix(line, "\r")
					if line == "" {
						suffix.WriteString(" *\n")
					} else {
						fmt.Fprintf(&suffix, " * %s\n", line)
					}
				}
				suffix.WriteString(" */\n")
			}
			// Metadata uses encoding/json, which re-escapes U+2028/U+2029.
			// Restore serde_json spelling without decoding objects into maps.
			value, err := contract.EncodeJSON(constant.Value)
			if err != nil {
				return fmt.Errorf("plugin %s constant %s: %w", page.ID, constant.Name, err)
			}
			fmt.Fprintf(&suffix, "export const %s = %s\n", constant.Name, value)
		}
		sources[output] = append(source, suffix.String()...)
	}
	for output := range sources {
		if strings.HasPrefix(output, "plugin-") && !owned[output] {
			return fmt.Errorf("output %s has no page manifest", output)
		}
	}
	return nil
}

func pagesOf(repository, app string, registered []pagemeta.Page) ([]pagemeta.Page, error) {
	path := filepath.Join(repository, app, "package.json")
	data, err := os.ReadFile(path)
	if err != nil {
		return nil, err
	}
	var manifest map[string]json.RawMessage
	if err := json.Unmarshal(data, &manifest); err != nil {
		return nil, fmt.Errorf("%s: %w", path, err)
	}
	if manifest == nil {
		return nil, fmt.Errorf("%s: expected package object", path)
	}
	dependencies := map[string]*string{}
	if raw, ok := manifest["dependencies"]; ok {
		if err := json.Unmarshal(raw, &dependencies); err != nil {
			return nil, fmt.Errorf("%s: %w", path, err)
		}
		if dependencies == nil {
			return nil, fmt.Errorf("%s: dependencies must be an object", path)
		}
		for name, version := range dependencies {
			if version == nil {
				return nil, fmt.Errorf("%s: dependency %s must be a string", path, name)
			}
		}
	}
	var pages []pagemeta.Page
	for _, page := range registered {
		if _, ok := dependencies[page.Package]; ok {
			pages = append(pages, page)
		}
	}
	return pages, nil
}

func pageBinding(plugin string) string {
	var name strings.Builder
	upper := false
	for _, char := range plugin {
		if char == '-' {
			upper = true
		} else if upper {
			name.WriteString(strings.ToUpper(string(char)))
			upper = false
		} else {
			name.WriteRune(char)
		}
	}
	return name.String() + "Page"
}

func registryModule(pages []pagemeta.Page) []byte {
	var source strings.Builder
	source.WriteString(tsHeader)
	source.WriteString("import type { AnyPluginPage } from \"@demicodes/plugin-sdk\"\n")
	for _, page := range pages {
		fmt.Fprintf(&source, "import %s from %s\n", pageBinding(page.ID), q(page.Package))
	}
	source.WriteString("\n/** The plugin pages this app shows, in the order the backend registers their plugins (`plugin-pages.md` § Registration). */\n")
	source.WriteString("export const PLUGIN_PAGES: readonly AnyPluginPage[] = [\n")
	for _, page := range pages {
		fmt.Fprintf(&source, "  %s,\n", pageBinding(page.ID))
	}
	source.WriteString("]\n")
	return []byte(source.String())
}

func writeRegistries(repository string, registered []pagemeta.Page, verify bool) error {
	for _, app := range []struct{ directory, output string }{
		{"packages/web", "src/plugins/generated/pages.ts"},
		{"packages/web-gallery", "src/generated/pages.ts"},
	} {
		pages, err := pagesOf(repository, app.directory, registered)
		if err != nil {
			return err
		}
		path := filepath.Join(repository, app.directory, app.output)
		if !verify {
			if err := os.MkdirAll(filepath.Dir(path), 0755); err != nil {
				return err
			}
		}
		if err := writeGenerated(path, registryModule(pages), verify); err != nil {
			return err
		}
	}
	return nil
}
