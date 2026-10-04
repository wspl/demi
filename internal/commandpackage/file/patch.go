//nolint:staticcheck // Command error messages are user-visible sentences, kept byte for byte.
package file

import (
	"bytes"
	"context"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"regexp"
	"slices"
	"strconv"
	"strings"

	"github.com/wspl/demi/internal/commandsdk"
	"github.com/wspl/demi/internal/contract"
)

type filePatch struct {
	old, new *string
	hunks    []hunk
}
type hunk struct {
	start, oldCount, newCount uint64
	lines                     []patchLine
}
type patchLine struct {
	kind    byte
	text    string
	newline bool
}
type change struct {
	path          string
	before, after []byte
	permissions   os.FileMode
}

// applyPatch plans all file changes before publishing any of them.
func applyPatch(ctx context.Context, cwd, diff string, recording *commandsdk.Recording) (string, error) {
	patches, err := parsePatch(ctx, diff)
	if err != nil {
		return "", err
	}
	var changes []change
	for _, patch := range patches {
		if err := ctx.Err(); err != nil {
			return "", err
		}
		planned, err := planFilePatch(ctx, cwd, patch)
		if err != nil {
			return "", err
		}
		changes = append(changes, planned...)
	}
	touched := make(map[string]bool)
	for _, change := range changes {
		key := patchPathKey(change.path)
		if touched[key] {
			return "", errors.New("Patch changes the same path more than once")
		}
		touched[key] = true
	}
	changes = slices.DeleteFunc(
		changes,
		func(c change) bool { return (c.before == nil) == (c.after == nil) && bytes.Equal(c.before, c.after) },
	)
	if recording != nil {
		for _, change := range changes {
			recording.Track(ctx, change.path)
		}
	}
	if err := commitChanges(ctx, changes, recording); err != nil {
		return "", err
	}
	return fmt.Sprintf("Patched %d file(s)\n", len(patches)), nil
}

// commitChanges restores earlier file publications if any later mutation fails.
func commitChanges(ctx context.Context, changes []change, recording *commandsdk.Recording) error {
	for index, c := range changes {
		err := ctx.Err()
		if err == nil {
			if c.after == nil {
				err = os.Remove(c.path)
			} else {
				err = atomicWrite(ctx, c.path, c.after, c.before == nil)
			}
		}
		if err == nil {
			continue
		}
		var rollbacks []error
		for i := index - 1; i >= 0; i-- {
			prior := changes[i]
			var rollback error
			if prior.before == nil {
				rollback = os.Remove(prior.path)
			} else {
				rollback = atomicWrite(context.WithoutCancel(ctx), prior.path, prior.before, prior.after == nil)
				if rollback == nil {
					rollback = os.Chmod(prior.path, prior.permissions)
				}
			}
			if rollback != nil {
				rollbacks = append(rollbacks, rollback)
			} else if recording != nil {
				recording.Restored(prior.path)
			}
		}
		if len(rollbacks) != 0 {
			return &rollbackError{cause: err, failures: rollbacks}
		}
		return err
	}
	return nil
}

type rollbackError struct {
	cause    error
	failures []error
}

// Error reports the original mutation failure and each rollback failure.
func (e *rollbackError) Error() string {
	message := e.cause.Error()
	for _, err := range e.failures {
		message += "\nRollback failed: " + err.Error()
	}
	return message
}

// Unwrap exposes only rollback failures: a failed rollback is a command failure
// even if cancellation triggered it.
func (e *rollbackError) Unwrap() []error { return e.failures }

func applyHunks(ctx context.Context, original string, hunks []hunk) (string, error) {
	lines := strings.SplitAfter(original, "\n")
	if lines[len(lines)-1] == "" {
		lines = lines[:len(lines)-1]
	}
	offset := int64(0)
	for _, h := range hunks {
		if err := ctx.Err(); err != nil {
			return "", err
		}
		originalStart := h.start
		if h.oldCount != 0 && originalStart != 0 {
			originalStart--
		}
		if originalStart > uint64(^uint64(0)>>1) {
			return "", errors.New("Patch hunk position is out of range")
		}
		start := int64(originalStart) + offset
		if offset > 0 && start < 0 {
			return "", errors.New("Patch hunk position is out of range")
		}
		if start < 0 {
			return "", errors.New("Patch hunk starts before file")
		}
		old, updated := hunkLines(h)
		if uint64(len(old)) != h.oldCount || uint64(len(updated)) != h.newCount {
			return "", errors.New("Patch hunk line counts do not match header")
		}
		if start > int64(len(lines)) || int64(len(old)) > int64(len(lines))-start ||
			!slices.Equal(lines[int(start):int(start)+len(old)], old) {
			return "", fmt.Errorf("Patch does not apply at line %d", h.start)
		}
		lines = slices.Replace(lines, int(start), int(start)+len(old), updated...)
		offset += int64(len(updated)) - int64(len(old))
	}
	return strings.Join(lines, ""), nil
}

