package commandservice_test

import (
	"encoding/json/v2"
	"errors"
	"os"
	"strings"
	"testing"

	"github.com/wspl/demi/go/commandservice"
)

// The descriptor fixture the Rust crate's tests read as well, so both
// languages judge the same bytes.
const descriptorFixture = "../../crates/command-service/tests/command_service/fixtures/package.json"

// mutate returns a copy of the JSON document with the value at pointer, a
// path of object keys, replaced; a nil replacement removes the member.
func mutate(t *testing.T, document map[string]any, replacement any, path ...string) map[string]any {
	t.Helper()
	data, err := json.Marshal(document)
	if err != nil {
		t.Fatal(err)
	}
	var copied map[string]any
	if err := json.Unmarshal(data, &copied); err != nil {
		t.Fatal(err)
	}
	object := copied
	for _, key := range path[:len(path)-1] {
		object = object[key].(map[string]any)
	}
	last := path[len(path)-1]
	if replacement == nil {
		delete(object, last)
	} else {
		object[last] = replacement
	}
	return copied
}

func encoded(t *testing.T, document any) []byte {
	t.Helper()
	data, err := json.Marshal(document)
	if err != nil {
		t.Fatal(err)
	}
	return data
}

func loadDescriptorFixture(t *testing.T) (descriptor map[string]any, digest string) {
	t.Helper()
	data, err := os.ReadFile(descriptorFixture)
	if err != nil {
		t.Fatal(err)
	}
	var fixture struct {
		Descriptor map[string]any `json:"descriptor"`
		Digest     string         `json:"digest"`
	}
	if err := json.Unmarshal(data, &fixture); err != nil {
		t.Fatal(err)
	}
	return fixture.Descriptor, fixture.Digest
}

func decodeDescriptor(t *testing.T, document map[string]any) (commandservice.PackageDescriptor, error) {
	t.Helper()
	return commandservice.Decode[commandservice.PackageDescriptor](encoded(t, document))
}

func TestDescriptorValidatesAndHashesToItsRecordedDigest(t *testing.T) {
	document, digest := loadDescriptorFixture(t)
	descriptor, err := decodeDescriptor(t, document)
	if err != nil {
		t.Fatal(err)
	}
	got, err := descriptor.Digest()
	if err != nil {
		t.Fatal(err)
	}
	if got != digest {
		t.Errorf("digest = %s, want %s", got, digest)
	}
	// A development release carries fewer targets, but never an unknown one.
	development := mutate(t, document, nil, "targets", "aarch64-apple-darwin")
	if _, err := decodeDescriptor(t, development); err != nil {
		t.Errorf("a release with fewer targets: %v", err)
	}
	unknown := mutate(t, development, map[string]any{"sha256": strings.Repeat("0", 64), "size": 1}, "targets", "riscv64-unknown-linux-musl")
	if _, err := decodeDescriptor(t, unknown); err == nil {
		t.Error("a release with an unknown target was accepted")
	}
}

