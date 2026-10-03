package skills

import (
	"context"
	"errors"
	"fmt"
	"strings"

	"github.com/wspl/demi/internal/plugin"
)

func findSource(ctx context.Context, port plugin.Port, id string) (*storedSource, error) {
	value, err := port.Value(ctx, id)
	if err != nil || value == nil {
		return nil, err
	}
	decoded, err := decodeSource(value.Value)
	if err != nil {
		return nil, fmt.Errorf("a stored source does not read: %w", err)
	}
	return &storedSource{source: decoded, revision: value.Revision}, nil
}

func readSource(ctx context.Context, port plugin.Port, id string) (*storedSource, error) {
	value, err := findSource(ctx, port, id)
	if err != nil {
		return nil, err
	}
	if value == nil {
		return nil, sourceNotFound(id)
	}
	return value, nil
}

func sourceConflict(err error) bool {
	var conflict *plugin.PortRefusalConflict
	return errors.As(err, &conflict)
}

func (i *instance) addSource(ctx context.Context, port plugin.Port, text string) (string, error) {
	origin, err := parseOrigin(text)
	if err != nil {
		return "", &plugin.ErrorRefused{Reason: "invalid_origin", Message: err.Error()}
	}
	id := origin.id()
	all, err := readSources(ctx, port)
	if err != nil {
		return "", err
	}
	taken := &plugin.ErrorRefused{Reason: "source_exists", Message: origin.url + " is added already"}
	if _, ok := all[id]; ok {
		return "", taken
	}
	var added uint64
	for _, stored := range all {
		added = max(added, stored.source.Added)
	}
	value := source{Origin: strings.TrimSpace(text), Added: added + 1, Skills: []userSkill{}, Skipped: []Skipped{}}
	if _, err := writeSource(ctx, port, id, value, nil); err != nil {
		if sourceConflict(err) {
			return "", taken
		}
		return "", err
	}
	if err := port.Changed(ctx, plugin.ScopeUser); err != nil {
		return "", err
	}
	return id, nil
}

func (i *instance) removeSource(ctx context.Context, port plugin.Port, id string) error {
	for {
		stored, err := readSource(ctx, port, id)
		if err != nil {
			return err
		}
		err = port.RemoveValue(ctx, id, stored.revision)
		if sourceConflict(err) {
			continue
		}
		if err != nil {
			return err
		}
		break
	}
	return i.directoriesChanged(ctx, port)
}

func (i *instance) switchSource(
	ctx context.Context,
	port plugin.Port,
	id string,
	chosen func(source) map[string]bool,
	enabled bool,
) error {
	for {
		all, err := readSources(ctx, port)
		if err != nil {
			return err
		}
		stored, ok := all[id]
		if !ok {
			return sourceNotFound(id)
		}
		changed, err := switchSkills(all, id, chosen(stored.source), enabled)
		if err != nil {
			return err
		}
		_, err = writeSource(ctx, port, id, changed, &stored.revision)
		if sourceConflict(err) {
			continue
		}
		if err != nil {
			return err
		}
		break
	}
	return i.directoriesChanged(ctx, port)
}

func (i *instance) directoriesChanged(ctx context.Context, port plugin.Port) error {
	all, err := readSources(ctx, port)
	if err != nil {
		return err
	}
	if _, err := port.SetDirectories(ctx, sourceDirectories(all)); err != nil {
		return err
	}
	return port.Changed(ctx, plugin.ScopeUser)
}
