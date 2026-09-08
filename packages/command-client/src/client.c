#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <uv.h>
#ifdef _WIN32
#include <windows.h>
#include <shellapi.h>
#endif
#include "metadata.h"
#include "contract.h"

typedef struct {
    uv_pipe_t pipe;
    uv_connect_t connect;
    unsigned char *buffer;
    size_t length;
    int control;
} Connection;
static uv_loop_t loop;
static Connection data_peer, control_peer;
static uv_fs_t input_req, output_req;
static uv_signal_t interrupt_signal, terminate_signal;
static unsigned char input[DEMI_CHUNK_SIZE];
static unsigned char *output;
static size_t output_length, output_offset;
static int output_fd, input_pending, input_ended, waiting_watch;
static char *endpoint, *context, *metadata;
static char call_id[33];
typedef struct { union { uv_pipe_t pipe; uv_tty_t tty; uv_tcp_t tcp; } handle; uv_stream_t *stream; int initialized; } Stdio;
static Stdio stdio_handles[3];
static uv_write_t output_write;
static uv_stream_t *stdio_stream(int fd) {
    Stdio *io = &stdio_handles[fd];
    if (io->initialized) return io->stream;
    io->initialized = 1;
    uv_handle_type type = uv_guess_handle(fd);
    int status = 0;
    if (type == UV_TTY) { status = uv_tty_init(&loop, &io->handle.tty, fd, fd == 0); io->stream = (uv_stream_t *)&io->handle.tty; }
    else if (type == UV_NAMED_PIPE) {
        status = uv_pipe_init(&loop, &io->handle.pipe, 0);
        if (!status) status = uv_pipe_open(&io->handle.pipe, fd);
        io->stream = (uv_stream_t *)&io->handle.pipe;
    } else if (type == UV_TCP) {
        status = uv_tcp_init(&loop, &io->handle.tcp);
        if (!status) status = uv_tcp_open(&io->handle.tcp, fd);
        io->stream = (uv_stream_t *)&io->handle.tcp;
    } else if (type != UV_FILE) { fprintf(stderr, "demi: unsupported stdio handle\n"); exit(1); }
    if (status < 0) { fprintf(stderr, "demi: %s\n", uv_strerror(status)); exit(1); }
    return io->stream;
}
static void parse(Connection *peer);
static void fail(const char *message) { fprintf(stderr, "demi: %s\n", message); exit(1); }
static void check(int status) { if (status < 0) fail(uv_strerror(status)); }
static unsigned int u32(const unsigned char *p) { return ((unsigned)p[0]<<24)|((unsigned)p[1]<<16)|((unsigned)p[2]<<8)|p[3]; }
static void put32(unsigned char *p, unsigned int n) { p[0]=(unsigned char)(n>>24); p[1]=(unsigned char)(n>>16); p[2]=(unsigned char)(n>>8); p[3]=(unsigned char)n; }
typedef struct { uv_write_t req; unsigned char bytes[]; } Write;
static void wrote(uv_write_t *req, int status) { free(req); check(status); }
static void send_frame(Connection *peer, unsigned type, const void *body, size_t length) {
    if (length > DEMI_MAX_FRAME) fail("frame too large");
    Write *w = malloc(sizeof(*w) + 5 + length);
    if (!w) fail("out of memory");
    put32(w->bytes, (unsigned)length); w->bytes[4] = (unsigned char)type;
    if (length) memcpy(w->bytes+5, body, length);
    uv_buf_t buf = uv_buf_init((char *)w->bytes, (unsigned)(5 + length));
    int status = uv_write(&w->req, (uv_stream_t *)&peer->pipe, &buf, 1, wrote);
    if (status < 0) { free(w); check(status); }
}
static void input_read(uv_fs_t *req) {
    ssize_t n = req->result; uv_fs_req_cleanup(req); input_pending = 0;
    if (n < 0) check((int)n);
    if (!n) { input_ended = 1; send_frame(&data_peer, DEMI_INPUT_END, NULL, 0); }
    else send_frame(&data_peer, DEMI_INPUT, input, (size_t)n);
}
static void input_alloc(uv_handle_t *handle, size_t suggested, uv_buf_t *buf) {
    (void)handle; (void)suggested; *buf = uv_buf_init((char *)input, sizeof(input));
}
static void input_stream_read(uv_stream_t *stream, ssize_t n, const uv_buf_t *buf) {
    (void)buf;
    if (!n) return;
    uv_read_stop(stream); input_pending = 0;
    if (n == UV_EOF) { input_ended = 1; send_frame(&data_peer, DEMI_INPUT_END, NULL, 0); }
    else { check((int)n); send_frame(&data_peer, DEMI_INPUT, input, (size_t)n); }
}
static void pull_input(void) {
    if (input_pending || input_ended) fail("invalid input demand");
    input_pending = 1;
    uv_stream_t *stream = stdio_stream(0);
    if (stream) { check(uv_read_start(stream, input_alloc, input_stream_read)); return; }
    uv_buf_t buf = uv_buf_init((char *)input, sizeof(input));
    check(uv_fs_read(&loop, &input_req, 0, &buf, 1, -1, input_read));
}
static void write_output(void);
static void output_written(uv_fs_t *req) {
    ssize_t n = req->result; uv_fs_req_cleanup(req);
    if (n <= 0) fail(n ? uv_strerror((int)n) : "output made no progress");
    output_offset += (size_t)n;
    if (output_offset < output_length) { write_output(); return; }
    free(output); output = NULL;
    parse(&data_peer);
}
static void output_stream_written(uv_write_t *req, int status) {
    (void)req; check(status); free(output); output = NULL; parse(&data_peer);
}
static void write_output(void) {
    uv_buf_t buf = uv_buf_init((char *)output + output_offset, (unsigned)(output_length-output_offset));
    uv_stream_t *stream = stdio_stream(output_fd);
    if (stream) check(uv_write(&output_write, stream, &buf, 1, output_stream_written));
    else check(uv_fs_write(&loop, &output_req, output_fd, &buf, 1, -1, output_written));
}
static void alloc_read(uv_handle_t *handle, size_t suggested, uv_buf_t *buf) {
    (void)handle; (void)suggested;
    *buf = uv_buf_init(malloc(DEMI_CHUNK_SIZE), DEMI_CHUNK_SIZE);
    if (!buf->base) fail("out of memory");
}
static void received(uv_stream_t *stream, ssize_t n, const uv_buf_t *buf) {
    Connection *peer = stream->data;
    if (n < 0) { free(buf->base); fail(n == UV_EOF ? "runner disconnected before exit" : uv_strerror((int)n)); }
    if (!n) { free(buf->base); return; }
    if (n > 0) {
        uv_read_stop(stream);
        if (peer->length + (size_t)n > DEMI_MAX_FRAME + DEMI_CHUNK_SIZE + 5) fail("receive buffer exceeded");
        unsigned char *next = realloc(peer->buffer, peer->length + (size_t)n);
        if (!next) fail("out of memory");
        peer->buffer = next; memcpy(next + peer->length, buf->base, (size_t)n); peer->length += (size_t)n;
    }
    free(buf->base);
    parse(peer);
}
static void control_connected(uv_connect_t *req, int status) {
    (void)req; check(status);
    check(uv_read_start((uv_stream_t *)&control_peer.pipe, alloc_read, received));
    char json[180];
    snprintf(json, sizeof(json), "{\"version\":%d,\"id\":\"%s\",\"context\":\"%s\"}", DEMI_VERSION, call_id, context);
    send_frame(&control_peer, DEMI_WATCH, json, strlen(json));
}
static void parse(Connection *peer) {
    while (peer->length >= 5 && !(peer == &data_peer && output)) {
        unsigned length = u32(peer->buffer), type = peer->buffer[4];
        if (length > DEMI_MAX_FRAME) fail("frame exceeds limit");
        if (peer->length < (size_t)length + 5) break;
        unsigned char *body = peer->buffer + 5;
        if (type == DEMI_ERROR) { fwrite(body, 1, length, stderr); fputc('\n', stderr); exit(1); }
        if (!peer->control && !control_peer.pipe.loop && type != DEMI_READY) fail("runner handshake required");
        if (peer->control) {
            if (type != DEMI_READY || length || !waiting_watch) fail("invalid control response");
            waiting_watch = 0;
        } else if (type == DEMI_READY) {
            if (length || waiting_watch || control_peer.pipe.loop) fail("duplicate ready");
            waiting_watch = 1;
            check(uv_pipe_init(&loop, &control_peer.pipe, 0));
            control_peer.control = 1; control_peer.pipe.data = &control_peer;
            uv_pipe_connect(&control_peer.connect, &control_peer.pipe, endpoint, control_connected);
        } else if (type == DEMI_PULL) {
            if (length) fail("invalid input request");
            pull_input();
        } else if (type == DEMI_STDOUT || type == DEMI_STDERR) {
            if (length > DEMI_CHUNK_SIZE) fail("output chunk exceeds limit");
            if (length) {
                output = malloc(length); if (!output) fail("out of memory");
                memcpy(output, body, length); output_length = length; output_offset = 0;
                output_fd = type == DEMI_STDOUT ? 1 : 2;
            }
        } else if (type == DEMI_EXIT) {
            if (length != 4 || u32(body) > 255) fail("invalid exit status");
            exit((int)u32(body));
        } else fail("unexpected response");
        memmove(peer->buffer, peer->buffer + 5 + length, peer->length - 5 - length);
        peer->length -= 5 + length;
        if (output && peer == &data_peer) { write_output(); return; }
    }
    check(uv_read_start((uv_stream_t *)&peer->pipe, alloc_read, received));
}
static void connected(uv_connect_t *req, int status) {
    (void)req; check(status);
    check(uv_read_start((uv_stream_t *)&data_peer.pipe, alloc_read, received));
    send_frame(&data_peer, DEMI_INVOKE, metadata, strlen(metadata));
    free(metadata); metadata = NULL;
}
static void interrupted(uv_signal_t *handle, int signum) { (void)handle; exit(128 + signum); }
static char *environment(const char *name) {
    size_t size = 65536; char *value = malloc(size);
    if (!value) fail("out of memory");
    if (uv_os_getenv(name, value, &size) || !value[0]) { free(value); return NULL; }
    return value;
}
int main(int argc, char **argv) {
#ifdef _WIN32
    int wide_count; wchar_t **wide = CommandLineToArgvW(GetCommandLineW(), &wide_count);
    if (!wide) return 1;
    argc = wide_count; argv = calloc((size_t)argc + 1, sizeof(char *));
    if (!argv) return 1;
    for (int i=0; i<argc; ++i) {
        int n = WideCharToMultiByte(CP_UTF8, WC_ERR_INVALID_CHARS, wide[i], -1, NULL, 0, NULL, NULL);
        if (!n || !(argv[i]=malloc((size_t)n))) return 1;
        if (!WideCharToMultiByte(CP_UTF8, WC_ERR_INVALID_CHARS, wide[i], -1, argv[i], n, NULL, NULL)) return 1;
    }
    LocalFree(wide);
#endif
    endpoint = environment(DEMI_ENDPOINT_ENV); context = environment(DEMI_CONTEXT_ENV);
    if (!endpoint || !context) fail("no execution context; run this command inside a runner job");
    if (strlen(context) != 32 || strspn(context, "0123456789abcdef") != 32) fail("invalid execution context");
    unsigned char random[16]; check(uv_random(NULL, NULL, random, sizeof(random), 0, NULL));
    for (int i=0; i<16; ++i) snprintf(call_id+i*2, 3, "%02x", random[i]);
    metadata = demi_metadata(argc, argv, call_id, context);
    check(uv_loop_init(&loop));
    check(uv_signal_init(&loop, &interrupt_signal)); check(uv_signal_start(&interrupt_signal, interrupted, SIGINT));
    check(uv_signal_init(&loop, &terminate_signal)); check(uv_signal_start(&terminate_signal, interrupted, SIGTERM));
    check(uv_pipe_init(&loop, &data_peer.pipe, 0)); data_peer.pipe.data = &data_peer;
    uv_pipe_connect(&data_peer.connect, &data_peer.pipe, endpoint, connected);
    uv_run(&loop, UV_RUN_DEFAULT);
    fail("runner did not return an exit status");
    return 1;
}
