//go:build darwin || linux

package engine

import (
	"context"
	"errors"
	"fmt"
	"io"
	"os"
	"os/exec"
	"strconv"
	"sync"

	"github.com/wspl/demi/internal/commandsdk"
)

// extraDescriptors forwards writable numbered outputs through the recorder.
// Read/write descriptors keep their native file semantics.
func extraDescriptors(
	ctx context.Context,
	launchesDone <-chan struct{},
	cmd *exec.Cmd,
	files map[string]io.ReadWriteCloser,
) (func() error, error) {
	var forwards []*descriptorForwarding
	copies := make(map[*recordedFile]*os.File)
	success := false
	defer func() {
		if !success {
			for _, forward := range forwards {
				_ = forward.reader.Close()
				_ = forward.writer.Close()
			}
		}
	}()
	for name, file := range files {
		n, err := strconv.Atoi(name)
		if err != nil || n < 3 {
			continue
		}
		f, err := descriptorFile(ctx, cmd, name, file, copies, &forwards)
		if err != nil {
			return nil, err
		}
		for len(cmd.ExtraFiles) <= n-3 {
			cmd.ExtraFiles = append(cmd.ExtraFiles, nil)
		}
		cmd.ExtraFiles[n-3] = f
	}
	finish := forwardDescriptors(ctx, launchesDone, forwards)
	success = true
	return finish, nil
}

type descriptorForwarding struct {
	reader, writer *os.File
	target         *recordedFile
	err            error
}

// descriptorFile shares one forwarding pipe among aliases of a writable recorded file.
func descriptorFile(
	ctx context.Context,
	cmd *exec.Cmd,
	name string,
	file io.ReadWriteCloser,
	copies map[*recordedFile]*os.File,
	forwards *[]*descriptorForwarding,
) (*os.File, error) {
	tracked, ok := file.(*recordedFile)
	if !ok {
		native, ok := file.(*os.File)
		if !ok {
			return nil, fmt.Errorf("descriptor %s has no native file", name)
		}
		return native, nil
	}
	if tracked.readWrite {
		return tracked.file, nil
	}
	if existing := copies[tracked]; existing != nil {
		return existing, nil
	}
	pair, err := commandsdk.Retry(ctx, func() ([2]*os.File, error) {
		reader, writer, err := os.Pipe()
		return [2]*os.File{reader, writer}, err
	})
	if err != nil {
		return nil, err
	}
	native := pair[1]
	copies[tracked] = native
	*forwards = append(*forwards, &descriptorForwarding{reader: pair[0], writer: pair[1], target: tracked})
	if cmd.Stdout == tracked {
		cmd.Stdout = native
	}
	if cmd.Stderr == tracked {
		cmd.Stderr = native
	}
	return native, nil
}

// forwardDescriptors owns the forwarding workers until their caller joins them.
func forwardDescriptors(
	ctx context.Context,
	launchesDone <-chan struct{},
	forwards []*descriptorForwarding,
) func() error {
	interrupted := make(chan struct{})
	stop := context.AfterFunc(ctx, func() {
		<-launchesDone
		for _, forward := range forwards {
			_ = forward.reader.Close()
			_ = forward.writer.Close()
		}
		close(interrupted)
	})
	var workers sync.WaitGroup
	for _, forward := range forwards {
		workers.Go(func() {
			_, forward.err = io.Copy(forward.target, forward.reader)
			_ = forward.reader.Close()
		})
	}
	return sync.OnceValue(func() error {
		for _, forward := range forwards {
			_ = forward.writer.Close()
		}
		workers.Wait()
		if !stop() {
			<-interrupted
		}
		var err error
		for _, forward := range forwards {
			if !errors.Is(forward.err, os.ErrClosed) {
				err = errors.Join(err, forward.err)
			}
		}
		return err
	})
}
