package skills_test

import (
	"context"
	"errors"
	"strings"
	"sync/atomic"
	"testing"

	"github.com/go-git/go-git/v5/plumbing/filemode"

	"github.com/wspl/demi/internal/plugin"
	"github.com/wspl/demi/internal/plugin/plugintest"
	"github.com/wspl/demi/internal/plugin/skills"
	"github.com/wspl/demi/internal/plugin/skills/skillstest"
)

// These additional boundary scenarios run locally without sleeps or processes.
func TestRemoveDuringFetchAndIgnoreConcurrentUpdate(t *testing.T) {
	repos := skillstest.New(t)
	toolsRepository(t, repos)
	entered, release := make(chan struct{}), make(chan struct{})
	var uploads atomic.Int32
	repos.BeforeUpload = func(ctx context.Context, _ string) error {
		if uploads.Add(1) == 1 {
			close(entered)
		}
		select {
		case <-release:
			return nil
		case <-ctx.Done():
			return ctx.Err()
		}
	}
	p := newPlugged(t, repos)
	before := p.demi.Changes()
	raw, err := p.method(t.Context(), "add_source", skills.AddSource{Origin: "acme/tools"})
	if err != nil {
		t.Fatal(err)
	}
	added, err := skills.DecodeAddedSource(raw)
	if err != nil {
		t.Fatal(err)
	}
	<-entered
	state := p.state(t)
	if !state.Sources[0].Fetching {
		t.Fatal("fetch is not visible on page")
	}
	if _, err := p.method(t.Context(), "update_source", skills.SourceCall(added)); err != nil {
		t.Fatal(err)
	}
	if _, err := p.method(t.Context(), "remove_source", skills.SourceCall(added)); err != nil {
		t.Fatal(err)
	}
	close(release)
	if err := p.demi.Until(
		t.Context(),
		func(d *plugintest.TestDemi) bool { return d.Changes() >= before+4 },
	); err != nil {
		t.Fatal(err)
	}
	if uploads.Load() != 1 {
		t.Fatal("update started a second fetch")
	}
	sources := len(p.state(t).Sources)
	_, found := p.demi.Value(added.Source)
	if sources != 0 || found || len(p.demi.Directories()) != 0 {
		t.Fatal("fetch resurrected removed source")
	}
	_, err = p.method(t.Context(), "remove_source", skills.SourceCall(added))
	requireRefusal(t, err, "source_not_found")
}

func TestSourceExtractionBoundaries(t *testing.T) {
	repos := skillstest.New(t)
	commit(t, repos, "acme/root",
		skillFile("SKILL.md", "description: Root skill."),
		skillFile("nested/SKILL.md", "description: Nested skill."),
		skillFile("broken/SKILL.md", "name: broken"),
		skillstest.File{Path: "broken/private.txt", Bytes: []byte("not owned by root")},
		skillstest.File{Path: "script", Bytes: []byte("echo ok"), Executable: true},
		skillstest.File{Path: "link", Bytes: []byte("/outside"), Mode: filemode.Symlink},
		skillstest.File{Path: "linked/SKILL.md", Bytes: []byte("../SKILL.md"), Mode: filemode.Symlink})
	p := newPlugged(t, repos)
	id := p.add(t, "acme/root")
	state := p.state(t).Sources[0]
	if len(state.Skills) != 2 || state.Skills[0].Name != "root" || state.Skills[1].Name != "nested" ||
		len(state.Skipped) != 1 {
		t.Fatalf("extraction: %+v", state)
	}
	if _, err := p.method(
		t.Context(),
		"set_source_enabled",
		skills.SetSourceEnabled{Source: id, Enabled: true},
	); err != nil {
		t.Fatal(err)
	}
	for _, directory := range p.demi.Directories() {
		for _, file := range directory.Files {
			if strings.Contains(file.Path, "broken") || strings.Contains(file.Path, "link") ||
				strings.Contains(file.Path, "nested/") {
				t.Fatalf("nested or linked file leaked: %+v", file)
			}
		}
	}
}