func TestDescriptorConstraintsAreEnforced(t *testing.T) {
	document, _ := loadDescriptorFixture(t)
	darwin := []string{"targets", "aarch64-apple-darwin"}
	for name, invalid := range map[string]map[string]any{
		"id":                mutate(t, document, "Invalid package", "id"),
		"single-part id":    mutate(t, document, "fixture", "id"),
		"empty version":     mutate(t, document, "", "version"),
		"protocol version":  mutate(t, document, 2, "protocolVersion"),
		"no operations":     mutate(t, document, []any{}, "operations"),
		"empty operation":   mutate(t, document, []any{""}, "operations"),
		"repeated":          mutate(t, document, []any{"same", "same"}, "operations"),
		"null operations":   mutate(t, document, []any(nil), "operations"),
		"digest":            mutate(t, document, strings.Repeat("g", 64), append(darwin, "sha256")...),
		"upper case digest": mutate(t, document, strings.Repeat("A", 64), append(darwin, "sha256")...),
		"short digest":      mutate(t, document, strings.Repeat("a", 63), append(darwin, "sha256")...),
		"zero size":         mutate(t, document, 0, append(darwin, "size")...),
		"fractional size":   mutate(t, document, 1.5, append(darwin, "size")...),
		"size beyond 2^53":  mutate(t, document, uint64(1)<<53, append(darwin, "size")...),
		"missing size":      mutate(t, document, nil, append(darwin, "size")...),
		"extra member":      mutate(t, document, true, "extra"),
	} {
		if _, err := decodeDescriptor(t, invalid); err == nil {
			t.Errorf("%s: the descriptor was accepted", name)
		}
	}
	// The sizes at the limits of what a JavaScript peer holds exactly are fine.
	for _, size := range []uint64{1, 1<<53 - 1} {
		if _, err := decodeDescriptor(t, mutate(t, document, size, append(darwin, "size")...)); err != nil {
			t.Errorf("size %d: %v", size, err)
		}
	}
	// A descriptor that breaks a rule is not encoded either.
	descriptor, err := decodeDescriptor(t, document)
	if err != nil {
		t.Fatal(err)
	}
	descriptor.Operations = []string{"same", "same"}
	if _, err := commandservice.Encode(descriptor); err == nil {
		t.Error("a descriptor with a repeated operation was encoded")
	}
}

func TestADescriptorServesTheCatalogOfItsOperationsInAnyOrder(t *testing.T) {
	descriptor := commandservice.PackageDescriptor{ProtocolVersion: 1, Operations: []string{"a", "b"}}
	for name, test := range map[string]struct {
		info   commandservice.ServiceInfo
		serves bool
	}{
		"the same operations":      {commandservice.ServiceInfo{ProtocolVersion: 1, Operations: []string{"a", "b"}}, true},
		"in another order":         {commandservice.ServiceInfo{ProtocolVersion: 1, Operations: []string{"b", "a"}}, true},
		"an operation more":        {commandservice.ServiceInfo{ProtocolVersion: 1, Operations: []string{"a", "b", "c"}}, false},
		"an operation less":        {commandservice.ServiceInfo{ProtocolVersion: 1, Operations: []string{"a"}}, false},
		"another protocol version": {commandservice.ServiceInfo{ProtocolVersion: 2, Operations: []string{"a", "b"}}, false},
	} {
		if got := descriptor.Serves(test.info); got != test.serves {
			t.Errorf("%s: serves = %v, want %v", name, got, test.serves)
		}
	}
}

func TestAPackageWithoutAnArtifactForATargetSaysSoInTheRustsWords(t *testing.T) {
	document, _ := loadDescriptorFixture(t)
	descriptor, err := decodeDescriptor(t, mutate(t, document, nil, "targets", "aarch64-apple-darwin"))
	if err != nil {
		t.Fatal(err)
	}
	_, err = descriptor.Artifact(commandservice.TargetDarwinArm64)
	if !errors.Is(err, commandservice.ErrMissingTarget) || err.Error() != "the package has no artifact for aarch64-apple-darwin" {
		t.Errorf("error = %v", err)
	}
}

func TestArtifactLocationsAreHTTPOrHTTPSWithoutCredentials(t *testing.T) {
	valid := func(document string) bool {
		_, err := commandservice.Decode[commandservice.ArtifactLocation]([]byte(document))
		return err == nil
	}
	for _, accepted := range []string{
		`{"url":"https://demi-native.s3.amazonaws.com/native/blobs/a"}`,
		`{"url":"http://192.168.5.2:3271/native-artifacts/a","expiresAt":1700000000000}`,
		`{"path":"/opt/demi/artifacts/abc/demi-commands"}`,
	} {
		if !valid(accepted) {
			t.Errorf("%s was refused", accepted)
		}
	}
	for _, refused := range []string{
		`{"url":"https://user:secret@demi-native.s3.amazonaws.com/native/blobs/a"}`,
		`{"url":"https://user@demi-native.s3.amazonaws.com/native/blobs/a"}`,
		`{"url":"ftp://192.168.5.2/native-artifacts/a"}`,
		`{"url":"file:///native-artifacts/a"}`,
		`{"url":"native-artifacts/a"}`,
		`{"url":"https://"}`,
		`{"url":"https://host/%zz"}`,
		`{"url":""}`,
		`{"path":""}`,
		`{}`,
		`{"url":"https://host/a","path":"/a"}`,
		`{"url":"https://host/a","expiresAt":null}`,
	} {
		if valid(refused) {
			t.Errorf("%s was accepted", refused)
		}
	}
}

