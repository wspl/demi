package skills_test

import (
	"context"
	"crypto/sha256"
	"encoding/json"
	"errors"
	"fmt"
	"slices"
	"strings"
	"sync"
	"testing"
	"unicode/utf8"

	"github.com/google/go-cmp/cmp"

	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/plugin"
	"github.com/wspl/demi/internal/plugin/plugintest"
	"github.com/wspl/demi/internal/plugins/skills"
	"github.com/wspl/demi/internal/plugins/skills/skillstest"
)

type plugged struct {
	instance plugin.Plugin
	call     plugin.Plugin
	demi     *plugintest.TestDemi
}

func newPlugged(t *testing.T, repos *skillstest.Repos) *plugged {
	t.Helper()
	factory, err := repos.Factory()
	if err != nil {
		t.Fatal(err)
	}
	instance := factory.Instance()
	closer, ok := instance.(plugin.Closer)
	if !ok {
		t.Fatal("skills instance has no lifetime owner")
	}
	t.Cleanup(closer.Close)
	demi := plugintest.New()
	demi.Plugin = "skills"
	return &plugged{instance: instance, call: plugintest.Loopback(instance), demi: demi}
}

func (p *plugged) method(ctx context.Context, name string, args json.Marshaler) (json.RawMessage, error) {
	params, err := args.MarshalJSON()
	if err != nil {
		return nil, err
	}
	reply, err := p.call.Call(
		ctx,
		&plugin.RequestPageCall{User: "aaaaaaaaaaaaaaaaaaaaaaaaaa", Method: name, Params: params},
		p.demi.Port(),
	)
	if err != nil {
		return nil, err
	}
	result, ok := reply.(*plugin.ReplyResult)
	if !ok {
		return nil, fmt.Errorf("page returned %T", reply)
	}
	return result.Result, nil
}

func (p *plugged) state(t *testing.T) skills.SkillsState {
	t.Helper()
	reply, err := p.call.Call(t.Context(), &plugin.RequestPageState{User: "aaaaaaaaaaaaaaaaaaaaaaaaaa"}, p.demi.Port())
	if err != nil {
		t.Fatal(err)
	}
	result, ok := reply.(*plugin.ReplyState)
	if !ok {
		t.Fatalf("state returned %T", reply)
	}
	state, err := skills.DecodeSkillsState(result.State)
	if err != nil {
		t.Fatal(err)
	}
	return state
}

func (p *plugged) add(t *testing.T, origin string) string {
	t.Helper()
	before := p.demi.Changes()
	raw, err := p.method(t.Context(), "add_source", skills.AddSource{Origin: origin})
	if err != nil {
		t.Fatal(err)
	}
	added, err := skills.DecodeAddedSource(raw)
	if err != nil {
		t.Fatal(err)
	}
	if err := p.demi.Until(
		t.Context(),
		func(d *plugintest.TestDemi) bool { return d.Changes() >= before+3 },
	); err != nil {
		t.Fatal(err)
	}
	return added.Source
}

func (p *plugged) update(t *testing.T, source string) {
	t.Helper()
	before := p.demi.Changes()
	if _, err := p.method(t.Context(), "update_source", skills.SourceCall{Source: source}); err != nil {
		t.Fatal(err)
	}
	if err := p.demi.Until(
		t.Context(),
		func(d *plugintest.TestDemi) bool { return d.Changes() >= before+2 },
	); err != nil {
		t.Fatal(err)
	}
}

func (p *plugged) enable(t *testing.T, source, skill string) {
	t.Helper()
	if _, err := p.method(
		t.Context(),
		"set_enabled",
		skills.SetEnabled{Source: source, Skill: skill, Enabled: true},
	); err != nil {
		t.Fatal(err)
	}
}

func (p *plugged) context(t *testing.T, cwd, turn string, seen []string) *string {
	t.Helper()
	if seen == nil {
		seen = []string{}
	}
	reply, err := p.call.Call(
		t.Context(),
		&plugin.RequestContext{
			User:         "aaaaaaaaaaaaaaaaaaaaaaaaaa",
			Conversation: "0b6f7f3e-8f3a-4c1e-9d2b-7a1c2e3f4a5b",
			Node:         "conversation",
			CWD:          cwd,
			Turn:         core.TurnID(turn),
			Seen:         seen,
		},
		p.demi.Port(),
	)
	if err != nil {
		t.Fatal(err)
	}
	result, ok := reply.(*plugin.ReplyContext)
	if !ok {
		t.Fatalf("context returned %T", reply)
	}
	return result.Text
}

