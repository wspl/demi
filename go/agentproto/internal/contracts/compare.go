package main

import (
	"bytes"
	"fmt"
	"regexp"
	"sort"
	"strings"

	"github.com/wspl/demi/go/internal/wiregen"
)

var definitionEnd = regexp.MustCompile(`(?m)^export type (\w+) = z\.infer<typeof (\w+)>\n`)
var schemaReference = regexp.MustCompile(`\b\w+Schema\b`)
var documentation = regexp.MustCompile(`(?ms)^[ \t]*/\*\*.*?\*/\n`)

type browserDefinition struct {
	name string
	text []byte
}

// splitBrowser separates generated browser definitions without changing their bytes.
func splitBrowser(data []byte) ([]byte, map[string]browserDefinition, error) {
	boundary := bytes.Index(data, []byte("\n\n"))
	if boundary < 0 {
		return nil, nil, fmt.Errorf("browser module has no header boundary")
	}
	start := boundary + 2
	header := data[:start]
	definitions := map[string]browserDefinition{}
	for _, match := range definitionEnd.FindAllSubmatchIndex(data, -1) {
		name := string(data[match[2]:match[3]])
		schema := string(data[match[4]:match[5]])
		definitions[schema] = browserDefinition{name: name, text: data[start:match[1]]}
		start = match[1]
		if start < len(data) && data[start] == '\n' {
			start++
		}
	}
	if start != len(data) {
		return nil, nil, fmt.Errorf("unrecognized browser module tail")
	}
	return header, definitions, nil
}

// compareWeb reports byte differences in each root's transitive local definitions.
func compareWeb(actual, expected []byte, options wiregen.BrowserOptions) error {
	actualHeader, actualDefs, err := splitBrowser(actual)
	if err != nil {
		return err
	}
	expectedHeader, expectedDefs, err := splitBrowser(expected)
	if err != nil {
		return err
	}
	exact := 0
	for _, root := range webRoots {
		name := root.Type.Name
		if renamed := options.Names[root.Type]; renamed != "" {
			name = renamed
		}
		var visit func(string)
		visited := map[string]bool{}
		differences := []string{}
		visit = func(schema string) {
			if visited[schema] {
				return
			}
			visited[schema] = true
			got, exists := actualDefs[schema]
			want, wanted := expectedDefs[schema]
			if !exists && !wanted {
				return
			} // Imported protocol definition.
			if !bytes.Equal(got.text, want.text) {
				reason := "schema or missing definition"
				if exists && wanted && bytes.Equal(documentation.ReplaceAll(got.text, nil), documentation.ReplaceAll(want.text, nil)) {
					reason = "documentation"
				}
				differences = append(differences, want.name+" ("+reason+")")
			}
			for _, ref := range schemaReference.FindAllString(string(want.text), -1) {
				visit(ref)
			}
		}
		found := false
		for schema, def := range expectedDefs {
			if def.name == name {
				visit(schema)
				found = true
				break
			}
		}
		if !found {
			differences = append(differences, "root missing")
		}
		if !bytes.Equal(actualHeader, expectedHeader) {
			differences = append(differences, "header/imports")
		}
		if len(differences) == 0 {
			exact++
			continue
		}
		sort.Strings(differences)
		fmt.Printf("%s: %s\n", name, strings.Join(differences, ", "))
	}
	fmt.Printf("web roots: %d/%d byte-identical including transitive local definitions\n", exact, len(webRoots))
	if !bytes.Equal(actual, expected) {
		return fmt.Errorf("web TypeScript differs: Go %d bytes, Rust %d bytes", len(actual), len(expected))
	}
	return nil
}
