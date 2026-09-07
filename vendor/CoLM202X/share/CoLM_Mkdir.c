#include <errno.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>

#ifdef _WIN32
#include <direct.h>
#include <io.h>
#include <windows.h>
#define PATH_SEP '\\'
#else
#include <dirent.h>
#include <unistd.h>
#define PATH_SEP '/'
#endif

/* Keep mode_t and platform-specific mkdir signatures on the C side of the ABI. */
int colm_mkdir_one(const char *path)
{
#ifdef _WIN32
    int status = _mkdir(path);
#else
    int status = mkdir(path, 0777);
#endif
    if (status == 0) return 0;
    int error = errno;
#ifdef _WIN32
    struct _stat info;
    if (_stat(path, &info) == 0 && (info.st_mode & _S_IFMT) == _S_IFDIR) return 0;
#else
    struct stat info;
    if (stat(path, &info) == 0 && S_ISDIR(info.st_mode)) return 0;
#endif
    return error;
}

static int is_separator(char c)
{
#ifdef _WIN32
    return c == '/' || c == '\\';
#else
    return c == '/';
#endif
}

static int starts_with(const char *text, const char *prefix)
{
    size_t n = strlen(prefix);
    return strncmp(text, prefix, n) == 0;
}

static int ends_with(const char *text, const char *suffix)
{
    size_t text_len = strlen(text);
    size_t suffix_len = strlen(suffix);
    return suffix_len <= text_len && strcmp(text + text_len - suffix_len, suffix) == 0;
}

static char *copy_string(const char *text)
{
    size_t n = strlen(text) + 1u;
    char *copy = (char *)malloc(n);
    if (copy) memcpy(copy, text, n);
    return copy;
}

static int compare_strings(const void *left, const void *right)
{
    const char *const *a = (const char *const *)left;
    const char *const *b = (const char *const *)right;
    return strcmp(*a, *b);
}

static void free_matches(char **matches, size_t count)
{
    if (!matches) return;
    for (size_t i = 0; i < count; ++i) free(matches[i]);
    free(matches);
}

static int append_match(char ***matches, size_t *count, size_t *capacity,
                        const char *directory, const char *name)
{
    if (*count == *capacity) {
        size_t next_capacity = *capacity == 0 ? 16 : *capacity * 2;
        char **next = (char **)realloc(*matches, next_capacity * sizeof(char *));
        if (!next) return ENOMEM;
        *matches = next;
        *capacity = next_capacity;
    }

    size_t dir_len = strlen(directory);
    size_t name_len = strlen(name);
    int needs_sep = dir_len > 0 && strcmp(directory, ".") != 0 && !is_separator(directory[dir_len - 1]);
    size_t total = dir_len + (needs_sep ? 1u : 0u) + name_len + 1u;
    char *path = (char *)malloc(total);
    if (!path) return ENOMEM;

    if (strcmp(directory, ".") == 0) {
        snprintf(path, total, "%s", name);
    } else if (needs_sep) {
        snprintf(path, total, "%s%c%s", directory, PATH_SEP, name);
    } else {
        snprintf(path, total, "%s%s", directory, name);
    }
    (*matches)[(*count)++] = path;
    return 0;
}

static int split_prefix(const char *prefix, char **directory, const char **name_prefix)
{
    const char *last = NULL;
    for (const char *p = prefix; *p; ++p) {
        if (is_separator(*p)) last = p;
    }

    if (!last) {
        *directory = copy_string(".");
        *name_prefix = prefix;
        return *directory ? 0 : ENOMEM;
    }

    size_t dir_len = (size_t)(last - prefix);
    if (dir_len == 0) dir_len = 1; /* root directory */
#ifdef _WIN32
    if (dir_len == 2 && prefix[1] == ':' && is_separator(prefix[2])) dir_len = 3;
#endif
    *directory = (char *)malloc(dir_len + 1u);
    if (!*directory) return ENOMEM;
    memcpy(*directory, prefix, dir_len);
    (*directory)[dir_len] = '\0';
    *name_prefix = last + 1;
    return 0;
}

static int write_matches(const char *listfile, char **matches, size_t count)
{
    FILE *out = fopen(listfile, "w");
    if (!out) return errno;
    for (size_t i = 0; i < count; ++i) {
        if (strchr(matches[i], '\n')) {
            fclose(out);
            return EINVAL;
        }
        if (fprintf(out, "%s\n", matches[i]) < 0) {
            int error = errno ? errno : EIO;
            fclose(out);
            return error;
        }
    }
    if (fclose(out) != 0) return errno;
    return 0;
}

