package builtincommands

import (
	"bytes"
	"context"
	"errors"
	"fmt"
	"io/fs"
	"math"
	"os"
	"regexp"
	"slices"
	"strconv"
	"strings"

	"github.com/wspl/demi/go/builtinproto"
	"github.com/wspl/demi/go/commandservice"
)

// A patchFailure is why a patch does not apply; its text is what the agent
// reads.
type patchFailure struct {
	text string
}

func (f *patchFailure) Error() string { return f.text }

func patchFail(format string, args ...any) error {
	return &patchFailure{text: fmt.Sprintf(format, args...)}
}

// The reasons a patch does not apply, worded as the Rust words them.
var (
	errDeleteLeavesContent = patchFail("Delete patch leaves file content")
	errDestinationExists   = patchFail("Patch destination already exists")
	errPathTwice           = patchFail("Patch changes the same path more than once")
	errHunkOutOfRange      = patchFail("Patch hunk position is out of range")
	errHunkBeforeFile      = patchFail("Patch hunk starts before file")
	errHunkCounts          = patchFail("Patch hunk line counts do not match header")
	errNewBeforeOld        = patchFail("New header precedes old header")
	errHunkHeader          = patchFail("Invalid patch hunk header")
	errHunkCount           = patchFail("Invalid hunk count")
	errHunkBeforeHeader    = patchFail("Hunk before file header")
	errHunkStart           = patchFail("Invalid hunk start")
	errNewlineMarker       = patchFail("Newline marker without patch line")
	errIncomplete          = patchFail("Invalid patch: missing file headers or hunks")
)

// A filePatch is the changes to one file: it is created when its old path is
// /dev/null, and deleted when its new one is.
type filePatch struct {
	old, new *string
	hunks    []hunk
}

// A hunk is a run of changed lines, and where it starts.
type hunk struct {
	start, oldCount, newCount uint64
	lines                     []patchLine
}

// A patchLine is a line of a hunk: a context line (' '), a removed line ('-')
// or an added one ('+').
type patchLine struct {
	kind    byte
	text    string
	newline bool
}

// String returns the line as the file holds it.
func (l patchLine) String() string {
	if l.newline {
		return l.text + "\n"
	}
	return l.text
}

// A change is what a patch does to one path. The bytes are absent for a path the
// patch creates (before) or deletes (after).
type change struct {
	path        string
	before      optionalBytes
	after       optionalBytes
	permissions *fs.FileMode
}

// optionalBytes are the bytes of a file, or its absence.
type optionalBytes struct {
	present bool
	data    []byte
}

func (a optionalBytes) equal(b optionalBytes) bool {
	return a.present == b.present && bytes.Equal(a.data, b.data)
}

// patch applies a unified diff to one or more files.
func (h *Handler) patch(call *commandservice.Call, args builtinproto.PatchArgs) error {
	ctx := call.Context()
	cwd := call.Invocation.Cwd
	return h.mutate(call, func(recording *commandservice.Recording) (string, error) {
		return applyPatch(ctx, cwd, args.Patch, recording)
	})
}

// applyPatch plans every file's change, checks that they fit together, and then
// writes them all, or none: a write that fails restores the files written
// before it.
func applyPatch(ctx context.Context, cwd, diff string, recording *commandservice.Recording) (string, error) {
	patches, err := parsePatch(ctx, diff)
	if err != nil {
		return "", err
	}
	var changes []change
	for _, patch := range patches {
		if err := checkCancelled(ctx); err != nil {
			return "", err
		}
		planned, err := plan(ctx, cwd, patch)
		if err != nil {
			return "", err
		}
		changes = append(changes, planned...)
	}
	touched := map[string]bool{}
	for _, c := range changes {
		key := commandservice.PathKey(c.path)
		if touched[key] {
			return "", errPathTwice
		}
		touched[key] = true
	}
	changes = slices.DeleteFunc(changes, func(c change) bool { return c.before.equal(c.after) })
	for _, c := range changes {
		recording.Track(c.path)
	}
	if err := commitChanges(ctx, changes, writeChange, recording); err != nil {
		return "", err
	}
	return fmt.Sprintf("Patched %d file(s)\n", len(patches)), nil
}