func invocationDocument() map[string]any {
	return map[string]any{
		"operation":    "read",
		"invocationId": "call",
		"context": map[string]any{
			"conversation": "conversation",
			"caller":       map[string]any{"kind": "agent", "number": 1},
			"locale":       map[string]any{"timeZone": "UTC", "languages": []any{"en-US"}},
		},
		"args": map[string]any{},
		"cwd":  "/work",
		"env":  map[string]any{},
	}
}

func decodeInvocation(t *testing.T, document map[string]any) error {
	t.Helper()
	_, err := commandservice.Decode[commandservice.Invocation](encoded(t, document))
	return err
}

func TestInvocationDecodingChecksNestedValuesAndOptionalNulls(t *testing.T) {
	valid := invocationDocument()
	if err := decodeInvocation(t, valid); err != nil {
		t.Fatal(err)
	}
	if err := decodeInvocation(t, mutate(t, valid, map[string]any{"kind": "user"}, "context", "caller")); err != nil {
		t.Errorf("a user caller: %v", err)
	}
	withEdits := mutate(t, valid, map[string]any{"directory": "/edits", "lock": "/edits.lock"}, "edits")
	if err := decodeInvocation(t, withEdits); err != nil {
		t.Errorf("edits: %v", err)
	}
	if err := decodeInvocation(t, mutate(t, valid, true, "json")); err != nil {
		t.Errorf("json: %v", err)
	}
	for name, invalid := range map[string]map[string]any{
		"empty operation":      mutate(t, valid, "", "operation"),
		"empty id":             mutate(t, valid, "", "invocationId"),
		"empty conversation":   mutate(t, valid, "", "context", "conversation"),
		"conversation chars":   mutate(t, valid, "a/b", "context", "conversation"),
		"agent without number": mutate(t, valid, map[string]any{"kind": "agent"}, "context", "caller"),
		"negative number":      mutate(t, valid, map[string]any{"kind": "agent", "number": -1}, "context", "caller"),
		"user with a number":   mutate(t, valid, map[string]any{"kind": "user", "number": 1}, "context", "caller"),
		"unknown caller":       mutate(t, valid, map[string]any{"kind": "system"}, "context", "caller"),
		"no languages":         mutate(t, valid, []any{}, "context", "locale", "languages"),
		"empty time zone":      mutate(t, valid, "", "context", "locale", "timeZone"),
		"empty language":       mutate(t, valid, []any{""}, "context", "locale", "languages"),
		"missing locale":       mutate(t, valid, nil, "context", "locale"),
		"missing context":      mutate(t, valid, nil, "context"),
		"args not an object":   mutate(t, valid, []any{}, "args"),
		"null args":            mutate(t, valid, nil, "args"),
		"NUL in cwd":           mutate(t, valid, "bad\x00path", "cwd"),
		"empty cwd":            mutate(t, valid, "", "cwd"),
		"env name with =":      mutate(t, valid, map[string]any{"A=B": "value"}, "env"),
		"env name empty":       mutate(t, valid, map[string]any{"": "value"}, "env"),
		"NUL in env value":     mutate(t, valid, map[string]any{"A": "bad\x00value"}, "env"),
		"relative edits":       mutate(t, valid, map[string]any{"directory": "edits", "lock": "/edits.lock"}, "edits"),
		"edits without paths":  mutate(t, valid, map[string]any{}, "edits"),
		"an old grant":         mutate(t, valid, map[string]any{"id": "old", "kind": "browser"}, "resource"),
	} {
		if err := decodeInvocation(t, invalid); err == nil {
			t.Errorf("%s: the invocation was accepted", name)
		}
	}
	// null where a value is optional is refused, as a missing required member is.
	for _, raw := range []string{
		strings.Replace(string(encoded(t, valid)), `"env":{}`, `"env":{},"edits":null`, 1),
		strings.Replace(string(encoded(t, valid)), `"env":{}`, `"env":{},"json":null`, 1),
		strings.Replace(string(encoded(t, valid)), `"env":{}`, `"env":null`, 1),
	} {
		if _, err := commandservice.Decode[commandservice.Invocation]([]byte(raw)); err == nil {
			t.Errorf("%s was accepted", raw)
		}
	}
}

