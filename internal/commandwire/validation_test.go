package commandwire_test

import (
	"strings"
	"testing"

	"github.com/wspl/demi/internal/commandwire"
	"github.com/wspl/demi/internal/contract"
)

func TestCommandInputValidation(t *testing.T) {
	valid := `{"operation":"echo","invocationId":"i","context":{"conversation":"c-1","caller":{"kind":"agent","number":18446744073709551615},"locale":{"timeZone":"UTC","languages":["en"]}},"args":{},"cwd":"/tmp","env":{}}`
	if _, err := commandwire.DecodeInvocation([]byte(valid)); err != nil {
		t.Fatal(err)
	}
	for _, tc := range []struct{ name, old, new string }{
		{"unsafe conversation", `"c-1"`, `"../c"`},
		{"unknown caller", `"agent"`, `"robot"`},
		{"empty language", `["en"]`, `[""]`},
		{"long language", `["en"]`, `["` + strings.Repeat("a", 65) + `"]`},
		{"arguments array", `"args":{}`, `"args":[]`},
		{"nul cwd", `"/tmp"`, `"/tmp\u0000"`},
		{"bad environment name", `"env":{}`, `"env":{"A=B":"x"}`},
		{"bad environment value", `"env":{}`, `"env":{"A":"\u0000"}`},
		{"null optional", `"args":{}`, `"args":{},"json":null`},
		{"unknown field", `"args":{}`, `"args":{},"extra":1`},
		{"duplicate arguments", `"args":{}`, `"args":{"a":1,"a":2}`},
	} {
		t.Run(tc.name, func(t *testing.T) {
			if _, err := commandwire.DecodeInvocation([]byte(strings.Replace(valid, tc.old, tc.new, 1))); err == nil {
				t.Fatal("accepted malformed invocation")
			}
		})
	}
}

func TestStreamOutcomes(t *testing.T) {
	for _, tc := range []struct {
		data  string
		valid bool
	}{
		{`{"id":1,"first":1}`, true}, {`{"id":1,"error":"failed"}`, true},
		{`{"id":1}`, false}, {`{"id":1,"first":1,"error":"failed"}`, false},
		{`{"id":1,"first":0}`, false}, {`{"id":1,"error":""}`, false},
	} {
		_, err := commandwire.DecodeNumbersAnswer([]byte(tc.data))
		if (err == nil) != tc.valid {
			t.Fatalf("%s: %v", tc.data, err)
		}
	}
	for _, tc := range []struct {
		data  string
		valid bool
	}{
		{`{"id":1,"installed":[]}`, true}, {`{"id":1,"path":"/tmp/x"}`, true},
		{`{"id":1,"error":"failed"}`, true}, {`{"id":1}`, false},
		{`{"id":1,"installed":[],"path":"/tmp/x"}`, false},
	} {
		_, err := commandwire.DecodeArtifactAnswer([]byte(tc.data))
		if (err == nil) != tc.valid {
			t.Fatalf("%s: %v", tc.data, err)
		}
	}
	for _, data := range []string{`{"id":1}`, `{"id":1,"installed":{"name":"x"},"install":null}`} {
		if _, err := commandwire.DecodeArtifactRequest([]byte(data)); err == nil {
			t.Fatal("accepted invalid artifact request")
		}
	}
}

func TestPackageValidation(t *testing.T) {
	for _, tc := range []struct {
		url   string
		valid bool
	}{
		{"https://example.com/a?signature=abc", true}, {"http://localhost/a", true},
		{"file:///tmp/a", false}, {"https://user:pass@example.com/a", false}, {"https://user@example.com", false},
	} {
		value := commandwire.ArtifactURL{URL: tc.url}
		_, err := contract.EncodeJSON(&value)
		if (err == nil) != tc.valid {
			t.Fatalf("%s: %v", tc.url, err)
		}
	}
	for _, entry := range []string{"", "/a", "a/../b", "a//b", "a\\b", "./a"} {
		if err := (&commandwire.ArtifactArchive{Entry: entry}).Validate(); err == nil {
			t.Fatalf("accepted archive entry %q", entry)
		}
	}
	if err := (&commandwire.ArtifactArchive{Entry: "bin/tool"}).Validate(); err != nil {
		t.Fatal(err)
	}
}

func TestEditJournalBoundary(t *testing.T) {
	valid := `{"files":[{"path":"file.txt","kind":"modified","edits":[{"original":"0.before","modified":"0.after"}]}],"bytesCopied":10,"nextSegment":1,"filesTruncated":false}`
	if _, err := commandwire.DecodeEditJournal([]byte(valid)); err != nil {
		t.Fatal(err)
	}
	for _, tc := range []struct{ old, new string }{
		{`"bytesCopied":10`, `"bytesCopied":67108865`},
		{`"nextSegment":1`, `"nextSegment":1001`},
		{`"file.txt"`, `"file\u0000.txt"`},
		{`"0.before"`, `null`},
		{`"modified"`, `"deleted"`},
	} {
		if _, err := commandwire.DecodeEditJournal([]byte(strings.Replace(valid, tc.old, tc.new, 1))); err == nil {
			t.Fatal("accepted invalid edit journal")
		}
	}
	for _, tc := range []struct {
		data []byte
		text bool
	}{
		{[]byte(""), true}, {[]byte("hello 😀\n"), true}, {[]byte{0}, false}, {[]byte{255}, false},
	} {
		if commandwire.IsText(tc.data) != tc.text {
			t.Fatalf("text classification: %q", tc.data)
		}
	}
}

func TestConversationAndEditContext(t *testing.T) {
	for _, valid := range []string{`{"operation":"release","conversation":"conv_1"}`, `{"operation":"status"}`} {
		if _, err := commandwire.DecodeConversationRequest([]byte(valid)); err != nil {
			t.Fatal(err)
		}
	}
	for _, invalid := range []string{`{"operation":"release","conversation":".."}`, `{"operation":"status","conversation":"conv_1"}`} {
		if _, err := commandwire.DecodeConversationRequest([]byte(invalid)); err == nil {
			t.Fatal("accepted invalid lifecycle request")
		}
	}
	context := commandwire.EditContext{Directory: t.TempDir(), Lock: t.TempDir()}
	if err := context.Validate(); err != nil {
		t.Fatal(err)
	}
	context.Lock = "relative"
	if err := context.Validate(); err == nil {
		t.Fatal("accepted relative lock")
	}
	context.Lock = context.Directory + "\x00"
	if err := context.Validate(); err == nil {
		t.Fatal("accepted NUL in lock")
	}
}
