#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <uv.h>
#include "metadata.h"
#include "contract.h"

typedef struct { char *data; size_t size, capacity; } Text;
static void append(Text *text, const char *value, size_t n) {
    if (text->size + n + 1 > DEMI_MAX_FRAME) { fprintf(stderr, "demi: invocation metadata too large\n"); exit(1); }
    if (text->size + n + 1 > text->capacity) {
        text->capacity = (text->size + n + 1) * 2;
        char *data = realloc(text->data, text->capacity);
        if (!data) exit(1);
        text->data = data;
    }
    memcpy(text->data + text->size, value, n); text->size += n; text->data[text->size] = 0;
}
static void str(Text *t, const char *s) { append(t, s, strlen(s)); }
static void quote(Text *t, const char *s) {
    str(t, "\"");
    for (const unsigned char *p = (const unsigned char *)s; *p; ++p) {
        if (*p == '"' || *p == '\\') { str(t, "\\"); append(t, (const char *)p, 1); }
        else if (*p < 32) { char esc[7]; snprintf(esc, sizeof(esc), "\\u%04x", *p); str(t, esc); }
        else append(t, (const char *)p, 1);
    }
    str(t, "\"");
}
char *demi_metadata(int argc, char **argv, const char *id, const char *context) {
    Text t = {0}; char version[32]; snprintf(version, sizeof(version), "{\"version\":%d,\"id\":", DEMI_VERSION);
    str(&t, version); quote(&t, id); str(&t, ",\"context\":"); quote(&t, context);
    const char *root = argv[0];
    for (const char *p = root; *p; ++p) if (*p == '/' || *p == '\\') root = p + 1;
    char *name = strdup(root);
    if (!name) { fprintf(stderr, "demi: out of memory\n"); exit(1); }
    size_t len = strlen(name);
#ifdef _WIN32
    if (len > 4 && !strcmp(name + len - 4, ".exe")) name[len - 4] = 0;
#else
    (void)len;
#endif
    str(&t, ",\"root\":"); quote(&t, name); free(name); str(&t, ",\"argv\":[");
    for (int i = 1; i < argc; ++i) { if (i > 1) str(&t, ","); quote(&t, argv[i]); }
    char *cwd = malloc(DEMI_MAX_FRAME); size_t size = DEMI_MAX_FRAME;
    if (!cwd || uv_cwd(cwd, &size)) { fprintf(stderr, "demi: cannot read cwd\n"); exit(1); }
    str(&t, "],\"cwd\":"); quote(&t, cwd); free(cwd); str(&t, ",\"env\":{");
    uv_env_item_t *items; int count;
    if (uv_os_environ(&items, &count)) exit(1);
    for (int i = 0; i < count; ++i) { if (i) str(&t, ","); quote(&t, items[i].name); str(&t, ":"); quote(&t, items[i].value); }
    uv_os_free_environ(items, count);
    int live = 0; const char *fd = getenv(DEMI_STDIN_FD_ENV); char *end = NULL;
    long inherited = fd ? strtol(fd, &end, 10) : -1;
    if (fd && *fd && end && !*end && inherited >= 3 && inherited <= 0x7fffffff) {
        uv_fs_t a, b;
        int ar = uv_fs_fstat(NULL, &a, 0, NULL), br = uv_fs_fstat(NULL, &b, (uv_file)inherited, NULL);
        live = !ar && !br && a.statbuf.st_dev == b.statbuf.st_dev && a.statbuf.st_ino == b.statbuf.st_ino;
        uv_fs_req_cleanup(&a); uv_fs_req_cleanup(&b);
    }
    str(&t, live ? "},\"live\":true}" : "},\"live\":false}");
    return t.data;
}
