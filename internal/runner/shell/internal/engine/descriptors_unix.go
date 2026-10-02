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

	"github.com/wspl/demi/internal/cmdsdk"
)

// extraDescriptors forwards writable numbered outputs through the recorder.
// Read/write descriptors retain their native file semantics, as in the Rust host.
func extraDescriptors(ctx context.Context, launchesDone <-chan struct{}, cmd *exec.Cmd, files map[string]io.ReadWriteCloser) (func() error, error) {
	type forwarding struct {
		reader, writer *os.File
		target         *recordedFile
		err            error
	}
	var forwards []*forwarding
	copies := make(map[*recordedFile]*os.File)
	success := false
	defer func() {
		if !success {
			for _, copy := range forwards {
				_ = copy.reader.Close()
				_ = copy.writer.Close()
			}
		}
	}()
	for name, file := range files {
		n, err := strconv.Atoi(name)
		if err != nil || n < 3 {
			continue
		}
		f, ok := file.(*os.File)
		if tracked, yes := file.(*recordedFile); yes {
			f, ok = tracked.file, true
			if !tracked.readwrite {
				if existing := copies[tracked]; existing != nil {
					f = existing
				} else {
					pair, err := cmdsdk.Retry(ctx, func() ([2]*os.File, error) {
						r, w, err := os.Pipe()
						return [2]*os.File{r, w}, err
					})
					if err != nil {
						return nil, err
					}
					f = pair[1]
					copies[tracked] = f
					forwards = append(forwards, &forwarding{reader: pair[0], writer: pair[1], target: tracked})
					if cmd.Stdout == tracked {
						cmd.Stdout = f
					}
					if cmd.Stderr == tracked {
						cmd.Stderr = f
					}
				}
			}
		}
		if !ok {
			return nil, fmt.Errorf("descriptor %s has no native file", name)
		}
		for len(cmd.ExtraFiles) <= n-3 {
			cmd.ExtraFiles = append(cmd.ExtraFiles, nil)
		}
		cmd.ExtraFiles[n-3] = f
	}
	interrupted := make(chan struct{})
	stop := context.AfterFunc(ctx, func() {
		<-launchesDone
		for _, copy := range forwards {
			_ = copy.reader.Close()
			_ = copy.writer.Close()
		}
		close(interrupted)
	})
	var workers sync.WaitGroup
	for _, copy := range forwards {
		workers.Go(func() {
			_, copy.err = io.Copy(copy.target, copy.reader)
			_ = copy.reader.Close()
		})
	}
	success = true
	return sync.OnceValue(func() error {
		for _, copy := range forwards {
			_ = copy.writer.Close()
		}
		workers.Wait()
		if !stop() {
			<-interrupted
		}
		var err error
		for _, copy := range forwards {
			if !errors.Is(copy.err, os.ErrClosed) {
				err = errors.Join(err, copy.err)
			}
		}
		return err
	}), nil
}