func commit(t *testing.T, repos *skillstest.Repos, name string, files ...skillstest.File) string {
	t.Helper()
	hash, err := repos.Commit(t.Context(), name, files)
	if err != nil {
		t.Fatal(err)
	}
	return hash
}

func skillFile(path, front string) skillstest.File {
	return skillstest.File{Path: path, Bytes: []byte(skillstest.SkillMD(front))}
}

func toolsRepository(t *testing.T, repos *skillstest.Repos) {
	t.Helper()
	commit(
		t,
		repos,
		"acme/tools",
		skillFile("skills/review/SKILL.md", "name: review\ndescription: Review a change."),
		skillstest.File{
			Path:       "skills/review/scripts/check.sh",
			Bytes:      []byte("#!/bin/sh\necho ok\n"),
			Executable: true,
		},
		skillFile("skills/Bad_Name/SKILL.md", "name: Bad_Name\ndescription: Breaks the rule."),
		skillFile("skills/broken/SKILL.md", "name: broken"),
		skillFile(
			"skills/hidden/SKILL.md",
			"name: hidden\ndescription: The user's alone.\ndisable-model-invocation: true",
		),
	)
}

func requireRefusal(t *testing.T, err error, reason string) string {
	t.Helper()
	var refused *plugin.ErrorRefused
	if !errors.As(err, &refused) || refused.Reason != reason {
		t.Fatalf("wanted %s, got %v", reason, err)
	}
	return refused.Message
}

// Tests use loopback JSON and fixture HTTP repositories. Ordinary scenarios
// should complete in under one second; the large-pack scenario states its cost.
func TestAddingSourceListsWarningsSkippedAndBlobs(t *testing.T) {
	repos := skillstest.New(t)
	toolsRepository(t, repos)
	p := newPlugged(t, repos)
	id := p.add(t, "acme/tools")
	state := p.state(t)
	if len(state.Sources) != 1 {
		t.Fatalf("sources: %+v", state)
	}
	listed := state.Sources[0]
	at, err := core.TimestampFromMillisecond(skillstest.FetchedAt)
	if err != nil {
		t.Fatal(err)
	}
	if listed.ID != id || listed.Origin != "acme/tools" || listed.Commit == nil || listed.FetchedAt == nil ||
		*listed.FetchedAt != at ||
		listed.Fetching ||
		listed.Failure != nil {
		t.Fatalf("source: %+v", listed)
	}
	if len(listed.Skills) != 3 || listed.Skills[0].Name != "Bad_Name" || listed.Skills[1].Name != "hidden" ||
		listed.Skills[2].Name != "review" {
		t.Fatalf("skills: %+v", listed.Skills)
	}
	for _, skill := range listed.Skills {
		if skill.Enabled {
			t.Fatal("new skill starts enabled")
		}
	}
	if listed.Skills[0].DisableModelInvocation || listed.Skills[2].DisableModelInvocation ||
		!listed.Skills[1].DisableModelInvocation ||
		len(listed.Skills[0].Warnings) != 1 ||
		!strings.Contains(listed.Skills[0].Warnings[0], "not 1 to 64 lowercase") ||
		len(listed.Skills[2].Warnings) != 0 {
		t.Fatalf("flags/warnings: %+v", listed.Skills)
	}
	if len(listed.Skipped) != 1 || listed.Skipped[0].Path != "skills/broken/SKILL.md" ||
		!strings.Contains(listed.Skipped[0].Reason, "no description") {
		t.Fatalf("skipped: %+v", listed.Skipped)
	}
	script := core.BlobRef(fmt.Sprintf("%x", sha256.Sum256([]byte("#!/bin/sh\necho ok\n"))))
	if !slices.Contains(p.demi.ValueBlobs(id), script) {
		t.Fatal("script digest missing from value blobs")
	}
	found := false
	for _, blob := range p.demi.ValueBlobs(id) {
		if content, ok := p.demi.BlobBytes(blob); ok && string(content) == "#!/bin/sh\necho ok\n" {
			found = true
		}
	}
	if !found {
		t.Fatal("script blob not retained")
	}
	_, err = p.method(t.Context(), "add_source", skills.AddSource{Origin: "https://github.com/acme/tools.git/"})
	requireRefusal(t, err, "source_exists")
	_, err = p.method(t.Context(), "add_source", skills.AddSource{Origin: "not a repository"})
	requireRefusal(t, err, "invalid_origin")
}

