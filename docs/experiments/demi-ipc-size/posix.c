/* Transport-size probe only: stdin -> UDS -> stdout, not the Demi protocol. */
#include <errno.h>
#include <pthread.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/socket.h>
#include <sys/un.h>
#include <unistd.h>

static int peer;
static void fail(const char *what) { perror(what); exit(1); }
static void write_all(int fd, const char *data, size_t length) {
    while (length) {
        ssize_t n = write(fd, data, length);
        if (n < 0 && errno == EINTR) continue;
        if (n <= 0) fail("write");
        data += n; length -= (size_t)n;
    }
}
static void *send_input(void *unused) {
    (void)unused;
    char buffer[65536];
    for (;;) {
        ssize_t n = read(0, buffer, sizeof(buffer));
        if (n < 0 && errno == EINTR) continue;
        if (n < 0) fail("stdin");
        if (!n) break;
        write_all(peer, buffer, (size_t)n);
    }
    if (shutdown(peer, SHUT_WR)) fail("shutdown");
    return NULL;
}
int main(int argc, char **argv) {
    if (argc != 2) { fprintf(stderr, "Usage: ipc-client socket-path\n"); return 2; }
    struct sockaddr_un address = { .sun_family = AF_UNIX };
    if (strlen(argv[1]) >= sizeof(address.sun_path)) return 2;
    memcpy(address.sun_path, argv[1], strlen(argv[1]) + 1);
    signal(SIGPIPE, SIG_IGN);
    peer = socket(AF_UNIX, SOCK_STREAM, 0);
    if (peer < 0 || connect(peer, (struct sockaddr *)&address, sizeof(address))) fail("connect");
    pthread_t input;
    if (pthread_create(&input, NULL, send_input, NULL)) return 1;
    char buffer[65536];
    for (;;) {
        ssize_t n = read(peer, buffer, sizeof(buffer));
        if (n < 0 && errno == EINTR) continue;
        if (n < 0) fail("receive");
        if (!n) break;
        write_all(1, buffer, (size_t)n);
    }
    /* Process exit also ends a sender blocked on interactive stdin. */
    close(peer);
    return 0;
}
