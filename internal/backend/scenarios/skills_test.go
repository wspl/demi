package scenarios_test

import (
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/wspl/demi/internal/backend/backendtest"
	"github.com/wspl/demi/internal/plugin/skills"
	"github.com/wspl/demi/internal/plugin/skills/skillstest"
	"github.com/wspl/demi/internal/webapiproto"
)

// TestSkillCatalogInstallsOnlyForJobsAndRemovesDisabledSkills uses a local Git endpoint and real runner to
// exercise catalog-only and installed skills.
func TestSkillCatalogInstallsOnlyForJobsAndRemovesDisabledSkills(t *testing.T) {
	t.Parallel()
	repos := skillstest.New(t)
	_, err := repos.Commit(
		t.Context(),
		"acme/tools",
		[]skillstest.File{
			{Path: "review/SKILL.md", Bytes: []byte(skillstest.SkillMD("name: review\ndescription: Review a change."))},
			{Path: "review/check.sh", Bytes: []byte("#!/bin/sh\necho checked\n"), Executable: true},
		},
	)
	wireMust(t, err)
	factory, err := repos.Factory()
	wireMust(t, err)
	w := filesWorking(t, "", func(h *backendtest.Harness) {
		for i, f := range h.Config.Plugins {
			if f.Manifest().ID == "skills" {
				h.Config.Plugins[i] = factory
			}
		}
	})
	release := filepath.Join(w.root, ".agents/skills/release")
	wireMust(t, os.MkdirAll(release, 0o755))
	wireMust(
		t,
		os.WriteFile(
			filepath.Join(release, "SKILL.md"),
			[]byte(skillstest.SkillMD("name: release\ndescription: Cut a release.")),
			0o644,
		),
	)
	page, _ := conversationPage(w.ctx, t, w.backend, &w.session)
	added := conversationDecode(
		t,
		conversationRequest(
			w.ctx,
			t,
			w.backend,
			&w.session,
			"POST",
			"/api/plugins/skills/calls/add_source",
			conversationJSON(t, skills.AddSource{Origin: "acme/tools"}),
			200,
		),
		skills.DecodeAddedSource,
	)
	_, err = page.Until(w.ctx, func(event webapiproto.SyncEvent) bool {
		p, ok := event.(*webapiproto.SyncEventPlugin)
		if !ok || p.Plugin != "skills" {
			return false
		}
		state, e := skills.DecodeSkillsState(p.State)
		wireMust(t, e)
		return len(state.Sources) > 0 && state.Sources[0].Commit != nil && !state.Sources[0].Fetching
	})
	wireMust(t, err)
	enabled := func(on bool) {
		conversationRequest(
			w.ctx,
			t,
			w.backend,
			&w.session,
			"POST",
			"/api/plugins/skills/calls/set_enabled",
			conversationJSON(t, skills.SetEnabled{Source: added.Source, Skill: "review", Enabled: on}),
			200,
		)
	}
	enabled(true)
	w.vendor.Respond(conversationAnswer(t, []string{"hello"}, 1, 1))
	_, err = w.socket.Chat(w.ctx, "m1", "go")
	wireMust(t, err)
	filesContains(
		t,
		filesModelField(t, w.vendor.Requests()[0].Body, "messages"),
		"<name>review</name>",
		"<location>~/.demi/plugins/skills/review-",
		"/.agents/skills/release/SKILL.md</location>",
	)
	if _, err := os.Stat(filepath.Join(w.paired.Runner.Home(), ".demi/plugins/skills")); !os.IsNotExist(err) {
		t.Fatalf("installed without a job: %v", err)
	}
	w.vendor.Respond(
		conversationShell(
			t,
			"t1",
			"cat ~/.demi/plugins/skills/review-*/SKILL.md && ~/.demi/plugins/skills/review-*/check.sh && "+
				"ls -l ~/.demi/plugins/skills/review-*/",
			10000,
		),
	)
	w.vendor.Respond(conversationAnswer(t, []string{"read"}, 1, 1))
	_, err = w.socket.Chat(w.ctx, "m2", "go")
	wireMust(t, err)
	requests := w.vendor.Requests()
	filesContains(
		t,
		conversationToolResult(t, requests[len(requests)-1], "t1"),
		"Review a change.",
		"checked",
		"-r-xr-xr-x",
		"-r--r--r--",
	)
	enabled(false)
	w.vendor.Respond(conversationShell(t, "t2", "ls -A ~/.demi/plugins/skills", 10000))
	w.vendor.Respond(conversationAnswer(t, []string{"listed"}, 1, 1))
	_, err = w.socket.Chat(w.ctx, "m3", "go")
	wireMust(t, err)
	requests = w.vendor.Requests()
	if result := conversationToolResult(t, requests[len(requests)-1], "t2"); strings.Contains(result, "review-") {
		t.Fatal(result)
	}
	blocks := func(body []byte) int {
		messages := filesModelField(t, body, "messages")
		return strings.Count(messages, "<available_skills>") + strings.Count(messages, "No skills are available now.")
	}
	before := blocks(requests[len(requests)-1].Body)
	conversationRequest(w.ctx, t, w.backend, &w.session, "PUT", "/api/plugins/skills", `{"enabled":false}`, 204)
	wireMust(t, os.RemoveAll(release))
	w.vendor.Respond(conversationAnswer(t, []string{"quiet"}, 1, 1))
	_, err = w.socket.Chat(w.ctx, "m4", "go")
	wireMust(t, err)
	requests = w.vendor.Requests()
	conversationEqual(t, blocks(requests[len(requests)-1].Body), before)
	wireMust(t, page.Close(w.ctx))
	wireMust(t, w.socket.Close(w.ctx))
	wireMust(t, w.backend.Close(w.ctx))
}
