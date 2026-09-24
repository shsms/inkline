/* Runs a readline command for inkline, catching a jump back to readline's
   top level (readline's abort: C-g, yank with an empty kill ring) or to
   bash's (shell code the command ran failed, as `${x:?}` does), which
   would otherwise skip Rust and Lisp frames. */
#include <setjmp.h>
#include <string.h>

extern sigjmp_buf _rl_top_level;
/* bash's `procenv_t top_level`, a sigjmp_buf where the system has POSIX
   sigsetjmp. */
extern sigjmp_buf top_level;

/* Calls f(count, key) and returns what it returns. *jumped is 0 when it
   returned, -1 when it jumped to readline's top level, and the value bash
   jumped to its own top level with (always above 0) when it jumped there;
   the function then returns -1. */
int inkline_call_command(int (*f)(int, int), int count, int key, int *jumped)
{
    sigjmp_buf saved_rl, saved_sh;
    int code, result;

    memcpy(saved_rl, _rl_top_level, sizeof saved_rl);
    memcpy(saved_sh, top_level, sizeof saved_sh);
    if (sigsetjmp(_rl_top_level, 0)) {
        code = -1;
        result = -1;
    } else if ((code = sigsetjmp(top_level, 0)) != 0) {
        result = -1;
    } else {
        result = f(count, key);
    }
    memcpy(_rl_top_level, saved_rl, sizeof saved_rl);
    memcpy(top_level, saved_sh, sizeof saved_sh);
    *jumped = code;
    return result;
}