// plan returns the changes that one file's patch makes: its new content at its
// new path, and the removal of its old path when it moved or was deleted.
func plan(ctx context.Context, cwd string, patch filePatch) ([]change, error) {
	oldPath, err := resolveOptional(cwd, patch.old)
	if err != nil {
		return nil, err
	}
	newPath, err := resolveOptional(cwd, patch.new)
	if err != nil {
		return nil, err
	}
	var before optionalBytes
	if oldPath != nil {
		data, err := os.ReadFile(*oldPath)
		if err != nil {
			return nil, system(err)
		}
		before = optionalBytes{present: true, data: data}
	}
	if err := checkUTF8(before.data); err != nil {
		return nil, err
	}
	label := patch.old
	if label == nil {
		label = patch.new
	}
	updated, err := applyHunks(ctx, string(before.data), patch.hunks)
	if err != nil {
		var failure *patchFailure
		if errors.As(err, &failure) {
			return nil, patchFail("Patch does not apply to %s: %s", *label, failure.text)
		}
		return nil, err
	}
	if newPath == nil && updated != "" {
		return nil, errDeleteLeavesContent
	}
	same := samePath(oldPath, newPath)
	if !same && newPath != nil {
		if _, err := os.Lstat(*newPath); err == nil {
			return nil, errDestinationExists
		}
	}
	var changes []change
	if newPath != nil {
		c := change{path: *newPath, after: optionalBytes{present: true, data: []byte(updated)}}
		if same {
			permissions, err := permissionsOf(*newPath)
			if err != nil {
				return nil, err
			}
			c.before = before
			c.permissions = &permissions
		}
		changes = append(changes, c)
	}
	if !same && oldPath != nil {
		permissions, err := permissionsOf(*oldPath)
		if err != nil {
			return nil, err
		}
		changes = append(changes, change{path: *oldPath, before: before, permissions: &permissions})
	}
	return changes, nil
}

// resolveOptional resolves a path a patch names, if it names one.
func resolveOptional(cwd string, path *string) (*string, error) {
	if path == nil {
		return nil, nil
	}
	resolved, err := commandservice.ResolvePath(cwd, *path)
	if err != nil {
		return nil, err
	}
	return &resolved, nil
}

// samePath reports whether two optional paths are the same path, or both absent,
// as Rust compares them.
func samePath(a, b *string) bool {
	if a == nil || b == nil {
		return a == nil && b == nil
	}
	return commandservice.PathKey(*a) == commandservice.PathKey(*b)
}

// permissionsOf returns the permissions of the file at path, which a change that
// writes or deletes it keeps or restores.
func permissionsOf(path string) (fs.FileMode, error) {
	info, err := os.Stat(path)
	if err != nil {
		return 0, system(err)
	}
	return info.Mode() & (fs.ModePerm | fs.ModeSetuid | fs.ModeSetgid | fs.ModeSticky), nil
}

// commitChanges writes the changes in order. When a write fails, the files
// written before it are restored, last first; the failure says what could not be
// restored too.
func commitChanges(ctx context.Context, changes []change, write func(change) error, recording *commandservice.Recording) error {
	for index, c := range changes {
		err := checkCancelled(ctx)
		if err == nil {
			err = write(c)
		}
		if err == nil {
			continue
		}
		var rollbacks []error
		for _, written := range slices.Backward(changes[:index]) {
			if err := restore(written); err != nil {
				rollbacks = append(rollbacks, err)
				continue
			}
			recording.Restored(written.path)
		}
		if len(rollbacks) == 0 {
			return err
		}
		return &rollbackFailure{err: err, rollbacks: rollbacks}
	}
	return nil
}

// A rollbackFailure is a write that failed and the failures to restore the files
// written before it. It is a reason to read, never a cancellation, whatever the
// write failed for.
type rollbackFailure struct {
	err       error
	rollbacks []error
}

func (f *rollbackFailure) Error() string {
	var text strings.Builder
	text.WriteString(f.err.Error())
	for _, rollback := range f.rollbacks {
		text.WriteString("\nRollback failed: ")
		text.WriteString(rollback.Error())
	}
	return text.String()
}

// writeChange writes a change: the new content, or the removal of a deleted file.
func writeChange(c change) error {
	if c.after.present {
		return atomicWrite(c.path, c.after.data, !c.before.present)
	}
	return system(os.Remove(c.path))
}

// restore undoes a change: it writes back what the file held, with its
// permissions, or removes a file the change created.
func restore(c change) error {
	if !c.before.present {
		return system(os.Remove(c.path))
	}
	if err := atomicWrite(c.path, c.before.data, !c.after.present); err != nil {
		return err
	}
	if c.permissions != nil {
		return system(os.Chmod(c.path, *c.permissions))
	}
	return nil
}

