package skills

import (
	"errors"
	"strings"
	"testing"

	"go.uber.org/goleak"

	"github.com/wspl/demi/internal/plugin"
)

func TestMain(m *testing.M) { goleak.VerifyTestMain(m) }

// All scenarios are in memory and are expected to finish in under one second.
func TestSourceOrigins(t *testing.T) {
	var id string
	for _, text := range []string{"acme/tools", " acme/tools.git ", "https://github.com/acme/tools.git/", "https://GITHUB.COM/acme/tools/"} {
		parsed, err := parseOrigin(text)
		if err != nil {
			t.Fatal(err)
		}
		if id == "" {
			id = parsed.id()
		}
		if parsed.id() != id || parsed.url != "https://github.com/acme/tools" || parsed.repository() != "tools" {
			t.Fatalf("origin %q: %+v", text, parsed)
		}
	}
	for _, text := range []string{"not a repository", "http://example.test/a", "git@example.test:a", "https://example.test/", "a/b/c"} {
		if _, err := parseOrigin(text); err == nil {
			t.Fatalf("accepted %q", text)
		}
	}
}

func TestCatalogVisibleHistory(t *testing.T) {
	if nextCatalog(nil, nil) != nil {
		t.Fatal("empty catalog introduced unnecessarily")
	}
	entries := []catalogEntry{{name: "z", description: "Unicode 雪 & < > \" '", location: "/z"}, {name: "a", description: "First", location: "/a"}}
	text := nextCatalog(entries, nil)
	if text == nil || !strings.Contains(*text, "Unicode 雪 &amp; &lt; &gt; &quot; &apos;") || strings.Index(*text, "<name>a</name>") > strings.Index(*text, "<name>z</name>") {
		t.Fatalf("bad catalog: %v", text)
	}
	if nextCatalog(entries, []string{*text}) != nil {
		t.Fatal("unchanged catalog repeated")
	}
	if got := nextCatalog(entries, nil); got == nil || *got != *text {
		t.Fatal("compaction did not restore catalog")
	}
	if got := nextCatalog(nil, []string{*text}); got == nil || *got != noneAvailable {
		t.Fatal("model did not learn skills were removed")
	}
	if nextCatalog(nil, []string{noneAvailable}) != nil {
		t.Fatal("empty notice repeated")
	}
}

func TestTakenSkillNames(t *testing.T) {
	all := sources{
		"tools": {source: source{Origin: "acme/tools", Skills: []userSkill{{Name: "review", Enabled: true}, {Name: "Bad_Name", Enabled: true}}}},
		"more":  {source: source{Origin: "acme/more", Skills: []userSkill{{Name: "review"}, {Name: "bad-name"}}}},
	}
	for _, name := range []string{"review", "bad-name"} {
		_, err := switchSkills(all, "more", map[string]bool{name: true}, true)
		var refused *plugin.ErrorRefused
		if !errors.As(err, &refused) || refused.Reason != "skill_name_taken" || !strings.Contains(refused.Message, "acme/tools") {
			t.Fatalf("collision: %v", err)
		}
	}
	for _, test := range []struct{ id, name, reason string }{{"more", "nope", "skill_not_found"}, {"missing", "review", "source_not_found"}} {
		_, err := switchSkills(all, test.id, map[string]bool{test.name: true}, true)
		var refused *plugin.ErrorRefused
		if !errors.As(err, &refused) || refused.Reason != test.reason {
			t.Fatalf("missing: %v", err)
		}
	}
	changed, err := switchSkills(all, "tools", map[string]bool{"review": true}, false)
	if err != nil || changed.Skills[0].Enabled || !all["tools"].source.Skills[0].Enabled {
		t.Fatalf("switch mutated original or failed: %v", err)
	}
	duplicate := sources{"a": {source: source{Origin: "acme/a", Skills: []userSkill{{Name: "Bad_Name"}, {Name: "bad-name"}}}}}
	if _, err := switchSkills(duplicate, "a", map[string]bool{"Bad_Name": true, "bad-name": true}, true); err == nil {
		t.Fatal("batch accepted colliding names")
	}
}

func TestDirectoryNames(t *testing.T) {
	for name, want := range map[string]string{"review": "review", "Bad_Name": "bad-name", "  a---B  ": "a-b", "雪": "skill", strings.Repeat("A", 63) + " B": strings.Repeat("a", 63)} {
		if got := directoryName(name); got != want {
			t.Errorf("%q: got %q, want %q", name, got, want)
		}
	}
}

func TestContractEntryValidation(t *testing.T) {
	for _, input := range []string{`{}`, `{"origin":null}`, `{"origin":"acme/tools","extra":1}`} {
		if _, err := DecodeAddSource([]byte(input)); err == nil {
			t.Fatalf("invalid page input accepted: %s", input)
		}
	}
	if _, err := decodeSource([]byte(`{"origin":"acme/tools","added":1,"skills":[],"skipped":[],"extra":1}`)); err == nil {
		t.Fatal("unknown stored field accepted")
	}
	value, err := decodeSource([]byte(`{"origin":"acme/<tools>&\u2028\u2029","added":1,"skills":[],"skipped":[],"commit":null}`))
	if err != nil {
		t.Fatal(err)
	}
	encoded, err := value.MarshalJSON()
	if err != nil {
		t.Fatal(err)
	}
	want := "{\"origin\":\"acme/<tools>&\u2028\u2029\",\"added\":1,\"skills\":[],\"skipped\":[]}"
	if string(encoded) != want {
		t.Fatalf("wire bytes: %s", encoded)
	}
}