func TestEmptyAndOversizedSourcesRecordFailure(t *testing.T) {
	// Several seconds: the 64 MiB bound is exercised by sending an actual pack
	// containing 65 MiB of incompressible bytes, rather than substituting a limit.
	repos := skillstest.New(t)
	commit(t, repos, "acme/empty", skillstest.File{Path: "README.md", Bytes: []byte("nothing here")})
	commit(t, repos, "acme/huge", skillFile("skills/big/SKILL.md", "name: big\ndescription: Too much."))
	if _, err := repos.CommitBytes(t.Context(), "acme/huge", "skills/big/blob.bin", noise(65*1024*1024)); err != nil {
		t.Fatal(err)
	}
	p := newPlugged(t, repos)
	p.add(t, "acme/empty")
	p.add(t, "acme/huge")
	state := p.state(t)
	at, err := core.TimestampFromMillisecond(skillstest.FetchedAt)
	if err != nil {
		t.Fatal(err)
	}
	if len(state.Sources) != 2 {
		t.Fatal(state)
	}
	for i, want := range []string{"the repository holds no skill", "the repository is larger than 64 MiB"} {
		source := state.Sources[i]
		if source.Origin != []string{"acme/empty", "acme/huge"}[i] || source.Commit != nil || len(source.Skills) != 0 ||
			source.Failure == nil ||
			source.Failure.Message != want ||
			source.Failure.At != at {
			t.Fatalf("failure: %+v, want %q", source, want)
		}
	}
}

func TestEnabledSkillDirectoryAndCatalog(t *testing.T) {
	repos := skillstest.New(t)
	toolsRepository(t, repos)
	p := newPlugged(t, repos)
	id := p.add(t, "acme/tools")
	p.enable(t, id, "review")
	p.enable(t, id, "hidden")
	directories := p.demi.Directories()
	index := slices.IndexFunc(directories, func(d plugin.HostDirectory) bool { return d.Name == "review" })
	if index < 0 {
		t.Fatal("missing review directory")
	}
	review := directories[index]
	if len(review.Files) != 2 || review.Files[0].Path != "SKILL.md" || review.Files[0].Executable ||
		review.Files[1].Path != "scripts/check.sh" ||
		!review.Files[1].Executable {
		t.Fatalf("files: %+v", review.Files)
	}
	block := p.context(t, "/home/me/app", "t1", nil)
	if block == nil || !strings.Contains(*block, "<location>"+review.Path("skills")+"/SKILL.md</location>") ||
		!strings.Contains(*block, "<description>Review a change.</description>") ||
		strings.Contains(*block, "hidden") {
		t.Fatalf("catalog: %v", block)
	}
	if p.context(t, "/home/me/app", "t2", []string{*block}) != nil {
		t.Fatal("unchanged catalog repeated")
	}
	if got := p.context(t, "/home/me/app", "t3", nil); got == nil || *got != *block {
		t.Fatal("compaction lost catalog")
	}
	for _, name := range []string{"review", "hidden"} {
		if _, err := p.method(
			t.Context(),
			"set_enabled",
			skills.SetEnabled{Source: id, Skill: name, Enabled: false},
		); err != nil {
			t.Fatal(err)
		}
	}
	if len(p.demi.Directories()) != 0 {
		t.Fatal("disabled directories remain")
	}
	if got := p.context(
		t,
		"/home/me/app",
		"t4",
		[]string{*block},
	); got == nil ||
		*got != "No skills are available now." {
		t.Fatalf("empty catalog: %v", got)
	}
}

func TestSecondEnabledNameRefusedWithSource(t *testing.T) {
	repos := skillstest.New(t)
	toolsRepository(t, repos)
	commit(t, repos, "acme/more", skillFile("review/SKILL.md", "name: review\ndescription: Another review."))
	p := newPlugged(t, repos)
	tools := p.add(t, "acme/tools")
	more := p.add(t, "acme/more")
	p.enable(t, tools, "review")
	_, err := p.method(t.Context(), "set_enabled", skills.SetEnabled{Source: more, Skill: "review", Enabled: true})
	if message := requireRefusal(t, err, "skill_name_taken"); !strings.Contains(message, "acme/tools") {
		t.Fatal(message)
	}
	_, err = p.method(t.Context(), "set_source_enabled", skills.SetSourceEnabled{Source: more, Enabled: true})
	requireRefusal(t, err, "skill_name_taken")
	_, err = p.method(t.Context(), "set_enabled", skills.SetEnabled{Source: more, Skill: "nope", Enabled: true})
	requireRefusal(t, err, "skill_not_found")
}