// applyHunks applies the hunks of a file's patch to its content: each must find
// the lines it removes or keeps where its header says, allowing for the lines
// the hunks before it added and removed.
func applyHunks(ctx context.Context, original string, hunks []hunk) (string, error) {
	lines := splitInclusive(original)
	var offset int64
	for _, h := range hunks {
		if err := checkCancelled(ctx); err != nil {
			return "", err
		}
		originalStart := h.start
		if h.oldCount != 0 {
			originalStart = max(h.start, 1) - 1
		}
		if originalStart > math.MaxInt64 {
			return "", errHunkOutOfRange
		}
		start := int64(originalStart)
		if offset > 0 && start > math.MaxInt64-offset {
			return "", errHunkOutOfRange
		}
		start += offset
		if start < 0 {
			return "", errHunkBeforeFile
		}
		var old, added []string
		for _, line := range h.lines {
			if line.kind != '+' {
				old = append(old, line.String())
			}
			if line.kind != '-' {
				added = append(added, line.String())
			}
		}
		if uint64(len(old)) != h.oldCount || uint64(len(added)) != h.newCount {
			return "", errHunkCounts
		}
		// The lines the hunk removes or keeps must be where its header says.
		if start > int64(len(lines)) || len(old) > len(lines)-int(start) {
			return "", patchFail("Patch does not apply at line %d", h.start)
		}
		end := int(start) + len(old)
		if !slices.Equal(lines[start:end], old) {
			return "", patchFail("Patch does not apply at line %d", h.start)
		}
		offset += int64(len(added)) - int64(len(old))
		lines = slices.Concat(lines[:start], added, lines[end:])
	}
	return strings.Join(lines, ""), nil
}

// splitInclusive splits text into its lines, each with its newline.
func splitInclusive(text string) []string {
	lines := strings.SplitAfter(text, "\n")
	if last := len(lines) - 1; lines[last] == "" {
		lines = lines[:last]
	}
	return lines
}

// The header of a hunk, and the date a header's file name may end with. Rust's
// regex reads \d and \s as Unicode classes; RE2's are ASCII, so the classes are
// written out.
var (
	hunkHeader = regexp.MustCompile(`^@@ -(\p{Nd}+)(?:,(\p{Nd}+))? \+\p{Nd}+(?:,(\p{Nd}+))? @@`)
	dateSuffix = regexp.MustCompile(`[\t-\r\x{85}\p{Z}]+\p{Nd}{4}-\p{Nd}{2}-\p{Nd}{2}(?:[ T]\p{Nd}{2}:\p{Nd}{2}:\p{Nd}{2}(?:\.\p{Nd}+)?(?:[\t-\r\x{85}\p{Z}]+[+-]\p{Nd}{4})?)?$`)
)

// parsePatch reads a unified diff: file headers, hunks and their lines. Lines
// before the first hunk that no header claims, such as diff and index lines, are
// ignored.
func parsePatch(ctx context.Context, diff string) ([]filePatch, error) {
	var patches []filePatch
	var pendingOld *string
	pending := false
	for _, line := range strings.Split(diff, "\n") {
		if err := checkCancelled(ctx); err != nil {
			return nil, err
		}
		if !pending && (strings.HasPrefix(line, "--- ") || strings.HasPrefix(line, "+++ ")) {
			// A line that looks like a header inside a hunk that still owes lines
			// is one of them: it removes or adds a line that starts with "-- " or
			// "++ ".
			if last := lastHunk(patches); last != nil && last.owes() {
				last.lines = append(last.lines, patchLine{kind: line[0], text: line[1:], newline: true})
				continue
			}
		}
		switch {
		case strings.HasPrefix(line, "--- "):
			pendingOld, pending = parsePath(line[4:]), true
		case strings.HasPrefix(line, "+++ "):
			if !pending {
				return nil, errNewBeforeOld
			}
			patches = append(patches, filePatch{old: pendingOld, new: parsePath(line[4:])})
			pendingOld, pending = nil, false
		case strings.HasPrefix(line, "@@ "):
			if err := addHunk(patches, line); err != nil {
				return nil, err
			}
		default:
			last := lastHunk(patches)
			if last == nil {
				continue
			}
			if err := addLine(last, line); err != nil {
				return nil, err
			}
		}
	}
	if len(patches) == 0 || pending || slices.ContainsFunc(patches, func(p filePatch) bool {
		return len(p.hunks) == 0 || p.old == nil && p.new == nil
	}) {
		return nil, errIncomplete
	}
	return patches, nil
}

// lastHunk returns the last hunk of the last file, if there is one.
func lastHunk(patches []filePatch) *hunk {
	if len(patches) == 0 {
		return nil
	}
	hunks := patches[len(patches)-1].hunks
	if len(hunks) == 0 {
		return nil
	}
	return &hunks[len(hunks)-1]
}

// owes reports whether the hunk has fewer lines than its header counts.
func (h *hunk) owes() bool {
	var old, added uint64
	for _, line := range h.lines {
		if line.kind != '+' {
			old++
		}
		if line.kind != '-' {
			added++
		}
	}
	return old < h.oldCount || added < h.newCount
}