// The wire's limits on the size of names and lists, at their edges: the value
// the wire allows is accepted and one more is refused.
func TestTheLimitsOfTheCommandContextAreExact(t *testing.T) {
	valid := invocationDocument()
	languages := func(count int) []any {
		tags := make([]any, count)
		for i := range tags {
			tags[i] = "en-US"
		}
		return tags
	}
	for name, test := range map[string]struct {
		within, over map[string]any
	}{
		"the conversation's name in characters": {
			mutate(t, valid, strings.Repeat("a", 64), "context", "conversation"),
			mutate(t, valid, strings.Repeat("a", 65), "context", "conversation"),
		},
		"the languages": {
			mutate(t, valid, languages(16), "context", "locale", "languages"),
			mutate(t, valid, languages(17), "context", "locale", "languages"),
		},
		"a language tag in characters": {
			mutate(t, valid, []any{strings.Repeat("l", 64)}, "context", "locale", "languages"),
			mutate(t, valid, []any{strings.Repeat("l", 65)}, "context", "locale", "languages"),
		},
		"the time zone in characters": {
			mutate(t, valid, strings.Repeat("z", 64), "context", "locale", "timeZone"),
			mutate(t, valid, strings.Repeat("z", 65), "context", "locale", "timeZone"),
		},
	} {
		if err := decodeInvocation(t, test.within); err != nil {
			t.Errorf("%s, at its limit: %v", name, err)
		}
		if err := decodeInvocation(t, test.over); err == nil {
			t.Errorf("%s, over its limit, was accepted", name)
		}
	}
}

// The limits of a job's edit record: 64 MiB copied, 500 files, and 1000 segments
// in all and in one file.
func TestTheLimitsOfAnEditJournalAreExact(t *testing.T) {
	file := func(segments int) map[string]any {
		edits := make([]any, segments)
		for i := range edits {
			edits[i] = map[string]any{}
		}
		return map[string]any{"path": "/a", "kind": "added", "edits": edits}
	}
	journal := func(files []any, bytesCopied, nextSegment uint64) map[string]any {
		return map[string]any{"files": files, "bytesCopied": bytesCopied, "nextSegment": nextSegment, "filesTruncated": false}
	}
	files := func(count int) []any {
		list := make([]any, count)
		for i := range list {
			list[i] = file(0)
		}
		return list
	}
	for name, test := range map[string]struct {
		within, over map[string]any
	}{
		"the bytes copied": {
			journal(nil, 64*1024*1024, 0),
			journal(nil, 64*1024*1024+1, 0),
		},
		"the next segment": {
			journal(nil, 0, 1000),
			journal(nil, 0, 1001),
		},
		"the files": {
			journal(files(500), 0, 0),
			journal(files(501), 0, 0),
		},
		"the segments of a file": {
			journal([]any{file(1000)}, 0, 0),
			journal([]any{file(1001)}, 0, 0),
		},
	} {
		if _, err := commandservice.Decode[commandservice.EditJournal](encoded(t, test.within)); err != nil {
			t.Errorf("%s, at its limit: %v", name, err)
		}
		if _, err := commandservice.Decode[commandservice.EditJournal](encoded(t, test.over)); err == nil {
			t.Errorf("%s, over its limit, was accepted", name)
		}
	}
}