func TestUpdateRetainsNamesAndFailedPin(t *testing.T) {
	repos := skillstest.New(t)
	commit(
		t,
		repos,
		"acme/tools",
		skillFile("review/SKILL.md", "name: review\ndescription: Review a change."),
		skillFile("lint/SKILL.md", "name: lint\ndescription: Lint it."),
	)
	p := newPlugged(t, repos)
	id := p.add(t, "acme/tools")
	p.enable(t, id, "review")
	p.enable(t, id, "lint")
	first := *p.state(t).Sources[0].Commit
	commit(
		t,
		repos,
		"acme/tools",
		skillFile("review/SKILL.md", "name: review\ndescription: Review a change, carefully."),
		skillFile("format/SKILL.md", "name: format\ndescription: Format it."),
	)
	p.update(t, id)
	listed := p.state(t).Sources[0]
	if listed.Commit == nil || *listed.Commit == first || len(listed.Skills) != 2 ||
		listed.Skills[0].Name != "format" ||
		listed.Skills[0].Enabled ||
		listed.Skills[1].Name != "review" ||
		!listed.Skills[1].Enabled {
		t.Fatalf("updated: %+v", listed)
	}
	directories := p.demi.Directories()
	if len(directories) != 1 || directories[0].Name != "review" {
		t.Fatalf("directories: %+v", directories)
	}
	commit(t, repos, "acme/tools", skillstest.File{Path: "README.md", Bytes: []byte("no skills now")})
	p.update(t, id)
	after := p.state(t).Sources[0]
	if diff := cmp.Diff(listed.Skills, after.Skills); diff != "" {
		t.Fatal(diff)
	}
	if after.Commit == nil || *after.Commit != *listed.Commit || after.Failure == nil ||
		after.Failure.Message != "the repository holds no skill" {
		t.Fatalf("failed update: %+v", after)
	}
}

func TestProjectSkillsPrecedenceAndRepositoryRoot(t *testing.T) {
	repos := skillstest.New(t)
	toolsRepository(t, repos)
	p := newPlugged(t, repos)
	id := p.add(t, "acme/tools")
	p.enable(t, id, "review")
	project := func(name, description string) []byte {
		return []byte(skillstest.SkillMD("name: " + name + "\ndescription: " + description))
	}
	p.demi.HostFiles = map[string][]byte{
		"/home/me/app/.git/HEAD":                            []byte("ref: refs/heads/main"),
		"/home/me/app/web/.claude/skills/release/SKILL.md":  project("release", "Release from web."),
		"/home/me/app/.agents/skills/release/SKILL.md":      project("release", "Release from the root."),
		"/home/me/app/.agents/skills/group/review/SKILL.md": project("review", "The repository's review."),
		"/home/me/app/.claude/skills/tests/SKILL.md":        project("tests", "Write tests."),
		"/home/me/.agents/skills/outside/SKILL.md":          project("outside", "Above the repository."),
	}
	block := p.context(t, "/home/me/app/web", "t1", nil)
	if block == nil {
		t.Fatal("missing catalog")
	}
	for _, path := range []string{
		"/home/me/app/web/.claude/skills/release/SKILL.md",
		"/home/me/app/.agents/skills/group/review/SKILL.md",
		"/home/me/app/.claude/skills/tests/SKILL.md",
	} {
		if !strings.Contains(*block, "<location>"+path+"</location>") {
			t.Fatal(*block)
		}
	}
	for _, excluded := range []string{"Release from the root.", "Review a change.", "outside"} {
		if strings.Contains(*block, excluded) {
			t.Fatal(*block)
		}
	}
	release := strings.Index(*block, "<name>release</name>")
	tests := strings.Index(*block, "<name>tests</name>")
	if release < 0 || tests < 0 || release >= tests {
		t.Fatal("catalog not sorted")
	}
}