// addHunk starts the hunk that a header line begins in the last file.
func addHunk(patches []filePatch, line string) error {
	match := hunkHeader.FindStringSubmatchIndex(line)
	if match == nil {
		return errHunkHeader
	}
	if len(patches) == 0 {
		return errHunkBeforeHeader
	}
	number := func(group int) (uint64, bool) {
		if match[2*group] < 0 {
			return 1, true
		}
		value, err := strconv.ParseUint(line[match[2*group]:match[2*group+1]], 10, 64)
		return value, err == nil
	}
	start, err := strconv.ParseUint(line[match[2]:match[3]], 10, 64)
	if err != nil {
		return errHunkStart
	}
	oldCount, ok := number(2)
	if !ok {
		return errHunkCount
	}
	newCount, ok := number(3)
	if !ok {
		return errHunkCount
	}
	file := &patches[len(patches)-1]
	file.hunks = append(file.hunks, hunk{start: start, oldCount: oldCount, newCount: newCount})
	return nil
}

// addLine adds a line of a hunk's body: a line of the file, a marker that the
// line before it has no newline, or a line git adds between files, which is
// ignored.
func addLine(h *hunk, line string) error {
	if line != "" && (line[0] == ' ' || line[0] == '-' || line[0] == '+') {
		h.lines = append(h.lines, patchLine{kind: line[0], text: line[1:], newline: true})
		return nil
	}
	switch {
	case line == `\ No newline at end of file`:
		if len(h.lines) == 0 {
			return errNewlineMarker
		}
		h.lines[len(h.lines)-1].newline = false
	case line == "", strings.HasPrefix(line, "diff "), strings.HasPrefix(line, "index "):
	default:
		return patchFail("Invalid patch line: %s", line)
	}
	return nil
}

// parsePath reads the file name of a header: without the time a diff appends to
// it, and without the a/ or b/ that git prefixes. /dev/null is no file.
func parsePath(value string) *string {
	value = strings.TrimSpace(value)
	value, _, _ = strings.Cut(value, "\t")
	if loc := dateSuffix.FindStringIndex(value); loc != nil {
		value = value[:loc[0]]
	}
	if value == "/dev/null" {
		return nil
	}
	if trimmed, ok := strings.CutPrefix(value, "a/"); ok {
		value = trimmed
	} else if trimmed, ok := strings.CutPrefix(value, "b/"); ok {
		value = trimmed
	}
	return &value
}

// checkUTF8 refuses content that is not text, with the words Rust uses for the
// first invalid byte.
func checkUTF8(data []byte) error {
	valid, length := utf8Fault(data)
	switch {
	case valid < 0:
		return nil
	case length == 0:
		return patchFail("incomplete utf-8 byte sequence from index %d", valid)
	}
	return patchFail("invalid utf-8 sequence of %d bytes from index %d", length, valid)
}

// utf8Fault finds the first invalid sequence of data as Rust's UTF-8 validation
// does. It returns the index where the valid text ends (negative when all of it
// is valid) and the length of the invalid sequence, which is 0 for a sequence
// the data ends in the middle of. The utf8 package is not enough: it says only
// whether data is valid, and decodes any invalid byte as a sequence of one, where
// the length of the sequence is part of the message the agent reads.
func utf8Fault(data []byte) (valid, length int) {
	continuation := func(i int) bool { return data[i]&0xC0 == 0x80 }
	for i := 0; i < len(data); {
		first := data[i]
		if first < 0x80 {
			i++
			continue
		}
		var second func(byte) bool
		width := 0
		switch {
		case first >= 0xC2 && first <= 0xDF:
			width = 2
			second = func(b byte) bool { return b&0xC0 == 0x80 }
		case first == 0xE0:
			width = 3
			second = func(b byte) bool { return b >= 0xA0 && b <= 0xBF }
		case first >= 0xE1 && first <= 0xEC, first == 0xEE, first == 0xEF:
			width = 3
			second = func(b byte) bool { return b&0xC0 == 0x80 }
		case first == 0xED:
			width = 3
			second = func(b byte) bool { return b >= 0x80 && b <= 0x9F }
		case first == 0xF0:
			width = 4
			second = func(b byte) bool { return b >= 0x90 && b <= 0xBF }
		case first >= 0xF1 && first <= 0xF3:
			width = 4
			second = func(b byte) bool { return b&0xC0 == 0x80 }
		case first == 0xF4:
			width = 4
			second = func(b byte) bool { return b >= 0x80 && b <= 0x8F }
		default:
			return i, 1
		}
		if i+1 >= len(data) {
			return i, 0
		}
		if !second(data[i+1]) {
			return i, 1
		}
		for next := 2; next < width; next++ {
			if i+next >= len(data) {
				return i, 0
			}
			if !continuation(i + next) {
				return i, next
			}
		}
		i += width
	}
	return -1, 0
}
