//go:build darwin

package host

import (
	"context"
	"fmt"
	"path/filepath"
	"strings"
	"sync"
	"sync/atomic"
	"unsafe"

	"github.com/ebitengine/purego"
)

// Native signatures and constants follow the installed macOS SDK FSEvents.h
// and CoreFoundation headers. Unsafe is confined to the documented C ABI:
// purego cannot infer native pointer lengths or verify function signatures.
var native struct {
	arrayCallbacks                  uintptr
	once                            sync.Once
	err                             error
	stringCreate                    func(uintptr, string, uint32) uintptr
	arrayCreate                     func(uintptr, *uintptr, int64, uintptr) uintptr
	release                         func(uintptr)
	create                          func(uintptr, uintptr, *streamContext, uintptr, uint64, float64, uint32) uintptr
	start                           func(uintptr) bool
	stop, invalidate, streamRelease func(uintptr)
	setQueue                        func(uintptr, uintptr)
	queueCreate                     func(string, uintptr) uintptr
	queueRelease                    func(uintptr)
	syncQueue                       func(uintptr, uintptr, uintptr)
	strlen                          func(*byte) uintptr
	callback, barrier               uintptr
}

type streamContext struct {
	_    uintptr // version zero
	info uintptr
	_    [3]uintptr // nil retain, release, and description callbacks
}
type nativeWatch struct {
	stream, queue, id uintptr
	report            func(WatchEvent)
}

var (
	watches       sync.Map
	nextID        atomic.Uint64
	activeStreams atomic.Int64
)

// loadNative binds the FSEvents and dispatch APIs once for all tree watches.
func loadNative() error {
	native.once.Do(func() {
		bindings := []struct {
			path    string
			symbols map[string]any
		}{
			{
				"/System/Library/Frameworks/CoreFoundation.framework/CoreFoundation",
				map[string]any{
					"CFStringCreateWithCString": &native.stringCreate,
					"CFArrayCreate":             &native.arrayCreate,
					"CFRelease":                 &native.release,
				},
			},
			{
				"/System/Library/Frameworks/CoreServices.framework/CoreServices",
				map[string]any{
					"FSEventStreamCreate":           &native.create,
					"FSEventStreamStart":            &native.start,
					"FSEventStreamStop":             &native.stop,
					"FSEventStreamInvalidate":       &native.invalidate,
					"FSEventStreamRelease":          &native.streamRelease,
					"FSEventStreamSetDispatchQueue": &native.setQueue,
				},
			},
			{"/usr/lib/libSystem.B.dylib", map[string]any{
				"dispatch_queue_create": &native.queueCreate, "dispatch_release": &native.queueRelease,
				"dispatch_sync_f": &native.syncQueue, "strlen": &native.strlen,
			}},
		}
		for _, b := range bindings {
			h, err := purego.Dlopen(b.path, purego.RTLD_NOW|purego.RTLD_LOCAL)
			if err != nil {
				native.err = err
				return
			}
			if strings.Contains(b.path, "CoreFoundation.framework") {
				native.arrayCallbacks, err = purego.Dlsym(h, "kCFTypeArrayCallBacks")
				if err != nil {
					native.err = err
					return
				}
			}
			// Framework handles intentionally live for the process, like the two
			// callback trampolines; no per-watch dlopen or NewCallback allocation.
			for name, target := range b.symbols {
				sym, err := purego.Dlsym(h, name)
				if err != nil {
					native.err = err
					return
				}
				purego.RegisterFunc(target, sym)
			}
		}
		native.callback = purego.NewCallback(deliver)
		native.barrier = purego.NewCallback(func(uintptr) {})
	})
	return native.err
}

// startPlatformWatch watches one directory tree and delivers copied paths without blocking dispatch.
func startPlatformWatch(ctx context.Context, trees []string, report func(WatchEvent)) (func(), error) {
	if err := loadNative(); err != nil {
		return nil, err
	}
	var stringsToWatch []uintptr
	defer func() {
		for _, str := range stringsToWatch {
			native.release(str)
		}
	}()
	for _, root := range trees {
		if err := ctx.Err(); err != nil {
			return nil, err
		}
		str, err := watchRootString(root)
		if err != nil {
			return nil, err
		}
		stringsToWatch = append(stringsToWatch, str)
	}
	if len(stringsToWatch) == 0 {
		return nil, fmt.Errorf("no watch roots")
	}
	arr := native.arrayCreate(0, &stringsToWatch[0], int64(len(stringsToWatch)), native.arrayCallbacks)
	if arr == 0 {
		return nil, fmt.Errorf("CFArrayCreate failed")
	}
	defer native.release(arr)
	w := &nativeWatch{id: uintptr(nextID.Add(1)), report: report}
	watches.Store(w.id, w)
	nativeContext := streamContext{info: w.id}
	w.stream = native.create(0, native.callback, &nativeContext, arr, ^uint64(0), 0.01, 0x10|0x02|0x04)
	if w.stream == 0 {
		watches.Delete(w.id)
		return nil, fmt.Errorf("FSEventStreamCreate failed")
	}
	activeStreams.Add(1)
	w.queue = native.queueCreate("demi.host.fsevents", 0)
	if w.queue == 0 {
		native.streamRelease(w.stream)
		activeStreams.Add(-1)
		watches.Delete(w.id)
		return nil, fmt.Errorf("dispatch_queue_create failed")
	}
	native.setQueue(w.stream, w.queue)
	started := native.start(w.stream)
	if !started {
		w.disposeScheduledStream()
		return nil, fmt.Errorf("FSEventStreamStart failed")
	}
	return func() {
		native.stop(w.stream)
		w.disposeScheduledStream()
	}, nil
}

// disposeScheduledStream releases a scheduled tree stream after startup failure or stop.
func (w *nativeWatch) disposeScheduledStream() {
	native.invalidate(w.stream)
	native.syncQueue(w.queue, 0, native.barrier)
	native.streamRelease(w.stream)
	native.queueRelease(w.queue)
	watches.Delete(w.id)
	activeStreams.Add(-1)
}

// deliver converts the FSEvents callback's borrowed arrays into owned Go events.
func deliver(_ uintptr, id uintptr, count uintptr, paths **byte, flags *uint32, _ *uint64) {
	value, ok := watches.Load(id)
	if !ok {
		return // Callback ownership has ended after its serial queue was drained.
	}
	w := value.(*nativeWatch)
	ps := unsafe.Slice(paths, count)
	fs := unsafe.Slice(flags, count)
	for i, p := range ps {
		path := string(unsafe.Slice(p, native.strlen(p)))
		flag := fs[i]
		if flag&(0x01|0x02|0x04|0x20|0x40|0x80) != 0 {
			w.report(WatchEvent{Kind: WatchLost})
			continue
		}
		// FSEvents.h: created/removed/renamed/modified versus inode/Finder/owner/xattr.
		content := flag&(0x100|0x200|0x800|0x1000) != 0
		metadata := flag&(0x400|0x2000|0x4000|0x8000) != 0 && !content
		w.report(WatchEvent{Kind: WatchChanged, Path: path, Metadata: metadata})
	}
}

func watchRootString(path string) (uintptr, error) {
	root, err := filepath.EvalSymlinks(path)
	if err != nil {
		return 0, err
	}
	root, err = filepath.Abs(root)
	if err != nil {
		return 0, err
	}
	if strings.ContainsRune(root, 0) {
		return 0, fmt.Errorf("NUL in root")
	}
	str := native.stringCreate(0, root, 0x08000100)
	if str == 0 {
		return 0, fmt.Errorf("CFStringCreateWithCString failed")
	}
	return str, nil
}
