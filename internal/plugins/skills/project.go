package skills

import (
	"cmp"
	"context"
	"fmt"
	"log/slog"
	"slices"
	"strings"

	"github.com/wspl/demi/internal/plugin"
)

const projectMaxDirectories = 2000
const projectMaxDepth = 6
const sourceMaxSkills = 100

type projectSkill struct {
	catalogEntry
	disableModelInvocation bool
}

type projectNode struct {
	path  string
	rank  int
	depth int
}

// searchProject reads repository skills through the port without waking the Host.
func searchProject(ctx context.Context, port plugin.Port, cwd string) ([]projectSkill, error) {
	searched, err := searchedDirectories(ctx, port, cwd)
	if err != nil {
		return nil, err
	}
	frontier := []projectNode{}
	for rank, directory := range searched {
		for place, skills := range []string{".agents/skills", ".claude/skills"} {
			frontier = append(frontier, projectNode{path: hostJoin(directory, skills), rank: rank*2 + place})
		}
	}
	type ranked struct {
		rank  int
		skill projectSkill
	}
	found := []ranked{}
	read := 0
	for len(frontier) > 0 && read < projectMaxDirectories && len(found) < sourceMaxSkills {
		frontier = frontier[:min(len(frontier), projectMaxDirectories-read)]
		read += len(frontier)
		reads := make([]plugin.HostRead, 0, len(frontier)*2)
		for _, node := range frontier {
			reads = append(reads, plugin.HostRead{Path: node.path}, plugin.HostRead{Path: hostJoin(node.path, "SKILL.md"), Limit: skillMDMaxBytes + 1})
		}
		files, err := port.ReadHostFiles(ctx, reads)
		if err != nil {
			return nil, err
		}
		if len(files) != len(reads) {
			return nil, fmt.Errorf("host returned %d files for %d reads", len(files), len(reads))
		}
		next := []projectNode{}
		for i, node := range frontier {
			listing, manifest := files[2*i], files[2*i+1]
			if file, ok := manifest.(*plugin.HostFileFile); ok && node.depth > 0 {
				skill, err := parseProjectSkill(node.path, file)
				if err != nil {
					slog.Info("a project SKILL.md is not a skill", "skill", hostJoin(node.path, "SKILL.md"), "reason", err)
				} else {
					found = append(found, ranked{node.rank, skill})
				}
				continue
			}
			directory, ok := listing.(*plugin.HostFileDirectory)
			if !ok || node.depth == projectMaxDepth {
				continue
			}
			for _, entry := range directory.Entries {
				if entry.Kind == plugin.EntryKindDirectory || entry.Kind == plugin.EntryKindSymlink {
					next = append(next, projectNode{path: hostJoin(node.path, entry.Name), rank: node.rank, depth: node.depth + 1})
				}
			}
		}
		frontier = next
	}
	found = found[:min(len(found), sourceMaxSkills)]
	slices.SortStableFunc(found, func(a, b ranked) int { return cmp.Compare(a.rank, b.rank) })
	skills := []projectSkill{}
	for _, value := range found {
		index := slices.IndexFunc(skills, func(kept projectSkill) bool { return kept.name == value.skill.name })
		if index >= 0 {
			slog.Info("a project skill is shadowed by one of the same name", "skill", value.skill.location, "kept", skills[index].location)
		} else {
			skills = append(skills, value.skill)
		}
	}
	return skills, nil
}

func searchedDirectories(ctx context.Context, port plugin.Port, cwd string) ([]string, error) {
	ancestors := hostAncestors(cwd)
	reads := make([]plugin.HostRead, len(ancestors))
	for i, directory := range ancestors {
		reads[i] = plugin.HostRead{Path: hostJoin(directory, ".git")}
	}
	files, err := port.ReadHostFiles(ctx, reads)
	if err != nil {
		return nil, err
	}
	if len(files) != len(reads) {
		return nil, fmt.Errorf("host returned %d files for %d reads", len(files), len(reads))
	}
	for i, file := range files {
		switch file.(type) {
		case *plugin.HostFileMissing, *plugin.HostFileUnreadable:
		case *plugin.HostFileDirectory, *plugin.HostFileFile, *plugin.HostFileOther:
			return ancestors[:i+1], nil
		}
	}
	return ancestors[:1], nil
}

// hostAncestors preserves the Host's slash-separated spelling, independent of the backend OS.
func hostAncestors(cwd string) []string {
	directory := strings.TrimRight(cwd, "/")
	if directory == "" {
		return []string{"/"}
	}
	ancestors := []string{directory}
	for {
		index := strings.LastIndexByte(directory, '/')
		if index < 0 {
			return ancestors
		}
		directory = directory[:index]
		if directory == "" {
			directory = "/"
		}
		ancestors = append(ancestors, directory)
		if directory == "/" {
			return ancestors
		}
	}
}

// hostJoin appends a Host path component without cleaning its spelling.
func hostJoin(directory, name string) string { return strings.TrimRight(directory, "/") + "/" + name }

func parseProjectSkill(directory string, file *plugin.HostFileFile) (projectSkill, error) {
	if file.Size > skillMDMaxBytes {
		return projectSkill{}, fmt.Errorf("SKILL.md is larger than 256 KiB")
	}
	name := directory[strings.LastIndexByte(directory, '/')+1:]
	parsed, err := parseSkill(name, file.Bytes)
	if err != nil {
		return projectSkill{}, err
	}
	location := hostJoin(directory, "SKILL.md")
	for _, warning := range parsed.warnings {
		slog.Info("a project skill has a warning", "skill", location, "warning", warning)
	}
	return projectSkill{catalogEntry: catalogEntry{name: parsed.name, description: parsed.description, location: location}, disableModelInvocation: parsed.disableModelInvocation}, nil
}
