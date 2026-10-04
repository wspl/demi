package cmdproto_test

import (
	"strings"
	"testing"

	"github.com/wspl/demi/internal/cmdproto"
	"github.com/wspl/demi/internal/contract"
)

func TestCommandInputValidation(t *testing.T) {
	valid := `{"operation":"echo","invocationId":"i","context":{"conversation":"c-1",` +
		`"caller":{"kind":"agent","number":18446744073709551615},"locale":{"timeZone":"UTC",` +
		`"languages":["en"]}},"args":{},"cwd":"/tmp","env":{}}`
	agent := `"caller":{"kind":"agent","number":18446744073709551615}`
	locale := `"locale":{"timeZone":"UTC","languages":["en"]}`
	for _, caller := range []string{agent, `"caller":{"kind":"user"}`} {
		if _, err := cmdproto.DecodeInvocation([]byte(strings.Replace(valid, agent, caller, 1))); err != nil {
			t.Fatalf("%s: %v", caller, err)
		}
	}
	for _, tc := range []struct{ name, old, new string }{
		{"missing context", `"context":{"conversation":"c-1",` + agent + `,` + locale + `},`, ``},
		{"empty operation", `"operation":"echo"`, `"operation":""`},
		{"empty invocation", `"invocationId":"i"`, `"invocationId":""`},
		{"empty conversation", `"c-1"`, `""`},
		{"unsafe conversation", `"c-1"`, `"../c"`},
		{"negative agent", `18446744073709551615`, `-1`},
		{"agent without number", agent, `"caller":{"kind":"agent"}`},
		{"user with number", `"agent"`, `"user"`},
		{"unknown caller", `"agent"`, `"robot"`},
		{"missing locale", `,` + locale, ``},
		{"empty time zone", `"UTC"`, `""`},
		{"no language", `["en"]`, `[]`},
		{"too many languages", `["en"]`, `[` + strings.TrimSuffix(strings.Repeat(`"en",`, 17), ",") + `]`},
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
			invalid := strings.Replace(valid, tc.old, tc.new, 1)
			if invalid == valid {
				t.Fatalf("mutation missed %s", tc.old)
			}
			if _, err := cmdproto.DecodeInvocation([]byte(invalid)); err == nil {
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
		{`{"id":1,"first":1}`, true},
		{`{"id":1,"error":"failed"}`, true},
		{`{"id":1}`, false},
		{`{"id":1,"first":1,"error":"failed"}`, false},
		{`{"id":1,"first":0}`, false},
		{`{"id":1,"error":""}`, false},
	} {
		_, err := cmdproto.DecodeNumbersAnswer([]byte(tc.data))
		if (err == nil) != tc.valid {
			t.Fatalf("%s: %v", tc.data, err)
		}
	}
	for _, tc := range []struct {
		data  string
		valid bool
	}{
		{`{"id":1,"installed":[]}`, true},
		{`{"id":1,"path":"/tmp/x"}`, true},
		{`{"id":1,"error":"failed"}`, true},
		{`{"id":1}`, false},
		{`{"id":1,"installed":[],"path":"/tmp/x"}`, false},
	} {
		_, err := cmdproto.DecodeArtifactAnswer([]byte(tc.data))
		if (err == nil) != tc.valid {
			t.Fatalf("%s: %v", tc.data, err)
		}
	}
	for _, data := range []string{`{"id":1}`, `{"id":1,"installed":{"name":"x"},"install":null}`} {
		if _, err := cmdproto.DecodeArtifactRequest([]byte(data)); err == nil {
			t.Fatal("accepted invalid artifact request")
		}
	}
}

func TestPackageValidation(t *testing.T) {
	for _, tc := range []struct {
		url   string
		valid bool
	}{
		{"https://example.com/a?signature=abc", true},
		{"http://localhost/a", true},
		{"file:///tmp/a", false},
		{"https://user:pass@example.com/a", false},
		{"https://user@example.com", false},
		{"ftp://192.168.5.2/native-artifacts/a", false},
		{"native-artifacts/a", false},
	} {
		value := cmdproto.ArtifactURL{URL: tc.url}
		_, err := contract.EncodeJSON(&value)
		if (err == nil) != tc.valid {
			t.Fatalf("%s: %v", tc.url, err)
		}
	}
	for _, entry := range []string{"", "/a", "a/../b", "a//b", "a\\b", "./a"} {
		if err := (&cmdproto.ArtifactArchive{Entry: entry}).Validate(); err == nil {
			t.Fatalf("accepted archive entry %q", entry)
		}
	}
	if err := (&cmdproto.ArtifactArchive{Entry: "bin/tool"}).Validate(); err != nil {
		t.Fatal(err)
	}
}

func TestEditJournalBoundary(t *testing.T) {
	valid := `{"files":[{"path":"file.txt","kind":"modified","edits":[{"original":"0.before",` +
		`"modified":"0.after"}]}],"bytesCopied":10,"nextSegment":1,"filesTruncated":false}`
	if _, err := cmdproto.DecodeEditJournal([]byte(valid)); err != nil {
		t.Fatal(err)
	}
	for _, tc := range []struct{ old, new string }{
		{`"bytesCopied":10`, `"bytesCopied":67108865`},
		{`"nextSegment":1`, `"nextSegment":1001`},
		{`"file.txt"`, `"file\u0000.txt"`},
		{`"0.before"`, `null`},
		{`"modified"`, `"deleted"`},
	} {
		if _, err := cmdproto.DecodeEditJournal([]byte(strings.Replace(valid, tc.old, tc.new, 1))); err == nil {
			t.Fatal("accepted invalid edit journal")
		}
	}
	for _, tc := range []struct {
		data []byte
		text bool
	}{
		{[]byte(""), true}, {[]byte("hello 😀\n"), true}, {[]byte{0}, false}, {[]byte{255}, false},
	} {
		if cmdproto.IsText(tc.data) != tc.text {
			t.Fatalf("text classification: %q", tc.data)
		}
	}
}

func TestConversationAndEditContext(t *testing.T) {
	for _, valid := range []string{`{"operation":"release","conversation":"conv_1"}`, `{"operation":"status"}`} {
		if _, err := cmdproto.DecodeConversationRequest([]byte(valid)); err != nil {
			t.Fatal(err)
		}
	}
	for _, invalid := range []string{
		`{"operation":"release","conversation":".."}`,
		`{"operation":"status","conversation":"conv_1"}`,
	} {
		if _, err := cmdproto.DecodeConversationRequest([]byte(invalid)); err == nil {
			t.Fatal("accepted invalid lifecycle request")
		}
	}
	context := cmdproto.EditContext{Directory: t.TempDir(), Lock: t.TempDir()}
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

func TestConversationStatusChecksConversationIdentity(t *testing.T) {
	for _, s := range []string{`{"conversations":[]}`, `{"conversations":["conversation"]}`} {
		if _, err := cmdproto.DecodeConversationStatus([]byte(s)); err != nil {
			t.Fatalf("%s: %v", s, err)
		}
	}
	if _, err := cmdproto.DecodeConversationStatus([]byte(`{"conversations":[""]}`)); err == nil {
		t.Fatal("empty identity")
	}
}
