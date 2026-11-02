/* SPDX-License-Identifier: MIT */
/*
 * A sample C application running on the kernel. After app_main() returns, the
 * kernel flushes stdout and any open files, then powers off.
 */

#include <errno.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "kernel.h"

void app_main(void) {
    puts("hello from C");

    FILE *f = fopen("/hello.txt", "r");
    char text[64];
    const size_t length = f != NULL ? fread(text, 1, sizeof(text) - 1, f) : 0;
    text[length] = 0;
    if (f != NULL) {
        fclose(f);
    }
    printf("/hello.txt: %s", text);

    f = fopen("/data/note.txt", "w");
    if (f != NULL) {
        fprintf(f, "line %d\nline %d\n", 1, 2);
        fclose(f);
    }
    f = fopen("/data/note.txt", "r");
    if (f != NULL) {
        char line[16];
        if (fgets(line, sizeof(line), f) != NULL) {
            printf("first: %s", line);
        }
        fseek(f, -7, SEEK_END);
        if (fgets(line, sizeof(line), f) != NULL) {
            printf("last: %s", line);
        }
        fclose(f);
    }
    if (fopen("/missing.txt", "r") == NULL) {
        fprintf(stderr, "/missing.txt: %s\n", strerror(errno));
    }

    f = tmpfile(); /* pure in-memory, not visible on vfs */
    if (f != NULL) {
        fputs("in memory", f);
        rewind(f);
        if (fgets(text, sizeof(text), f) != NULL) {
            printf("tmpfile: %s\n", text);
        }
        fclose(f);
    }
    char name[] = "/tmp/helloXXXXXX";
    const int fd = mkstemp(name);
    if (fd >= 0) {
        printf("mkstemp: %s\n", name);
        fs_close(fd);
        remove(name);
    }

    /* stdout is line buffered. */
    fwrite("stdout\nbuffered ", 1, 16, stdout);
    for (int i = 0; i < 2000; i++) {
        putchar('.');
    }
    putchar('\n');

    struct fs_listing listing = {0};
    const int result = fs_list("/", &listing);
    printf("ls /:\n");
    if (result < 0) {
        printf("  %s\n", strerror(-result));
    }
    for (size_t i = 0; i < listing.count; i++) {
        printf("  %s%s\n", listing.entries[i].name, listing.entries[i].kind == FS_KIND_DIRECTORY ? "/" : "");
    }
    free(listing.entries);
    free(listing.names);

    printf("type a line: ");
    fflush(stdout);
    char line[80];
    size_t count = 0;
    for (int c; count < sizeof(line) - 1 && (c = getchar()) != EOF && c != '\r' && c != '\n';) {
        line[count++] = (char) c;
    }
    line[count] = 0;
    printf("\nthe line: %s\n", line);
}