// A refusal says which field broke which rule, and never quotes the value:
// values can be secrets (an environment variable), and errors are logged.
func TestARefusalNamesTheFieldAndTheRuleAndNeverTheValue(t *testing.T) {
	const secret = "s3cr3t-token"
	valid := invocationDocument()
	decodeCompletion := func(document string) error {
		_, err := commandservice.Decode[commandservice.Completion]([]byte(document))
		return err
	}
	for name, test := range map[string]struct {
		err         error
		field, rule string
		value       string
	}{
		"an environment value with a NUL":   {decodeInvocation(t, mutate(t, valid, map[string]any{"TOKEN": secret + "\x00"}, "env")), "env", "pattern", secret},
		"a working directory with a NUL":    {decodeInvocation(t, mutate(t, valid, secret+"\x00", "cwd")), "cwd", "pattern", secret},
		"arguments that are not an object":  {decodeInvocation(t, mutate(t, valid, secret, "args")), "args", "type", secret},
		"a caller of another kind":          {decodeInvocation(t, mutate(t, valid, map[string]any{"kind": secret}, "context", "caller")), "kind", "enum", secret},
		"a time zone over its length":       {decodeInvocation(t, mutate(t, valid, strings.Repeat(secret, 6), "context", "locale", "timeZone")), "timeZone", "maxLength", secret},
		"an agent without its number":       {decodeInvocation(t, mutate(t, valid, map[string]any{"kind": "agent"}, "context", "caller")), "caller", "required", ""},
		"a user with a number":              {decodeInvocation(t, mutate(t, valid, map[string]any{"kind": "user", "number": 4711}, "context", "caller")), "number", "not", "4711"},
		"an exit code over the limit":       {decodeCompletion(`{"exitCode":31337}`), "exitCode", "maximum", "31337"},
		"a completion that is not JSON":     {decodeCompletion(secret), "root", "not valid JSON", secret},
		"a member that is not the expected": {decodeCompletion(`{"exitCode":"` + secret + `"}`), "exitCode", "type", secret},
	} {
		if test.err == nil {
			t.Errorf("%s was accepted", name)
			continue
		}
		message := test.err.Error()
		if !strings.Contains(message, test.field) || !strings.Contains(message, test.rule) {
			t.Errorf("%s: %q does not name the field %q and the rule %q", name, message, test.field, test.rule)
		}
		if test.value != "" && strings.Contains(message, test.value) {
			t.Errorf("%s: %q quotes the value", name, message)
		}
	}
}

func TestCompletionRequiresItsExitCode(t *testing.T) {
	decode := func(document string) error {
		_, err := commandservice.Decode[commandservice.Completion]([]byte(document))
		return err
	}
	for _, accepted := range []string{`{"exitCode":0}`, `{"exitCode":255}`, `{"exitCode":1,"error":{"code":"c","message":"m"}}`} {
		if err := decode(accepted); err != nil {
			t.Errorf("%s: %v", accepted, err)
		}
	}
	for _, refused := range []string{`{}`, `{"exitCode":-1}`, `{"exitCode":256}`, `{"exitCode":1.5}`, `{"exitCode":0,"error":null}`, `{"exitCode":1,"error":{"code":"c"}}`, `[]`, `null`, ``} {
		if err := decode(refused); err == nil {
			t.Errorf("%s was accepted", refused)
		}
	}
}