var (
	hunkHeader = regexp.MustCompile(`^@@ -(\p{Nd}+)(?:,(\p{Nd}+))? \+\p{Nd}+(?:,(\p{Nd}+))? @@`)
	dateSuffix = regexp.MustCompile(
		`[\p{Z}\t\n\v\f\r\x{0085}]+\p{Nd}{4}-\p{Nd}{2}-\p{Nd}{2}` +
			`(?:[ T]\p{Nd}{2}:\p{Nd}{2}:\p{Nd}{2}(?:\.\p{Nd}+)?(?:[\p{Z}\t\n\v\f\r\x{0085}]+[+-]\p{Nd}{4})?)?$`,
	)
)

func parsePatch(ctx context.Context, diff string) ([]filePatch, error) {
	var patches []filePatch
	var pending *filePatch
	for _, line := range strings.Split(diff, "\n") {
		if err := ctx.Err(); err != nil {
			return nil, err
		}
		current := lastPatchHunk(patches)
		if pending == nil && current != nil && (strings.HasPrefix(line, "--- ") || strings.HasPrefix(line, "+++ ")) {
			if hunkNeedsLines(current) {
				current.lines = append(current.lines, patchLine{line[0], line[1:], true})
				continue
			}
		}
		switch {
		case strings.HasPrefix(line, "--- "):
			pending = &filePatch{old: parsePath(line[4:])}
		case strings.HasPrefix(line, "+++ "):
			if pending == nil {
				return nil, errors.New("New header precedes old header")
			}
			pending.new = parsePath(line[4:])
			patches = append(patches, *pending)
			pending = nil
		case strings.HasPrefix(line, "@@ "):
			parsed, err := parseHunkHeader(line, len(patches) != 0)
			if err != nil {
				return nil, err
			}
			patch := &patches[len(patches)-1]
			patch.hunks = append(patch.hunks, parsed)
		case current != nil:
			if err := appendPatchLine(current, line); err != nil {
				return nil, err
			}
		}
	}
	if len(patches) == 0 || pending != nil {
		return nil, errors.New("Invalid patch: missing file headers or hunks")
	}
	for _, patch := range patches {
		if len(patch.hunks) == 0 || (patch.old == nil && patch.new == nil) {
			return nil, errors.New("Invalid patch: missing file headers or hunks")
		}
	}
	return patches, nil
}

// parsePath removes unified-diff prefixes and timestamps from a file header.
func parsePath(value string) *string {
	value = strings.TrimSpace(value)
	value, _, _ = strings.Cut(value, "\t")
	value = dateSuffix.ReplaceAllString(value, "")
	if value == "/dev/null" {
		return nil
	}
	if strings.HasPrefix(value, "a/") || strings.HasPrefix(value, "b/") {
		value = value[2:]
	}
	return &value
}

// patchPathKey compares patch destinations by components, retaining symlink-sensitive '..'.
// filepath.Clean cannot do this because it collapses parent components.
func patchPathKey(path string) string {
	volume := filepath.VolumeName(path)
	rest := path[len(volume):]
	prefix := volume
	if len(rest) > 0 && os.IsPathSeparator(rest[0]) {
		prefix += "/"
	}
	components := strings.FieldsFunc(rest, func(r rune) bool { return r < 128 && os.IsPathSeparator(uint8(r)) })
	components = slices.DeleteFunc(components, func(component string) bool { return component == "." })
	return prefix + strings.Join(components, "/")
}

