package host

import (
	"bytes"
	"context"
	"errors"
	"io"
	"os"
	"path/filepath"
	"syscall"

	"github.com/go-git/go-git/v5/plumbing/object"

	"github.com/wspl/demi/internal/cmdsdk"
	"github.com/wspl/demi/internal/runner/process"

	"github.com/wspl/demi/internal/runnerwire"
)

// MaxFiles is where the change list stops and reports truncated.
const MaxFiles = 5000

// MaxBlobBytes is the size beyond which files count no lines and GitShow refuses them.
const MaxBlobBytes = 8 * 1024 * 1024

// GitChanges reports uncommitted paths under the requested root, relative to
// that root, with status letters and HEAD-to-disk line counts. Nonrepositories
// receive an empty successful result. The service keeps at most eight watches,
// expires them after fifteen idle minutes, and shares computations per root.
// Computation has a thirty-second running deadline. Unavailable or failed
// watches cause whole walks; lost events invalidate the watched baseline.
func (s *Service) GitChanges(ctx context.Context, request runnerwire.GitChangesMessage) error {
	ctx, leave, err := s.life.enter(ctx)
	if err != nil {
		return err
	}
	defer leave()
	if err = admit(ctx, s.gitRequests); err != nil {
		return err
	}
	defer func() { <-s.gitRequests }()
	root, failure := s.resolve(nil, request.Root)
	if failure == nil {
		root, failure = filepath.EvalSymlinks(root)
	}
	if failure == nil {
		root, failure = filepath.Abs(root)
	}
	var result runnerwire.GitChanges
	if failure == nil {
		result, failure = s.git.changes(ctx, root)
	}
	return s.gitReply(s.life.ctx, request.ID, &runnerwire.GitChangesResult{Value: result}, failure)
}

// GitShow decodes a committed blob before replying and streams it whole into
// the output pipe after the reply. The working-tree permit covers decoding,
// not uploading. Oversized blobs answer too_large; missing paths answer ENOENT;
// a root outside a repository answers not_repository. Every named pipe ends
// with pipe_done, including a pipe unused because the request failed.
func (s *Service) GitShow(ctx context.Context, request runnerwire.GitShow) error {
	ctx, leave, err := s.life.enter(ctx)
	if err != nil {
		return err
	}
	defer leave()
	var data []byte
	failure := admit(ctx, s.gitRequests)
	if failure == nil {
		data, failure = cmdsdk.Retry(ctx, func() ([]byte, error) { return s.showBlob(ctx, request.Root, request.Path) })
		<-s.gitRequests
	}
	if err = s.gitReply(s.life.ctx, request.ID, &runnerwire.GitShowResult{}, failure); err != nil {
		return err
	}
	if failure == nil {
		failure = s.pipes.Put(ctx, request.Output.URL, io.NopCloser(bytes.NewReader(data)))
	}
	return process.ReportPipe(s.life.ctx, s.output, request.Output.ID, failure)
}

// gitReply applies the runner's bounded message size to a working-tree response.
func (s *Service) gitReply(ctx context.Context, id string, result runnerwire.GitResult, failure error) error {
	if failure != nil {
		problem := gitProblem(failure)
		return sendFrame(ctx, s.output, &runnerwire.GitError{ID: id, Code: problem.code, Message: problem.message})
	}
	frame, err := runnerwire.Encode(&runnerwire.GitOK{ID: id, Result: result})
	if err == nil {
		frame, err = runnerwire.WithinLimit(frame, func(reason string) ([]byte, error) {
			return runnerwire.Encode(&runnerwire.GitError{ID: id, Code: "too_large", Message: reason})
		})
	}
	if err != nil {
		return err
	}
	select {
	case <-ctx.Done():
		return ctx.Err()
	case s.output <- frame:
		return nil
	}
}

// showBlob decodes a committed blob whole before its pipe is acknowledged.
func (s *Service) showBlob(ctx context.Context, root, path string) (data []byte, err error) {
	defer func() {
		if recover() != nil {
			err = &gitFailure{code: "internal", message: "working-tree work panicked"}
		}
	}()
	if err = ctx.Err(); err != nil {
		return nil, err
	}
	root, err = s.resolve(nil, root)
	if err != nil {
		return nil, err
	}
	repo, location, err := openRepository(root)
	if err != nil {
		return nil, err
	}
	if location == nil {
		return nil, &gitFailure{code: "not_repository", message: "not inside a git repository"}
	}
	defer func() { err = errors.Join(err, closeRepository(repo)) }()
	tree, _, _, err := headEntries(ctx, repo)
	if err != nil {
		return nil, err
	}
	if tree == nil {
		return nil, os.ErrNotExist
	}
	name := path
	if location.prefix != "" {
		name = location.prefix + "/" + path
	}
	entry, err := tree.FindEntry(name)
	if errors.Is(err, object.ErrEntryNotFound) {
		return nil, os.ErrNotExist
	}
	if err != nil {
		return nil, err
	}
	content, err := blobContent(ctx, repo, *entry)
	if err != nil {
		return nil, err
	}
	if !content.present {
		return nil, syscall.EISDIR
	}
	if content.large {
		return nil, &gitFailure{code: "too_large", message: "the file is too large"}
	}
	return content.bytes, nil
}
