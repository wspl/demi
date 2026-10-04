package skills

import (
	"cmp"
	"context"
	"fmt"
	"maps"
	"slices"
	"strings"

	"github.com/wspl/demi/internal/plugin"
	"github.com/wspl/demi/internal/types"
)

type storedSource struct {
	source   source
	revision uint64
}
type sources map[string]storedSource

// readSources validates every stored source at the plugin port boundary.
func readSources(ctx context.Context, port plugin.Port) (sources, error) {
	values, err := port.Values(ctx)
	if err != nil {
		return nil, err
	}
	all := make(sources, len(values))
	for id, value := range values {
		decoded, err := decodeSource(value.Value)
		if err != nil {
			return nil, fmt.Errorf("a stored source does not read: %w", err)
		}
		all[id] = storedSource{source: decoded, revision: value.Revision}
	}
	return all, nil
}

// writeSource names every file blob so the value keeps its contents alive.
func writeSource(ctx context.Context, port plugin.Port, id string, value source, revision *uint64) (uint64, error) {
	encoded, err := value.MarshalJSON()
	if err != nil {
		return 0, fmt.Errorf("encode source: %w", err)
	}
	blobs := make(map[types.BlobRef]struct{})
	for _, skill := range value.Skills {
		for _, file := range skill.Files {
			blobs[file.Blob] = struct{}{}
		}
	}
	return port.WriteValueNaming(ctx, id, encoded, revision, slices.Sorted(maps.Keys(blobs)))
}

func sourceNotFound(id string) error {
	return &plugin.ErrorRefused{Reason: "source_not_found", Message: fmt.Sprintf("No skill source \"%s\"", id)}
}

// switchSkills checks all chosen skills before changing any enablement.
func switchSkills(all sources, id string, chosen map[string]bool, enabled bool) (source, error) {
	stored, ok := all[id]
	if !ok {
		return source{}, sourceNotFound(id)
	}
	changed := stored.source
	changed.Skills = slices.Clone(changed.Skills)
	for _, name := range slices.Sorted(maps.Keys(chosen)) {
		if !slices.ContainsFunc(changed.Skills, func(skill userSkill) bool { return skill.Name == name }) {
			return source{}, &plugin.ErrorRefused{
				Reason:  "skill_not_found",
				Message: fmt.Sprintf("The source has no skill \"%s\"", name),
			}
		}
	}
	if enabled {
		taken := make(map[string]string)
		for _, other := range slices.Sorted(maps.Keys(all)) {
			value := all[other].source
			for _, skill := range value.Skills {
				if skill.Enabled && (other != id || !chosen[skill.Name]) {
					taken[directoryName(skill.Name)] = value.Origin
				}
			}
		}
		turning := make(map[string]bool)
		for _, skill := range changed.Skills {
			if !chosen[skill.Name] {
				continue
			}
			directory := directoryName(skill.Name)
			if origin, exists := taken[directory]; exists {
				return source{}, &plugin.ErrorRefused{
					Reason:  "skill_name_taken",
					Message: fmt.Sprintf("A skill named \"%s\" from %s is on; turn it off first", skill.Name, origin),
				}
			}
			if turning[directory] {
				return source{}, &plugin.ErrorRefused{
					Reason:  "skill_name_taken",
					Message: fmt.Sprintf("%s has two skills named \"%s\"; turn on one", changed.Origin, skill.Name),
				}
			}
			turning[directory] = true
		}
	}
	for i := range changed.Skills {
		if chosen[changed.Skills[i].Name] {
			changed.Skills[i].Enabled = enabled
		}
	}
	return changed, nil
}

// pinSource replaces the pinned skills, retaining enablement only by name.
func pinSource(previous source, commit string, skills []userSkill, skipped []Skipped, at types.Timestamp) source {
	next := previous
	next.Commit = &commit
	next.FetchedAt = &at
	next.Failure = nil
	next.Skipped = skipped
	next.Skills = slices.Clone(skills)
	for i := range next.Skills {
		next.Skills[i].Enabled = slices.ContainsFunc(previous.Skills, func(old userSkill) bool {
			return old.Enabled && old.Name == next.Skills[i].Name
		})
	}
	return next
}

// sourceDirectories describes only enabled skills for installation by the Host owner.
func sourceDirectories(all sources) []plugin.HostDirectory {
	directories := []plugin.HostDirectory{}
	for _, id := range slices.Sorted(maps.Keys(all)) {
		for _, skill := range all[id].source.Skills {
			if skill.Enabled {
				directories = append(
					directories,
					plugin.HostDirectory{Name: directoryName(skill.Name), Files: slices.Clone(skill.Files)},
				)
			}
		}
	}
	return directories
}

// directoryName converts a lenient skill name to its stable Host directory name.
func directoryName(name string) string {
	if validName(name) {
		return name
	}
	var directory strings.Builder
	hyphen := false
	for _, c := range name {
		if c < 128 && asciiAlphanumeric(byte(c)) {
			if hyphen {
				directory.WriteByte('-')
			}
			directory.WriteString(asciiLower(string(c)))
			hyphen = false
		} else if directory.Len() > 0 {
			hyphen = true
		}
	}
	result := directory.String()
	if len(result) > 64 {
		result = result[:64]
	}
	result = strings.TrimRight(result, "-")
	if result == "" {
		return "skill"
	}
	return result
}

func validName(name string) bool {
	if len(name) < 1 || len(name) > 64 || strings.HasPrefix(name, "-") || strings.HasSuffix(name, "-") ||
		strings.Contains(name, "--") {
		return false
	}
	for _, c := range []byte(name) {
		if (c < 'a' || c > 'z') && (c < '0' || c > '9') && c != '-' {
			return false
		}
	}
	return true
}

// skillLocation uses the plugin owner's content digest for the catalog path.
func skillLocation(skill userSkill) string {
	directory := plugin.HostDirectory{Name: directoryName(skill.Name), Files: skill.Files}
	return directory.Path("skills") + "/SKILL.md"
}

// pageState orders the source summaries by the user's addition order.
func pageState(all sources, fetching map[string]bool) SkillsState {
	ids := slices.Sorted(maps.Keys(all))
	slices.SortStableFunc(ids, func(a, b string) int { return cmp.Compare(all[a].source.Added, all[b].source.Added) })
	state := SkillsState{Sources: make([]SourceState, 0, len(ids))}
	for _, id := range ids {
		value := all[id].source
		skills := make([]SkillState, 0, len(value.Skills))
		for _, skill := range value.Skills {
			skills = append(
				skills,
				SkillState{
					Name:                   skill.Name,
					Description:            skill.Description,
					Warnings:               slices.Clone(skill.Warnings),
					Enabled:                skill.Enabled,
					DisableModelInvocation: skill.DisableModelInvocation,
				},
			)
		}
		state.Sources = append(
			state.Sources,
			SourceState{
				ID:        id,
				Origin:    value.Origin,
				Commit:    value.Commit,
				FetchedAt: value.FetchedAt,
				Fetching:  fetching[id],
				Failure:   value.Failure,
				Skills:    skills,
				Skipped:   slices.Clone(value.Skipped),
			},
		)
	}
	return state
}
