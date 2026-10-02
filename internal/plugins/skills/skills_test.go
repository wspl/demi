package skills

import (
	"errors"
	"fmt"
	"strings"
	"testing"
	"unicode/utf8"

	"github.com/google/go-cmp/cmp"
	"go.uber.org/goleak"

	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/plugin"
	"github.com/wspl/demi/internal/plugin/plugintest"
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

func TestCatalogBudget(t *testing.T) {
	entries := make([]catalogEntry, 30)
	for i := range entries {
		name := fmt.Sprintf("skill-%02d", i)
		entries[i] = catalogEntry{name: name, description: strings.Repeat("word ", 60), location: "/repo/.agents/skills/" + name + "/SKILL.md"}
	}
	block := renderCatalog(entries)
	if utf8.RuneCountInString(block) > 8000 || strings.Count(block, "<skill>") != 30 || strings.Count(block, "…</description>") != 30 {
		t.Fatalf("catalog did not shorten all descriptions: %s", block)
	}
	length := -1
	for _, rest := range strings.Split(block, "<description>")[1:] {
		text, _, _ := strings.Cut(rest, "</description>")
		if length >= 0 && utf8.RuneCountInString(text) != length {
			t.Fatal("descriptions do not share a length")
		}
		length = utf8.RuneCountInString(text)
	}
	entries = make([]catalogEntry, 100)
	for i := range entries {
		name := fmt.Sprintf("%s%02d", strings.Repeat("a", 60), i)
		entries[i] = catalogEntry{name: name, description: "Short.", location: "/repo/.agents/skills/" + name + "/SKILL.md"}
	}
	block = renderCatalog(entries)
	shown := strings.Count(block, "<skill>")
	if utf8.RuneCountInString(block) > 8000 || strings.Contains(block, "<description>") || !strings.HasSuffix(block, fmt.Sprintf("%d more skills are not listed.", 100-shown)) {
		t.Fatalf("catalog did not count omitted skills: %s", block)
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

func TestUpdateRetainsEnabledNames(t *testing.T) {
	before := source{Origin: "acme/tools", Added: 1, Skills: []userSkill{{Name: "review", Enabled: true}, {Name: "lint", Enabled: true}}, Failure: &Failure{At: core.UnixEpoch, Message: "failed"}}
	after := pinSource(before, "second", []userSkill{{Name: "review", Description: "Review a change, carefully."}, {Name: "format"}}, []Skipped{}, core.UnixEpoch)
	if !after.Skills[0].Enabled || after.Skills[1].Enabled || after.Skills[1].Name != "format" || len(after.Skills) != 2 || after.Failure != nil || *after.Commit != "second" || after.Origin != before.Origin || after.Added != before.Added {
		t.Fatalf("bad update: %+v", after)
	}
	if before.Skills[1].Name != "lint" || before.Failure == nil {
		t.Fatal("update mutated previous source")
	}
}

func TestStoredSkillsAndDirectories(t *testing.T) {
	demi := plugintest.New()
	demi.Plugin = "skills"
	port := demi.Port()
	manifest, err := port.PutBlob(t.Context(), core.B64Bytes("---\nname: review\ndescription: Review a change.\n---\n"))
	if err != nil {
		t.Fatal(err)
	}
	script, err := port.PutBlob(t.Context(), core.B64Bytes("#!/bin/sh\necho ok\n"))
	if err != nil {
		t.Fatal(err)
	}
	skill := userSkill{Name: "review", Description: "Review a change.", Files: []plugin.DirectoryFile{{Path: "SKILL.md", Blob: manifest}, {Path: "scripts/check.sh", Executable: true, Blob: script}}, Warnings: []string{}}
	value := source{Origin: "acme/tools", Added: 2, Skills: []userSkill{skill}, Skipped: []Skipped{{Path: "broken/SKILL.md", Reason: "the front matter has no description"}}}
	revision, err := writeSource(t.Context(), port, "tools", value, nil)
	if err != nil {
		t.Fatal(err)
	}
	if len(demi.ValueBlobs("tools")) != 2 || demi.BlobBytes(script) == nil {
		t.Fatal("source did not retain file blobs")
	}
	all, err := readSources(t.Context(), port)
	if err != nil {
		t.Fatal(err)
	}
	if len(sourceDirectories(all)) != 0 {
		t.Fatal("off skill installed")
	}
	changed, err := switchSkills(all, "tools", map[string]bool{"review": true}, true)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := writeSource(t.Context(), port, "tools", changed, &revision); err != nil {
		t.Fatal(err)
	}
	all, err = readSources(t.Context(), port)
	if err != nil {
		t.Fatal(err)
	}
	directories := sourceDirectories(all)
	if len(directories) != 1 || !directories[0].Files[1].Executable {
		t.Fatal("enabled executable not installed")
	}
	paths, err := port.SetDirectories(t.Context(), directories)
	if err != nil {
		t.Fatal(err)
	}
	if skillLocation(skill) != paths[0].Path+"/SKILL.md" {
		t.Fatal("catalog path differs from Host path")
	}
	all["earlier"] = storedSource{source: source{Origin: "acme/earlier", Added: 1, Skills: []userSkill{}, Skipped: []Skipped{}}}
	state := pageState(all, map[string]bool{"tools": true})
	if state.Sources[0].ID != "earlier" || !state.Sources[1].Fetching || !state.Sources[1].Skills[0].Enabled || len(state.Sources[1].Skipped) != 1 {
		t.Fatalf("bad page state: %+v", state)
	}
	encoded, err := state.MarshalJSON()
	if err != nil {
		t.Fatal(err)
	}
	decoded, err := DecodeState(encoded)
	if err != nil {
		t.Fatal(err)
	}
	if diff := cmp.Diff(state, decoded); diff != "" {
		t.Fatal(diff)
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
