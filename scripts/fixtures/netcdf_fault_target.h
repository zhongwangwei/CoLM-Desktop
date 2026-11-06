#ifndef COLM_NETCDF_FAULT_TARGET_H
#define COLM_NETCDF_FAULT_TARGET_H
#include <stdlib.h>
#include <string.h>

/* Exact matching remains the default for runtime probes. Unique observation
 * staging names are admitted only inside the harness's private directory. */
static int colm_fault_target_matches(const char *path, const char *target) {
    const char *mode = getenv("COLM_TEST_IO_TARGET_MODE");
    if (!mode) return !strcmp(path, target);
    if (strcmp(mode, "observation-temp")) return 0;
    const char *root = getenv("COLM_TEST_IO_ROOT");
    if (!root || root[0] != '/' || target[0] != '/') return 0;
    size_t root_length = strlen(root);
    if (strncmp(target, root, root_length) || target[root_length] != '/') return 0;
    const char *name = strrchr(target, '/');
    if (!name || !name[1]) return 0;
    size_t directory_length = (size_t)(name - target) + 1;
    if (strncmp(path, target, directory_length)) return 0;
    const char *suffix = path + directory_length;
    size_t name_length = strlen(++name);
    if (*suffix++ != '.' || strncmp(suffix, name, name_length)) return 0;
    suffix += name_length;
    if (*suffix++ != '.') return 0;
    for (int group = 0; group < 3; group++) {
        const char *start = suffix;
        while (*suffix >= '0' && *suffix <= '9') suffix++;
        if (suffix == start) return 0;
        if (group < 2 && *suffix++ != '-') return 0;
    }
    return !strcmp(suffix, ".tmp");
}
#endif
