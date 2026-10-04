package main

import (
	"bytes"
	"encoding/json"
	"go/ast"
	"go/parser"
	"go/token"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"testing"

	"github.com/wspl/demi/tools/contractgen/testdata/pluginbrowser"
)

// The full golden browser manifest pins each page and stream use,
// including definition order and nullable references. Local generation <2 s.
func TestBrowserPluginSchemas(t *testing.T) {
	if err := generate(t.Context(), []string{"./testdata/pluginbrowser"}, false, "", true); err != nil {
		t.Fatal(err)
	}
	data, err := os.ReadFile("testdata/pluginbrowser/manifest.json")
	if err != nil {
		t.Fatal(err)
	}
	var manifest struct {
		Streams []struct{ Receives, Sends json.RawMessage }
		Page    struct {
			Conversation struct{ Schema json.RawMessage }
			Methods      []struct {
				Name           string
				Params, Result json.RawMessage
			}
		}
	}
	if err := json.Unmarshal(data, &manifest); err != nil {
		t.Fatal(err)
	}
	schemas := map[string]json.RawMessage{
		"BrowserTabs": pluginbrowser.BrowserTabsPluginJSONSchema(),
		"OpenTab":     pluginbrowser.OpenTabPluginJSONSchema(),
		"OpenedTab":   pluginbrowser.OpenedTabPluginJSONSchema(),
		"CloseTab":    pluginbrowser.CloseTabPluginJSONSchema(),
		"NavigateTab": pluginbrowser.NavigateTabPluginJSONSchema(),
		"TabHistory":  pluginbrowser.TabHistoryPluginJSONSchema(),
	}
	// g-browser owns the two schema markers, not yet merged on this base.
	// Add them only in the loader overlay; never modify its worktree or duplicate
	// the live contracts. Inspect the exact literals the generator emits.
	directory, err := filepath.Abs("../../internal/cmdpkg/browser/browserop")
	if err != nil {
		t.Fatal(err)
	}
	err = generateBatch(t.Context(), []string{directory}, false, "", false,
		map[string]bool{filepath.Join(directory, "contract_gen.go"): true}, map[string][]byte{},
		func(path string, source []byte) error {
			file, err := parser.ParseFile(token.NewFileSet(), path, source, 0)
			if err != nil {
				return err
			}
			for _, decl := range file.Decls {
				fn, ok := decl.(*ast.FuncDecl)
				if !ok ||
					fn.Name.Name != "LiveModuleMessagePluginJSONSchema" &&
						fn.Name.Name != "LiveViewerMessagePluginJSONSchema" {
					continue
				}
				ret := fn.Body.List[0].(*ast.ReturnStmt)
				literal := ret.Results[0].(*ast.CallExpr).Args[0].(*ast.BasicLit)
				value, err := strconv.Unquote(literal.Value)
				if err != nil {
					return err
				}
				schemas[strings.TrimSuffix(fn.Name.Name, "PluginJSONSchema")] = json.RawMessage(value)
			}
			return nil
		})
	if err != nil {
		t.Fatal(err)
	}
	var expected []json.RawMessage
	expected = append(expected, manifest.Page.Conversation.Schema)
	for _, method := range manifest.Page.Methods {
		expected = append(expected, method.Params)
		if method.Name == "open" {
			expected = append(expected, method.Result)
		}
	}
	for _, stream := range manifest.Streams {
		expected = append(expected, stream.Receives, stream.Sends)
	}
	if len(expected) != 8 {
		t.Fatalf("schema coverage: %d", len(expected))
	}
	for _, raw := range expected {
		var header struct{ Title string }
		if err := json.Unmarshal(raw, &header); err != nil {
			t.Fatal(err)
		}
		t.Run(header.Title, func(t *testing.T) {
			var compact bytes.Buffer
			if err := json.Compact(&compact, raw); err != nil {
				t.Fatal(err)
			}
			if !bytes.Equal(schemas[header.Title], compact.Bytes()) {
				t.Fatalf("plugin schema differs\ngot %s\nwant %s", schemas[header.Title], compact.Bytes())
			}
		})
	}
}
