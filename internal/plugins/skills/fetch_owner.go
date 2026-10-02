package skills

import (
	"context"
	"fmt"
	"log/slog"

	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/plugin"
)

// startFetch registers each worker before launch under the same mutex as Close.
func (p *instance) startFetch(port plugin.Port, id string) {
	p.mu.Lock()
	if p.ctx.Err() != nil || p.fetching[id] {
		p.mu.Unlock()
		return
	}
	p.fetching[id] = true
	p.workers.Add(1)
	p.mu.Unlock()
	go func() {
		defer p.workers.Done()
		if err := p.fetchSource(p.ctx, port, id); err != nil && p.ctx.Err() == nil {
			slog.Warn("a skill source's fetch was not recorded", "source", id, "error", err)
		}
		p.mu.Lock()
		delete(p.fetching, id)
		p.mu.Unlock()
		if p.ctx.Err() != nil {
			return
		}
		// A page that misses this change reads the state again when it reconnects.
		if err := port.Changed(p.ctx, plugin.ScopeUser); err != nil {
			slog.Warn("the pages did not learn of a fetch's end", "source", id, "error", err)
		}
	}()
}

func (p *instance) fetchSource(ctx context.Context, port plugin.Port, id string) error {
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
	fetched, fetchErr := fetch(ctx, p.resolve(origin.url), origin.repository())
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
	if err := p.enterMutation(ctx); err != nil {
		return err
	}
	defer p.leaveMutation()
	for {
		stored, err := findSource(ctx, port, id)
		if err != nil {
			return err
		}
		if stored == nil {
			// Removed while it was fetched.
			return nil
		}
		if err := ctx.Err(); err != nil {
			return err
		}
		recorded := stored.source
		at := p.clock.Now()
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
			return err
		}
		break
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
		skills = append(skills, userSkill{Name: skill.parsed.name, Description: skill.parsed.description, Directory: skill.directory, Files: files, Warnings: skill.parsed.warnings, DisableModelInvocation: skill.parsed.disableModelInvocation})
	}
	return skills, nil
}