int colm_list_matching_paths(const char *prefix, const char *suffix, const char *listfile)
{
    if (!prefix || !suffix || !listfile) return EINVAL;

    char *directory = NULL;
    const char *name_prefix = NULL;
    int status = split_prefix(prefix, &directory, &name_prefix);
    if (status != 0) return status;

    char **matches = NULL;
    size_t count = 0, capacity = 0;

#ifdef _WIN32
    size_t pattern_len = strlen(directory) + 3u;
    char *pattern = (char *)malloc(pattern_len);
    if (!pattern) {
        free(directory);
        return ENOMEM;
    }
    snprintf(pattern, pattern_len, "%s\\*", directory);

    WIN32_FIND_DATAA data;
    HANDLE handle = FindFirstFileA(pattern, &data);
    free(pattern);
    if (handle == INVALID_HANDLE_VALUE) {
        DWORD win_error = GetLastError();
        free(directory);
        return win_error == ERROR_FILE_NOT_FOUND ? write_matches(listfile, NULL, 0) : (int)win_error;
    }
    do {
        if (!(data.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY) &&
            starts_with(data.cFileName, name_prefix) && ends_with(data.cFileName, suffix)) {
            status = append_match(&matches, &count, &capacity, directory, data.cFileName);
            if (status != 0) break;
        }
    } while (FindNextFileA(handle, &data));
    DWORD find_error = GetLastError();
    FindClose(handle);
    if (status == 0 && find_error != ERROR_NO_MORE_FILES) status = (int)find_error;
#else
    DIR *dir = opendir(directory);
    if (!dir) {
        status = errno;
        free(directory);
        return status;
    }
    errno = 0;
    for (struct dirent *entry = readdir(dir); entry; entry = readdir(dir)) {
        if (starts_with(entry->d_name, name_prefix) && ends_with(entry->d_name, suffix)) {
            status = append_match(&matches, &count, &capacity, directory, entry->d_name);
            if (status != 0) break;
        }
    }
    if (status == 0 && errno != 0) status = errno;
    if (closedir(dir) != 0 && status == 0) status = errno;
#endif

    free(directory);
    if (status == 0) {
        if (count > 1) qsort(matches, count, sizeof(char *), compare_strings);
        status = write_matches(listfile, matches, count);
    }
    free_matches(matches, count);
    return status;
}


int colm_same_file(const char *left, const char *right)
{
    if (!left || !right) return -EINVAL;
#ifdef _WIN32
    HANDLE a = CreateFileA(left, 0, FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                           NULL, OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, NULL);
    if (a == INVALID_HANDLE_VALUE) return -(int)GetLastError();
    if (GetFileType(a) != FILE_TYPE_DISK) {
        CloseHandle(a);
        return -EINVAL;
    }
    HANDLE b = CreateFileA(right, 0, FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                           NULL, OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, NULL);
    if (b == INVALID_HANDLE_VALUE) {
        DWORD error = GetLastError();
        CloseHandle(a);
        return error == ERROR_FILE_NOT_FOUND || error == ERROR_PATH_NOT_FOUND ? 0 : -(int)error;
    }
    BY_HANDLE_FILE_INFORMATION ai, bi;
    int ok = GetFileInformationByHandle(a, &ai) && GetFileInformationByHandle(b, &bi);
    DWORD info_error = ok ? 0 : GetLastError();
    CloseHandle(a);
    CloseHandle(b);
    if (!ok) return -(int)info_error;
    return ai.dwVolumeSerialNumber == bi.dwVolumeSerialNumber &&
           ai.nFileIndexHigh == bi.nFileIndexHigh &&
           ai.nFileIndexLow == bi.nFileIndexLow;
#else
    struct stat a, b;
    if (stat(left, &a) != 0) return -errno;
    if (!S_ISREG(a.st_mode)) return -EINVAL;
    if (stat(right, &b) != 0) return errno == ENOENT ? 0 : -errno;
    if (!S_ISREG(b.st_mode)) return -EINVAL;
    return a.st_dev == b.st_dev && a.st_ino == b.st_ino;
#endif
}

int colm_rename_file(const char *src, const char *dst)
{
    if (!src || !dst) return EINVAL;
#ifdef _WIN32
    /* Do not delete dst first: a missing/unmovable source must leave it intact. */
    if (MoveFileExA(src, dst, MOVEFILE_REPLACE_EXISTING)) return 0;
    return (int)GetLastError();
#else
    if (rename(src, dst) == 0) return 0;
    return errno;
#endif
}
