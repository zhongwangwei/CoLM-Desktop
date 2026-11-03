#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <limits.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/syscall.h>
#include <unistd.h>

/* Test-only delayed I/O failure, limited to the exact private fixture pathname. */
int close(int fd) {
    static _Thread_local int inside;
    if (inside) return (int)syscall(SYS_close, fd);
    inside = 1;
    const char *target = getenv("COLM_TEST_IO_TARGET");
    const char *trace = getenv("COLM_TEST_IO_TRACE");
    const char *error = getenv("COLM_TEST_IO_ERRNO");
    char link[64], path[PATH_MAX + 1];
    struct stat status;
    snprintf(link, sizeof(link), "/proc/self/fd/%d", fd);
    ssize_t length = readlink(link, path, sizeof(path) - 1);
    if (length >= 0) path[length] = '\0';
    int inject = target && trace && error && length >= 0 && !strcmp(path, target)
        && fstat(fd, &status) == 0 && S_ISREG(status.st_mode) && status.st_size >= 4096;
    int result = (int)syscall(SYS_close, fd);
    int saved_errno = errno;
    if (inject && result == 0) {
        int number = !strcmp(error, "ENOSPC") ? ENOSPC : !strcmp(error, "EIO") ? EIO : 0;
        if (number) {
            char line[192];
            int size = snprintf(line, sizeof(line), "close errno=%d bytes=%lld\n", number, (long long)status.st_size);
            int log = (int)syscall(SYS_openat, AT_FDCWD, trace, O_WRONLY | O_CREAT | O_APPEND, 0600);
            if (log >= 0) {
                (void)syscall(SYS_write, log, line, size);
                (void)syscall(SYS_close, log);
            }
            inside = 0;
            errno = number;
            return -1;
        }
    }
    inside = 0;
    errno = saved_errno;
    return result;
}
