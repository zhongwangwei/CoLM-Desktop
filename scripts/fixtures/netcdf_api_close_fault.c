#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <limits.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <unistd.h>

extern int nc_inq_path(int, size_t *, char *);
extern int __real_nc_close(int);

/* Link only into fault probes; nc_close still performs the actual file I/O. */
int __wrap_nc_close(int ncid) {
    const char *target = getenv("COLM_TEST_IO_TARGET");
    const char *trace = getenv("COLM_TEST_IO_TRACE");
    const char *error = getenv("COLM_TEST_IO_ERRNO");
    size_t length = 0;
    char path[PATH_MAX + 1];
    int inject = target && trace && error && nc_inq_path(ncid, &length, NULL) == 0
        && length < sizeof(path) && nc_inq_path(ncid, NULL, path) == 0
        && !strcmp(path, target);
    /* Raw-libc mode only marks the boundary; never changes nc_close's result. */
    const char *phase = getenv("COLM_TEST_IO_CLOSE_PHASE");
    if (inject && phase) {
        int log = open(trace, O_WRONLY | O_CREAT | O_APPEND, 0600);
        if (log >= 0) {
            const char line[] = "nc_close enter\n";
            (void)write(log, line, sizeof(line) - 1);
            (void)close(log);
        }
        int marker = open(phase, O_WRONLY | O_CREAT | O_TRUNC, 0600);
        if (marker >= 0) (void)close(marker);
    }
    int result = __real_nc_close(ncid);
    if (inject && phase) {
        (void)unlink(phase);
        char line[96];
        int size = snprintf(line, sizeof(line), "nc_close exit result=%d\n", result);
        int log = open(trace, O_WRONLY | O_CREAT | O_APPEND, 0600);
        if (log >= 0) {
            (void)write(log, line, size);
            (void)close(log);
        }
        return result;
    }
    if (inject && result == 0) {
        struct stat status;
        int number = !strcmp(error, "ENOSPC") ? ENOSPC : !strcmp(error, "EIO") ? EIO : 0;
        if (number && stat(path, &status) == 0 && status.st_size >= 4096) {
            char line[192];
            int size = snprintf(line, sizeof(line), "nc_close errno=%d bytes=%lld real_close=0\n",
                                number, (long long)status.st_size);
            int log = open(trace, O_WRONLY | O_CREAT | O_APPEND, 0600);
            if (log >= 0) {
                (void)write(log, line, size);
                (void)close(log);
            }
            errno = number;
            return number;
        }
    }
    return result;
}