// planFilePatch resolves and applies one file patch before any file is published.
func planFilePatch(ctx context.Context, cwd string, patch filePatch) ([]change, error) {
	var oldPath, newPath string
	var err error
	if patch.old != nil {
		oldPath, err = commandsdk.Resolve(cwd, *patch.old)
		if err != nil {
			return nil, err
		}
	}
	if patch.new != nil {
		newPath, err = commandsdk.Resolve(cwd, *patch.new)
		if err != nil {
			return nil, err
		}
	}
	var before []byte
	if oldPath != "" {
		before, err = os.ReadFile(oldPath)
		if err != nil {
			return nil, err
		}
	}
	if err := contract.CheckUTF8(before); err != nil {
		return nil, err
	}
	updated, err := applyHunks(ctx, string(before), patch.hunks)
	if err != nil {
		if errors.Is(err, context.Canceled) || errors.Is(err, context.DeadlineExceeded) {
			return nil, err
		}
		label := patch.old
		if label == nil {
			label = patch.new
		}
		return nil, fmt.Errorf("Patch does not apply to %s: %w", *label, err)
	}
	if newPath == "" && updated != "" {
		return nil, errors.New("Delete patch leaves file content")
	}
	return patchChanges(oldPath, newPath, before, updated)
}

// patchChanges retains the original contents and permissions needed for rollback.
func patchChanges(oldPath, newPath string, before []byte, updated string) ([]change, error) {
	var changes []change
	samePath := patchPathKey(newPath) == patchPathKey(oldPath)
	if !samePath && newPath != "" {
		if _, err := os.Lstat(newPath); err == nil {
			return nil, errors.New("Patch destination already exists")
		}
	}
	if newPath != "" {
		c := change{path: newPath, after: []byte(updated)}
		if samePath {
			c.before = before
			info, err := os.Stat(newPath)
			if err != nil {
				return nil, err
			}
			c.permissions = info.Mode()
		}
		changes = append(changes, c)
	}
	if !samePath && oldPath != "" {
		info, err := os.Stat(oldPath)
		if err != nil {
			return nil, err
		}
		changes = append(changes, change{path: oldPath, before: before, permissions: info.Mode()})
	}
	return changes, nil
}

func hunkLines(h hunk) ([]string, []string) {
	var old, updated []string
	for _, line := range h.lines {
		text := line.text
		if line.newline {
			text += "\n"
		}
		if line.kind != '+' {
			old = append(old, text)
		}
		if line.kind != '-' {
			updated = append(updated, text)
		}
	}
	return old, updated
}

// parseHunkHeader checks the hunk header before requiring its file header.
func parseHunkHeader(line string, hasPatch bool) (hunk, error) {
	header := hunkHeader.FindStringSubmatch(line)
	if header == nil {
		return hunk{}, errors.New("Invalid patch hunk header")
	}
	if !hasPatch {
		return hunk{}, errors.New("Hunk before file header")
	}
	start, err := strconv.ParseUint(header[1], 10, 64)
	if err != nil {
		return hunk{}, errors.New("Invalid hunk start")
	}
	counts := [2]uint64{1, 1}
	for i := range counts {
		if header[i+2] != "" {
			counts[i], err = strconv.ParseUint(header[i+2], 10, 64)
			if err != nil {
				return hunk{}, errors.New("Invalid hunk count")
			}
		}
	}
	return hunk{start: start, oldCount: counts[0], newCount: counts[1]}, nil
}

func appendPatchLine(current *hunk, line string) error {
	switch {
	case len(line) > 0 && strings.ContainsRune(" -+", rune(line[0])):
		current.lines = append(current.lines, patchLine{line[0], line[1:], true})
	case line == `\ No newline at end of file`:
		if len(current.lines) == 0 {
			return errors.New("Newline marker without patch line")
		}
		current.lines[len(current.lines)-1].newline = false
	case line == "" || strings.HasPrefix(line, "diff ") || strings.HasPrefix(line, "index "):
	default:
		return fmt.Errorf("Invalid patch line: %s", line)
	}
	return nil
}

// hunkNeedsLines distinguishes header-looking content from the next file header.
func hunkNeedsLines(current *hunk) bool {
	var old, newCount uint64
	for _, line := range current.lines {
		if line.kind != '+' {
			old++
		}
		if line.kind != '-' {
			newCount++
		}
	}
	return old < current.oldCount || newCount < current.newCount
}

func lastPatchHunk(patches []filePatch) *hunk {
	if len(patches) == 0 {
		return nil
	}
	hunks := patches[len(patches)-1].hunks
	if len(hunks) == 0 {
		return nil
	}
	return &hunks[len(hunks)-1]
}
