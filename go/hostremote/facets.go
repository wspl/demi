package hostremote

import (
	"context"
	"encoding/json/jsontext"
	"io"

	"github.com/wspl/demi/go/commandservice"
	"github.com/wspl/demi/go/runnerproto"
)

type LogPage struct {
	Lines []runnerproto.LogLine
	Next  uint64
}

func (h *RemoteHost) GitChanges(ctx context.Context, root string) (runnerproto.GitChanges, error) {
	link, err := h.link(ctx)
	if err != nil {
		return runnerproto.GitChanges{}, err
	}
	value, err := link.call(ctx, `Git("changes")`, func(id string) runnerproto.Inbound { return runnerproto.InboundGitChanges{ID: id, Root: root} })
	if err != nil {
		return runnerproto.GitChanges{}, err
	}
	result, ok := value.(runnerproto.GitResultChanges)
	if !ok {
		return runnerproto.GitChanges{}, protocolError("the runner answered another working-tree operation")
	}
	return result.Value, nil
}
func (h *RemoteHost) GitShow(ctx context.Context, root, path string) ([]byte, error) {
	link, err := h.link(ctx)
	if err != nil {
		return nil, err
	}
	reader, err := filled(ctx, link, `Git("show")`, func(id string, output runnerproto.PipeRef) runnerproto.Inbound {
		return runnerproto.InboundGitShow{ID: id, Root: root, Path: path, Output: output}
	})
	if err != nil {
		return nil, err
	}
	defer reader.Close()
	return collectPipe(ctx, reader, 0)
}
func (h *RemoteHost) ReadLog(ctx context.Context, since *uint64, limit uint64, source *string) (LogPage, error) {
	link, err := h.link(ctx)
	if err != nil {
		return LogPage{}, err
	}
	value, err := link.call(ctx, "Log", func(id string) runnerproto.Inbound {
		return runnerproto.InboundLogRead{ID: id, Since: since, Limit: limit, Source: source}
	})
	if err != nil {
		return LogPage{}, err
	}
	result, ok := value.(LogPage)
	if !ok {
		return LogPage{}, protocolError("the runner answered another request")
	}
	return result, nil
}
func (h *RemoteHost) OpenNet(ctx context.Context, host string, port uint16, input, output runnerproto.PipeRef) error {
	link, err := h.link(ctx)
	if err != nil {
		return err
	}
	release, err := h.admit(ctx)
	if err != nil {
		return err
	}
	defer release()
	_, err = link.call(ctx, "Net", func(id string) runnerproto.Inbound {
		return runnerproto.InboundNetOpen{StreamID: id, Host: host, Port: port, Input: input, Output: output}
	})
	return err
}

type ServiceRequest struct {
	Context   commandservice.CommandContext
	Package   commandservice.PackageDescriptor
	Operation string
	Args      *jsontext.Value
	JSON      *bool
	Cwd       string
	Resolver  ArtifactResolver
}
type ServiceStream struct {
	link  *Link
	id    string
	entry *serviceEntry
}

func (s *ServiceStream) Done(ctx context.Context) (ServiceEnd, error) {
	select {
	case <-ctx.Done():
		return ServiceEnd{}, ctx.Err()
	case answer := <-s.entry.done:
		return answer.end, answer.err
	}
}
func (s *ServiceStream) Close() error {
	s.link.mu.Lock()
	delete(s.link.services, s.id)
	s.link.mu.Unlock()
	s.entry.cancel()
	return nil
}
func (h *RemoteHost) OpenService(ctx context.Context, request ServiceRequest, input, output runnerproto.PipeRef) (*ServiceStream, error) {
	link, err := h.link(ctx)
	if err != nil {
		return nil, err
	}
	id := requestID()
	lifetime, cancel := context.WithCancel(link.ctx)
	entry := &serviceEntry{descriptor: request.Package, resolver: request.Resolver, ctx: lifetime, cancel: cancel, done: make(chan serviceAnswer, 1)}
	stream := &ServiceStream{link: link, id: id, entry: entry}
	link.mu.Lock()
	if link.IsClosed() {
		link.mu.Unlock()
		cancel()
		return nil, link.offline()
	}
	link.services[id] = entry
	link.mu.Unlock()
	_, err = link.callID(ctx, id, "Service", func(id string) runnerproto.Inbound {
		return runnerproto.InboundServiceOpen{StreamID: id, Context: request.Context, Package: request.Package, Operation: request.Operation, Args: request.Args, JSON: request.JSON, Cwd: request.Cwd, Input: input, Output: output}
	})
	if err != nil {
		stream.Close()
		return nil, err
	}
	return stream, nil
}
func (h *RemoteHost) CallService(ctx context.Context, request ServiceRequest, input []byte, maxBytes int) ([]byte, error) {
	link, err := h.link(ctx)
	if err != nil {
		return nil, err
	}
	to, from := link.pipes.ToDevice(link.device), link.pipes.FromDevice(link.device)
	defer to.Fail("service call failed")
	defer from.Fail("service call failed")
	writer, err := to.Writer()
	if err != nil {
		return nil, err
	}
	reader, err := from.Reader()
	if err != nil {
		return nil, err
	}
	defer reader.Close()
	stream, err := h.OpenService(ctx, request, to.WireRef(), from.WireRef())
	if err != nil {
		return nil, err
	}
	defer stream.Close()
	if _, err = writer.WriteContext(ctx, input); err != nil {
		return nil, err
	}
	writer.Close()
	var bytes []byte
	for {
		chunk, err := reader.Next(ctx)
		if err != nil {
			if err == io.EOF {
				break
			}
			return nil, err
		}
		if len(bytes)+len(chunk) > maxBytes {
			return nil, &ServiceCallError{Limit: &maxBytes}
		}
		bytes = append(bytes, chunk...)
	}
	end, err := stream.Done(ctx)
	if err != nil {
		return nil, err
	}
	if end.ExitCode != 0 {
		return nil, &ServiceCallError{ExitCode: end.ExitCode, Stderr: end.Stderr, Stdout: bytes}
	}
	return bytes, nil
}
func (h *RemoteHost) ReleaseConversation(ctx context.Context, conversation string) error {
	link, err := h.link(ctx)
	if err != nil {
		return nil
	}
	return link.ReleaseConversation(ctx, conversation)
}