func TestStoppedHostRetriesAndEachTurnSearchesOnce(t *testing.T) {
	p := newPlugged(t, skillstest.New(t))
	if p.context(t, "/home/me/app", "t1", nil) != nil {
		t.Fatal("stopped host produced catalog")
	}
	p.demi.HostFiles = map[string][]byte{
		"/home/me/app/.agents/skills/release/SKILL.md": []byte(
			skillstest.SkillMD("name: release\ndescription: Cut a release."),
		),
	}
	block := p.context(t, "/home/me/app", "t1", nil)
	if block == nil || !strings.Contains(*block, "<name>release</name>") {
		t.Fatalf("woken host: %v", block)
	}
	p.demi.HostFiles = map[string][]byte{}
	if p.context(t, "/home/me/app", "t1", []string{*block}) != nil {
		t.Fatal("searched twice in one turn")
	}
	if got := p.context(
		t,
		"/home/me/app",
		"t2",
		[]string{*block},
	); got == nil ||
		*got != "No skills are available now." {
		t.Fatalf("next turn: %v", got)
	}
}

func TestCatalogBudgetThroughProjectSearch(t *testing.T) {
	p := newPlugged(t, skillstest.New(t))
	p.demi.HostFiles = map[string][]byte{}
	for i := 0; i < 30; i++ {
		name := fmt.Sprintf("skill-%02d", i)
		p.demi.HostFiles["/repo/.agents/skills/"+name+"/SKILL.md"] = []byte(
			skillstest.SkillMD("name: " + name + "\ndescription: " + strings.Repeat("word ", 60)),
		)
	}
	block := p.context(t, "/repo", "t1", nil)
	if block == nil || utf8.RuneCountInString(*block) > 8000 || strings.Count(*block, "<skill>") != 30 ||
		strings.Count(*block, "…</description>") != 30 {
		t.Fatalf("shortened: %v", block)
	}
	lengths := map[int]bool{}
	for _, rest := range strings.Split(*block, "<description>")[1:] {
		description, _, _ := strings.Cut(rest, "</description>")
		lengths[utf8.RuneCountInString(description)] = true
	}
	if len(lengths) != 1 {
		t.Fatalf("unequal descriptions: %v", lengths)
	}
	p.demi.HostFiles = map[string][]byte{}
	for i := 0; i < 100; i++ {
		name := fmt.Sprintf("%s%02d", strings.Repeat("a", 60), i)
		p.demi.HostFiles["/repo/.agents/skills/"+name+"/SKILL.md"] = []byte(
			skillstest.SkillMD("name: " + name + "\ndescription: Short."),
		)
	}
	block = p.context(t, "/repo", "t2", nil)
	if block == nil || utf8.RuneCountInString(*block) > 8000 || strings.Contains(*block, "<description>") ||
		!strings.HasSuffix(
			*block,
			fmt.Sprintf("%d more skills are not listed.", 100-strings.Count(*block, "<skill>")),
		) {
		t.Fatalf("omitted: %v", block)
	}
}

func TestShutdownDuringFetchPreservesSource(t *testing.T) {
	repos := skillstest.New(t)
	toolsRepository(t, repos)
	commit(t, repos, "acme/other", skillFile("other/SKILL.md", "name: other\ndescription: Another."))
	entered, ended := make(chan struct{}), make(chan struct{})
	var once sync.Once
	repos.BeforeUpload = func(ctx context.Context, name string) error {
		if name != "acme/tools" {
			return nil
		}
		once.Do(func() { close(entered) })
		<-ctx.Done()
		close(ended)
		return ctx.Err()
	}
	p := newPlugged(t, repos)
	raw, err := p.method(t.Context(), "add_source", skills.AddSource{Origin: "acme/tools"})
	if err != nil {
		t.Fatal(err)
	}
	added, err := skills.DecodeAddedSource(raw)
	if err != nil {
		t.Fatal(err)
	}
	<-entered
	p.instance.(plugin.Closer).Close()
	<-ended
	second := newPlugged(t, repos)
	second.demi = p.demi
	second.add(t, "acme/other")
	stored, found := p.demi.Value(added.Source)
	if !found || stored.Revision != 1 {
		t.Fatalf("cancelled fetch wrote: %+v", stored)
	}
	state := second.state(t).Sources[0]
	if state.Commit != nil || state.Failure != nil || len(state.Skills) != 0 {
		t.Fatalf("cancelled fetch changed source: %+v", state)
	}
}

func noise(length int) []byte {
	result := make([]byte, length)
	state := uint64(0x9e3779b97f4a7c15)
	for i := range result {
		state ^= state << 13
		state ^= state >> 7
		state ^= state << 17
		result[i] = byte(state)
	}
	return result
}