func TestSourceSkillAndFileBudgets(t *testing.T) {
	repos := skillstest.New(t)
	files := make([]skillstest.File, 101)
	for i := range files {
		name := strings.Repeat("a", i+1)
		files[i] = skillFile(name+"/SKILL.md", "description: Too many.")
	}
	commit(t, repos, "acme/many", files...)
	commit(
		t,
		repos,
		"acme/files",
		skillFile("SKILL.md", "description: Too many bytes."),
		skillstest.File{Path: "content", Bytes: make([]byte, 16*1024*1024)},
	)
	p := newPlugged(t, repos)
	p.add(t, "acme/many")
	p.add(t, "acme/files")
	for i, want := range []string{"the repository has more than 100 skills", "the skills' files hold more than 16 MiB"} {
		source := p.state(t).Sources[i]
		if source.Failure == nil || source.Failure.Message != want || source.Commit != nil || len(source.Skills) != 0 {
			t.Fatalf("budget: %+v", source)
		}
	}
}

func TestManifestAndRequestContracts(t *testing.T) {
	factory, err := skills.New()
	if err != nil {
		t.Fatal(err)
	}
	manifest := factory.Manifest()
	empty, err := (skills.SkillsState{Sources: []skills.SourceState{}}).MarshalJSON()
	if err != nil {
		t.Fatal(err)
	}
	if err := manifest.Page.User.Schema.Check(empty); err != nil {
		t.Fatal(err)
	}
	args, err := (skills.AddSource{Origin: "acme/tools"}).MarshalJSON()
	if err != nil {
		t.Fatal(err)
	}
	if err := manifest.Page.Methods[0].Params.Check(args); err != nil {
		t.Fatal(err)
	}
	p := newPlugged(t, skillstest.New(t))
	_, err = p.method(t.Context(), "does_not_exist", skills.SourceCall{Source: "none"})
	var failed *plugin.ErrorFailed
	if !errors.As(err, &failed) || failed.Message != `no method "does_not_exist"` {
		t.Fatalf("undeclared method: %v", err)
	}
	_, err = p.call.Call(
		t.Context(),
		&plugin.RequestPageCall{
			User:   "aaaaaaaaaaaaaaaaaaaaaaaaaa",
			Method: "add_source",
			Params: []byte(`{"origin":null}`),
		},
		p.demi.Port(),
	)
	var usage *plugin.ErrorUsage
	if !errors.As(err, &usage) {
		t.Fatalf("page input: %v", err)
	}
}

func TestProjectDepthAndHiddenSkills(t *testing.T) {
	p := newPlugged(t, skillstest.New(t))
	p.demi.HostFiles = map[string][]byte{
		"/repo/.agents/skills/group/one/two/three/four/last/SKILL.md": []byte(
			skillstest.SkillMD("name: last\ndescription: At depth six."),
		),
		"/repo/.agents/skills/group/one/two/three/four/extra/too-deep/SKILL.md": []byte(
			skillstest.SkillMD("description: At depth seven."),
		),
		"/repo/.agents/skills/outer/SKILL.md": []byte(
			skillstest.SkillMD("description: Outer."),
		),
		"/repo/.agents/skills/outer/inner/SKILL.md": []byte(
			skillstest.SkillMD("description: Do not descend into skills."),
		),
		"/repo/.agents/skills/hidden/SKILL.md": []byte(
			skillstest.SkillMD("description: Hidden.\ndisable-model-invocation: true"),
		),
		"/repo/.agents/skills/same/SKILL.md": []byte(
			skillstest.SkillMD("description: Agents precedence."),
		),
		"/repo/.claude/skills/same/SKILL.md": []byte(
			skillstest.SkillMD("description: Claude shadowed."),
		),
	}
	block := p.context(t, "/repo", "t1", nil)
	if block == nil {
		t.Fatal("no project catalog")
	}
	for _, want := range []string{"At depth six.", "Outer.", "Agents precedence."} {
		if !strings.Contains(*block, want) {
			t.Fatal(*block)
		}
	}
	for _, absent := range []string{"At depth seven.", "Do not descend", "Hidden.", "Claude shadowed."} {
		if strings.Contains(*block, absent) {
			t.Fatal(*block)
		}
	}
	// A stopped Host on the next turn keeps the last successful search.
	p.demi.HostFiles = nil
	if got := p.context(t, "/repo", "t2", nil); got == nil || *got != *block {
		t.Fatal("stopped Host lost cached skills")
	}
}
