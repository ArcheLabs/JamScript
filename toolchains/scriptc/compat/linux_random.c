/* JamScript's Linux glibc CSPRNG compatibility shim for ScriptC. */
#include "scr_runtime.h"

#if defined(__linux__) && !defined(SCR_MUSL)

#include <errno.h>
#include <sys/random.h>

/* Linux getrandom uses the same kernel CSPRNG source as arc4random_buf.
 * Retry EINTR and short reads; every other failure traps instead of returning
 * predictable or partially initialized bytes. */
void jamscript_secure_random(void *buf, size_t n) {
  unsigned char *p = buf;
  while (n > 0) {
    ssize_t got = getrandom(p, n, 0);
    if (got > 0) {
      p += (size_t)got;
      n -= (size_t)got;
      continue;
    }
    if (got < 0 && errno == EINTR) continue;
    scr_trap("scriptc: getrandom failed\n");
  }
}

#endif