// The wire's documents are at most 256 KiB, checked before anything else of
// them is: a document of exactly that size is accepted, one byte more is not.
func TestADocumentOverTheSizeLimitIsRefusedBeforeItIsChecked(t *testing.T) {
	const limit = 256 * 1024
	// The document is {"protocolVersion":1,"operations":["<name>"]}: the name
	// pads it to the size wanted.
	overhead := len(`{"protocolVersion":1,"operations":[""]}`)
	document := func(size int) []byte {
		return []byte(`{"protocolVersion":1,"operations":["` + strings.Repeat("a", size-overhead) + `"]}`)
	}
	if _, err := commandservice.Decode[commandservice.ServiceInfo](document(limit)); err != nil {
		t.Errorf("a document of %d bytes: %v", limit, err)
	}
	if _, err := commandservice.Decode[commandservice.ServiceInfo](document(limit + 1)); !errors.Is(err, commandservice.ErrTooLarge) {
		t.Errorf("a document of %d bytes: error = %v, want ErrTooLarge", limit+1, err)
	}
	within := commandservice.ServiceInfo{ProtocolVersion: 1, Operations: []string{strings.Repeat("a", limit-overhead)}}
	if _, err := commandservice.Encode(within); err != nil {
		t.Errorf("encoding a document of %d bytes: %v", limit, err)
	}
	over := commandservice.ServiceInfo{ProtocolVersion: 1, Operations: []string{strings.Repeat("a", limit-overhead+1)}}
	if _, err := commandservice.Encode(over); !errors.Is(err, commandservice.ErrTooLarge) {
		t.Errorf("encoding a document of %d bytes: error = %v, want ErrTooLarge", limit+1, err)
	}
}

func TestConversationValues(t *testing.T) {
	decodeRequest := func(document string) error {
		_, err := commandservice.Decode[commandservice.ConversationRequest]([]byte(document))
		return err
	}
	for _, accepted := range []string{`{"operation":"status"}`, `{"operation":"release","conversation":"one"}`} {
		if err := decodeRequest(accepted); err != nil {
			t.Errorf("%s: %v", accepted, err)
		}
	}
	for _, refused := range []string{
		`{"operation":"release"}`,
		`{"operation":"release","conversation":""}`,
		`{"operation":"acquire","conversation":"one"}`,
		`{"operation":"status","resource":"one"}`,
		`{"operation":"status","conversation":"one"}`,
	} {
		if err := decodeRequest(refused); err == nil {
			t.Errorf("%s was accepted", refused)
		}
	}
	decodeStatus := func(conversations ...string) error {
		_, err := commandservice.Encode(commandservice.ConversationStatus{Conversations: conversations})
		return err
	}
	if err := decodeStatus(); err != nil {
		t.Errorf("an empty status: %v", err)
	}
	if err := decodeStatus("conversation"); err != nil {
		t.Errorf("a status: %v", err)
	}
	if err := decodeStatus(""); err == nil {
		t.Error("a status naming no conversation was accepted")
	}
}

func TestNumbersValues(t *testing.T) {
	decodeRequest := func(document string) error {
		_, err := commandservice.Decode[commandservice.NumbersRequest]([]byte(document))
		return err
	}
	for _, accepted := range []string{
		`{"id":3,"conversation":"c1","sequence":"tab","count":1}`,
		`{"id":3,"conversation":"c1","sequence":"tab","count":16}`,
	} {
		if err := decodeRequest(accepted); err != nil {
			t.Errorf("%s: %v", accepted, err)
		}
	}
	for _, refused := range []string{
		`{"id":3,"conversation":"c1","sequence":"tab","count":0}`,
		`{"id":3,"conversation":"c1","sequence":"tab","count":17}`,
		`{"id":3,"conversation":"c1","sequence":"window","count":1}`,
		`{"id":3,"conversation":"","sequence":"tab","count":1}`,
		`{"id":-1,"conversation":"c1","sequence":"tab","count":1}`,
		`{"conversation":"c1","sequence":"tab","count":1}`,
	} {
		if err := decodeRequest(refused); err == nil {
			t.Errorf("%s was accepted", refused)
		}
	}
	decodeAnswer := func(document string) error {
		_, err := commandservice.Decode[commandservice.NumbersAnswer]([]byte(document))
		return err
	}
	for _, accepted := range []string{`{"id":3,"first":9}`, `{"id":3,"error":"refused"}`} {
		if err := decodeAnswer(accepted); err != nil {
			t.Errorf("%s: %v", accepted, err)
		}
	}
	for _, refused := range []string{`{"id":3}`, `{"id":3,"first":9,"error":"both"}`, `{"id":3,"first":0}`, `{"id":3,"error":""}`, `{"id":3,"first":null,"error":"x"}`} {
		if err := decodeAnswer(refused); err == nil {
			t.Errorf("%s was accepted", refused)
		}
	}
}
