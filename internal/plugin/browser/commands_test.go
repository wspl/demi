package browser_test

import (
	"strings"
	"testing"

	"github.com/wspl/demi/internal/commandpackage/browser/browserproto"
	"github.com/wspl/demi/internal/plugin/browser"
	"github.com/wspl/demi/internal/plugin/plugintest"
	"go.uber.org/goleak"
)

func TestMain(m *testing.M) {
	goleak.VerifyTestMain(m)
}

// All scenarios use in-memory declarations and ports; no processes or wall-time waits.
func TestEveryOperationHasACommandWithOneOperandSource(t *testing.T) {
	factory, err := browser.New()
	if err != nil {
		t.Fatal(err)
	}
	roots, err := plugintest.Roots(factory.Manifest())
	if err != nil {
		t.Fatal(err)
	}
	root := roots[0]
	for _, operation := range browserproto.Operations() {
		line := append([]string{"browser"}, strings.Split(strings.TrimPrefix(operation, browserproto.Prefix), ".")...)
		parsed, err := plugintest.Parse(root, append(line, "--help"), nil)
		if err != nil || !parsed.Help {
			t.Fatalf("%s: %v, %v", operation, parsed, err)
		}
	}
	for _, test := range []struct {
		line        []string
		body, field string
	}{
		{[]string{"browser", "eval", "t1"}, "document.title", "expression"},
		{[]string{"browser", "find", "t1", "--query"}, `{"match":{"role":"button"}}`, "body"},
		{[]string{"browser", "cdp", "send", "t1", "Network.enable"}, "{}", "params"},
		{[]string{"browser", "webmcp", "call", "t1", "search", "--tools", "tools-1"}, "{}", "arguments"},
	} {
		parsed, err := plugintest.Parse(root, test.line, &test.body)
		if err != nil {
			t.Fatal(err)
		}
		if value, _ := parsed.Values.Lookup(test.field); value != test.body {
			t.Fatalf("%v: %v", test.line, parsed.Values)
		}
		if _, err := plugintest.Parse(root, append(test.line, "--"+test.field, test.body), nil); err == nil {
			t.Fatalf("duplicate source accepted: %v", test.line)
		}
	}
	for _, test := range []struct {
		line         []string
		field, value string
	}{
		{[]string{"browser", "type", "t1", "--text", "hello"}, "text", "hello"},
		{[]string{"browser", "cdp", "detach", "t1"}, "tab", "t1"},
		{[]string{"browser", "key", "t1", "--key", "ControlOrMeta+A"}, "key", "ControlOrMeta+A"},
	} {
		parsed, err := plugintest.Parse(root, test.line, nil)
		if err != nil {
			t.Fatal(err)
		}
		if value, _ := parsed.Values.Lookup(test.field); value != test.value {
			t.Fatalf("%v: %v", test.line, parsed.Values)
		}
	}
	parsed, err := plugintest.Parse(
		root,
		[]string{"browser", "content", "fetch", "--url", "https://example.test/"},
		nil,
	)
	if err != nil {
		t.Fatal(err)
	}
	value, _ := parsed.Values.Lookup("url")
	urls, ok := value.([]any)
	if !ok || len(urls) != 1 || urls[0] != "https://example.test/" {
		t.Fatalf("urls: %v", parsed.Values)
	}
	help, err := plugintest.Help(root, []string{"browser"})
	if err != nil || !strings.Contains(help, "demi browser probe") {
		t.Fatalf("help: %s, %v", help, err)
	}
}
