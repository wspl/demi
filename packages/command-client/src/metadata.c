#include "metadata.h"
#include "contract.h"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <uv.h>

typedef struct {
  char *data;
  size_t size, capacity;
} Text;
static void append(Text *text, const char *value, size_t n) {
  if (text->size + n + 1 > DEMI_MAX_FRAME) {
    fprintf(stderr, "demi: invocation metadata too large\n");
    exit(1);
  }
  if (text->size + n + 1 > text->capacity) {
    text->capacity = (text->size + n + 1) * 2;
    char *data = realloc(text->data, text->capacity);
    if (!data) {
      exit(1);
    }
    text->data = data;
  }
  memcpy(text->data + text->size, value, n);
  text->size += n;
  text->data[text->size] = 0;
}

static void str(Text *t, const char *s) {
  append(t, s, strlen(s));
}

static void quote(Text *t, const char *s) {
  str(t, "\"");
  for (const unsigned char *p = (const unsigned char *)s; *p; ++p) {
    if (*p == '"' || *p == '\\') {
      str(t, "\\");
      append(t, (const char *)p, 1);
    } else if (*p < 32) {
      char esc[7];
      snprintf(esc, sizeof(esc), "\\u%04x", *p);
      str(t, esc);
    } else {
      append(t, (const char *)p, 1);
    }
  }
  str(t, "\"");
}

static char *command_root(const char *executable) {
  const char *root = executable;
  for (const char *p = root; *p; ++p) {
    if (*p == '/' || *p == '\\') {
      root = p + 1;
    }
  }
  char *name = strdup(root);
  if (!name) {
    fprintf(stderr, "demi: out of memory\n");
    exit(1);
  }
#ifdef _WIN32
  size_t len = strlen(name);
  if (len > 4 && !strcmp(name + len - 4, ".exe")) {
    name[len - 4] = 0;
  }
#endif
  return name;
}

static int stdin_is_live(void) {
  const char *inherited_fd = getenv(DEMI_STDIN_FD_ENV);
  if (!inherited_fd || !*inherited_fd) {
    return 0;
  }
  char *end;
  long fd = strtol(inherited_fd, &end, 10);
  if (*end || fd < 3 || fd > 0x7fffffff) {
    return 0;
  }

  // Redirection replaces fd 0. Live stdin still refers to the job's
  // inherited input.
  uv_fs_t current_input;
  uv_fs_t job_input;
  int current_status = uv_fs_fstat(NULL, &current_input, 0, NULL);
  int job_status = uv_fs_fstat(NULL, &job_input, (uv_file)fd, NULL);
  int same_input = current_status == 0 && job_status == 0 &&
                   current_input.statbuf.st_dev == job_input.statbuf.st_dev &&
                   current_input.statbuf.st_ino == job_input.statbuf.st_ino;
  uv_fs_req_cleanup(&current_input);
  uv_fs_req_cleanup(&job_input);
  return same_input;
}

char *demi_metadata(int argc, char **argv, const char *id,
                    const char *context) {
  Text metadata = {0};
  char version[32];
  snprintf(version, sizeof(version), "{\"version\":%d,\"id\":", DEMI_VERSION);
  str(&metadata, version);
  quote(&metadata, id);
  str(&metadata, ",\"context\":");
  quote(&metadata, context);
  char *name = command_root(argv[0]);
  str(&metadata, ",\"root\":");
  quote(&metadata, name);
  free(name);
  str(&metadata, ",\"argv\":[");
  for (int i = 1; i < argc; ++i) {
    if (i > 1) {
      str(&metadata, ",");
    }
    quote(&metadata, argv[i]);
  }
  char *cwd = malloc(DEMI_MAX_FRAME);
  size_t size = DEMI_MAX_FRAME;
  if (!cwd || uv_cwd(cwd, &size)) {
    fprintf(stderr, "demi: cannot read cwd\n");
    exit(1);
  }
  str(&metadata, "],\"cwd\":");
  quote(&metadata, cwd);
  free(cwd);
  str(&metadata, ",\"env\":{");
  uv_env_item_t *items;
  int count;
  if (uv_os_environ(&items, &count)) {
    exit(1);
  }
  for (int i = 0; i < count; ++i) {
    if (i) {
      str(&metadata, ",");
    }
    quote(&metadata, items[i].name);
    str(&metadata, ":");
    quote(&metadata, items[i].value);
  }
  uv_os_free_environ(items, count);
  int live = stdin_is_live();
  str(&metadata, live ? "},\"live\":true}" : "},\"live\":false}");
  return metadata.data;
}
