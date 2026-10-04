package live

import (
	"context"
	"fmt"
	"log/slog"
	"os"
	"path/filepath"

	"github.com/wspl/demi/internal/commandpackage/browser/browserproto"
	"github.com/wspl/demi/internal/commandpackage/browser/chrome/cdp"
	"github.com/wspl/demi/internal/commandpackage/browser/chrome/tabs"
)

type uploadFile struct {
	path          string
	size, written uint64
	handle        *os.File
}
type upload struct {
	request *browserproto.LiveViewerMessageUpload
	files   []uploadFile
	current int
}

func (u *upload) close() {
	for i := range u.files {
		if f := u.files[i].handle; f != nil {
			if err := f.Close(); err != nil {
				slog.Warn("live upload close", "error", err)
			}
			u.files[i].handle = nil
		}
	}
}

func (u *upload) advance() error {
	for u.current < len(u.files) {
		f := &u.files[u.current]
		if f.written != f.size {
			break
		}
		if f.handle != nil {
			err := f.handle.Close()
			f.handle = nil
			if err != nil {
				return &cdp.BrowserError{Kind: cdp.KindIO, Cause: err}
			}
		}
		u.current++
	}
	return nil
}

func prepareUpload(directoryBase string, request *browserproto.LiveViewerMessageUpload) (*upload, error) {
	name, err := cdp.Fresh("u")
	if err != nil {
		return nil, err
	}
	directory := filepath.Join(directoryBase, name)
	if err := os.Mkdir(directory, 0o777); err != nil {
		return nil, &cdp.BrowserError{Kind: cdp.KindIO, Cause: err}
	}
	u := &upload{request: request}
	success := false
	defer func() {
		if !success {
			u.close()
		}
	}()
	for _, file := range request.Files {
		duplicate := false
		for _, existing := range u.files {
			if filepath.Base(existing.path) == file.Name {
				duplicate = true
				break
			}
		}
		if file.Name == "." || file.Name == ".." || duplicate {
			return nil, &cdp.BrowserError{
				Kind:    cdp.KindConfiguration,
				Message: fmt.Sprintf("invalid file name: %s", file.Name),
			}
		}
		path := filepath.Join(directory, file.Name)
		handle, err := os.Create(path)
		if err != nil {
			return nil, &cdp.BrowserError{Kind: cdp.KindIO, Cause: err}
		}
		u.files = append(u.files, uploadFile{path: path, size: file.Size, handle: handle})
	}
	if err := u.advance(); err != nil {
		return nil, err
	}
	success = true
	return u, nil
}

func (u *upload) receive(file uint32, data []byte) error {
	if u.current >= len(u.files) || uint64(file) != uint64(u.current) {
		return &cdp.BrowserError{Kind: cdp.KindConfiguration, Message: "file bytes arrived out of order"}
	}
	entry := &u.files[u.current]
	if uint64(len(data)) > entry.size-entry.written {
		return &cdp.BrowserError{Kind: cdp.KindConfiguration, Message: "a file is larger than announced"}
	}
	if _, err := entry.handle.Write(data); err != nil {
		return &cdp.BrowserError{Kind: cdp.KindIO, Cause: err}
	}
	entry.written += uint64(len(data))
	return u.advance()
}

func runUploads(
	ctx context.Context,
	environment *tabs.Environment,
	member *membership,
	w *writer,
	items <-chan inbound,
) {
	var pending *upload
	defer func() {
		if pending != nil {
			pending.close()
		}
	}()
	refuse := func(token browserproto.ControlToken, err error) {
		w.notice(ctx, string(cdp.ErrorCode(err)), err.Error())
		w.control(ctx, &browserproto.LiveModuleMessageChoice{Token: token, Accepted: false})
	}
	for {
		if ctx.Err() != nil {
			return
		}
		item, ok := nextUpload(ctx, items)
		if !ok {
			return
		}
		var err error
		start, announced := item.message.(*browserproto.LiveViewerMessageUpload)
		if announced && member.operated(ctx) != nil {
			return
		}
		if announced {
			if pending != nil {
				pending.close()
			}
			pending, err = prepareUpload(environment.UploadDirectory(), start)
			if err != nil {
				refuse(start.Token, err)
				continue
			}
		} else {
			var received bool
			received, err = receiveUpload(pending, item)
			if !received {
				continue
			}
		}
		if pending == nil {
			continue
		}
		if err != nil {
			pending = refuseUpload(pending, err, refuse)
			continue
		}
		if pending.current != len(pending.files) {
			continue
		}
		done := pending
		pending = nil
		finishUpload(ctx, environment, w, done, refuse)
	}
}

func finishUpload(
	ctx context.Context,
	environment *tabs.Environment,
	w *writer,
	done *upload,
	refuse func(browserproto.ControlToken, error),
) {
	paths := make([]string, len(done.files))
	for i, f := range done.files {
		paths[i] = f.path
	}
	tab, err := environment.Tab(ctx, done.request.Tab, cdp.ControlTimeout)
	accepted := false
	if err == nil {
		accepted, err = attach(ctx, tab, done.request.Token, done.request.Revision, paths)
	}
	if err != nil {
		refuse(done.request.Token, err)
	} else {
		w.control(ctx, &browserproto.LiveModuleMessageChoice{Token: done.request.Token, Accepted: accepted})
	}
}

func refuseUpload(pending *upload, err error, refuse func(browserproto.ControlToken, error)) *upload {
	refuse(pending.request.Token, err)
	pending.close()
	pending = nil
	return pending
}

func receiveUpload(pending *upload, item inbound) (bool, error) {
	if pending == nil || item.file.Upload != pending.request.Upload {
		return false, nil
	}
	return true, pending.receive(item.file.File, item.data)
}

func nextUpload(ctx context.Context, items <-chan inbound) (inbound, bool) {
	var item inbound
	select {
	case <-ctx.Done():
		return inbound{}, false
	case value, ok := <-items:
		if !ok {
			return inbound{}, false
		}
		item = value
	}
	return item, true
}
