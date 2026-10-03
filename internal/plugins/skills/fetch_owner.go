package skills

import (
	"context"
	"fmt"
	"log/slog"

	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/plugin"
)

// startFetch registers each worker before launch under the same mutex as Close.
func (i *instance) startFetch(port plugin.Port, id string) {
	i.mu.Lock()
	if i.ctx.Err() != nil || i.fetching[id] {
		i.mu.Unlock()
		return
	}
	i.fetching[id] = true
	i.workers.Add(1)
	i.mu.Unlock()
	go func() {
		defer i.workers.Done()
		if err := i.fetchSource(i.ctx, port, id); err != nil && i.ctx.Err() == nil {
			slog.Warn("a skill source's fetch was not recorded", "source", id, "error", err)
		}
		i.mu.Lock()
		delete(i.fetching, id)
		i.mu.Unlock()
		if i.ctx.Err() != nil {
			return
		}
		// A page that misses this change reads the state again when it reconnects.
		if err := port.Changed(i.ctx, plugin.ScopeUser); err != nil {
			slog.Warn("the pages did not learn of a fetch's end", "source", id, "error", err)
		}
	}()
}

func (i *instance) fetchSource(ctx context.Context, port plugin.Port, id string) error {
	if err := port.Changed(ctx, plugin.ScopeUser); err != nil {
		return err
	}
	stored, err := readSource(ctx, port, id)
	if err != nil {
		return err
	}
	origin, err := parseOrigin(stored.source.Origin)
	if err != nil {
		return fmt.Errorf("a stored origin: %w", err)
	}
	fetched, fetchErr := fetch(ctx, i.resolve(origin.url), origin.repository())
	if ctx.Err() != nil {
		return ctx.Err()
	}
	var skills []userSkill
	if fetchErr == nil {
		skills, err = putFetchedFiles(ctx, port, fetched)
		if err != nil {
			return err
		}
	}
	if err := i.enterMutation(ctx); err != nil {
		return err
	}
	defer i.leaveMutation()
	recorded, err := i.recordFetch(ctx, port, id, fetched, skills, fetchErr)
	if err != nil {
		return err
	}
	if !recorded {
		return nil
	}
	if fetchErr == nil {
		all, err := readSources(ctx, port)
		if err != nil {
			return err
		}
		_, err = port.SetDirectories(ctx, sourceDirectories(all))
		return err
	}
	return nil
}

// putFetchedFiles puts every file before publishing a source that names its blob.
func putFetchedFiles(ctx context.Context, port plugin.Port, fetched fetched) ([]userSkill, error) {
	skills := make([]userSkill, 0, len(fetched.skills))
	for _, skill := range fetched.skills {
		files := make([]plugin.DirectoryFile, 0, len(skill.files))
		for _, file := range skill.files {
			if err := ctx.Err(); err != nil {
				return nil, err
			}
			blob, err := port.PutBlob(ctx, core.B64Bytes(file.bytes))
			if err != nil {
				return nil, err
			}
			files = append(files, plugin.DirectoryFile{Path: file.path, Executable: file.executable, Blob: blob})
		}
		skills = append(
			skills,
			userSkill{
				Name:                   skill.parsed.name,
				Description:            skill.parsed.description,
				Directory:              skill.directory,
				Files:                  files,
				Warnings:               skill.parsed.warnings,
				DisableModelInvocation: skill.parsed.disableModelInvocation,
			},
		)
	}
	return skills, nil
}

// recordFetch reports whether the source still existed when its fetch was recorded.
func (i *instance) recordFetch(
	ctx context.Context,
	port plugin.Port,
	id string,
	fetched fetched,
	skills []userSkill,
	fetchErr error,
) (bool, error) {
	for {
		stored, found, err := findSource(ctx, port, id)
		if err != nil {
			return false, err
		}
		if !found {
			// Removed while it was fetched.
			return false, nil
		}
		if err := ctx.Err(); err != nil {
			return false, err
		}
		recorded := stored.source
		at := i.clock.Now()
		if fetchErr == nil {
			recorded = pinSource(recorded, fetched.commit, skills, fetched.skipped, at)
		} else {
			recorded.Failure = &Failure{At: at, Message: fetchErr.Error()}
		}
		_, err = writeSource(ctx, port, id, recorded, &stored.revision)
		if sourceConflict(err) {
			continue
		}
		if err != nil {
			return false, err
		}
		break
	}
	return true, nil
}
