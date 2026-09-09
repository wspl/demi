/* Transport-size probe only. libuv selects UDS or Windows named pipes. */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <uv.h>

static uv_loop_t loop;
static uv_pipe_t peer;
static uv_fs_t input_req;
static uv_write_t write_req;
static uv_shutdown_t shutdown_req;
static char input[65536];
static void fail(int status) {
  fprintf(stderr, "%s\n", uv_strerror(status));
  exit(1);
}
static void check(int status) {
  if (status < 0)
    fail(status);
}
static void read_input(void);
static void sent(uv_write_t *req, int status) {
  (void)req;
  check(status);
  read_input();
}
static void shut(uv_shutdown_t *req, int status) {
  (void)req;
  check(status);
}
static void input_read(uv_fs_t *req) {
  ssize_t n = req->result;
  uv_fs_req_cleanup(req);
  if (n < 0)
    fail((int)n);
  if (!n) {
    check(uv_shutdown(&shutdown_req, (uv_stream_t *)&peer, shut));
    return;
  }
  uv_buf_t buf = uv_buf_init(input, (unsigned)n);
  check(uv_write(&write_req, (uv_stream_t *)&peer, &buf, 1, sent));
}
static void read_input(void) {
  uv_buf_t buf = uv_buf_init(input, sizeof(input));
  check(uv_fs_read(&loop, &input_req, 0, &buf, 1, -1, input_read));
}
static void alloc_read(uv_handle_t *handle, size_t suggested, uv_buf_t *buf) {
  (void)handle;
  (void)suggested;
  *buf = uv_buf_init(malloc(65536), 65536);
  if (!buf->base)
    exit(1);
}
static void received(uv_stream_t *stream, ssize_t n, const uv_buf_t *buf) {
  (void)stream;
  if (n == UV_EOF) {
    free(buf->base);
    exit(0);
  }
  if (n < 0)
    fail((int)n);
  size_t offset = 0;
  while (offset < (size_t)n) {
    uv_fs_t req;
    uv_buf_t out =
        uv_buf_init(buf->base + offset, (unsigned)((size_t)n - offset));
    int written = uv_fs_write(&loop, &req, 1, &out, 1, -1, NULL);
    uv_fs_req_cleanup(&req);
    if (written <= 0)
      fail(written ? written : UV_EIO);
    offset += (size_t)written;
  }
  free(buf->base);
}
static void connected(uv_connect_t *req, int status) {
  (void)req;
  check(status);
  check(uv_read_start((uv_stream_t *)&peer, alloc_read, received));
  read_input();
}
int main(int argc, char **argv) {
  if (argc != 2) {
    fprintf(stderr, "Usage: ipc-client endpoint\n");
    return 2;
  }
  check(uv_loop_init(&loop));
  check(uv_pipe_init(&loop, &peer, 0));
  uv_connect_t connect_req;
  uv_pipe_connect(&connect_req, &peer, argv[1], connected);
  uv_run(&loop, UV_RUN_DEFAULT);
  return 1;
}
