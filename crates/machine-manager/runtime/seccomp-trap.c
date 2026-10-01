#define _GNU_SOURCE
#include <linux/filter.h>
#include <linux/seccomp.h>
#include <stddef.h>
#include <stdio.h>
#include <signal.h>
#include <sys/prctl.h>
#include <sys/syscall.h>
#include <ucontext.h>
#include <unistd.h>
static volatile unsigned long observed;
static void trap(int sig, siginfo_t *info, void *context) {
  ucontext_t *state = context;
  observed = state->uc_mcontext.regs[0];
  state->uc_mcontext.regs[0] = 0;
}
int main(void) {
  struct sigaction action = {.sa_sigaction = trap, .sa_flags = SA_SIGINFO};
  if (sigaction(SIGSYS, &action, 0) != 0)
    return 2;
  struct sock_filter code[] = {
    BPF_STMT(BPF_LD | BPF_W | BPF_ABS, offsetof(struct seccomp_data, nr)),
    BPF_JUMP(BPF_JMP | BPF_JEQ | BPF_K, SYS_getpid, 0, 1),
    BPF_STMT(BPF_RET | BPF_K, SECCOMP_RET_TRAP),
    BPF_STMT(BPF_RET | BPF_K, SECCOMP_RET_ALLOW)
  };
  struct sock_fprog program = {.len = 4, .filter = code};
  if (prctl(PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) ||
      prctl(PR_SET_SECCOMP, SECCOMP_MODE_FILTER, &program))
    return 2;
  syscall(SYS_getpid, 0x123456);
  printf("observed x0=0x%lx expected=0x123456 syscall=%d\n", observed, SYS_getpid);
  return observed == 0x123456 ? 0 : 1;
}
