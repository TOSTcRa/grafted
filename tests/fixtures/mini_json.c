// Mini JSON key extractor — reads stdin, finds "key": "value" pattern
#include <stdio.h>
#include <string.h>
#include <stdlib.h>

int main(int argc, char **argv) {
    if (argc < 2) {
        fprintf(stderr, "usage: %s <key>\n", argv[0]);
        return 1;
    }

    char input[4096];
    size_t total = 0;
    size_t n;
    while ((n = fread(input + total, 1, sizeof(input) - total - 1, stdin)) > 0)
        total += n;
    input[total] = '\0';

    // Find "key"
    char pattern[256];
    snprintf(pattern, sizeof(pattern), "\"%s\"", argv[1]);

    char *pos = strstr(input, pattern);
    if (!pos) {
        fprintf(stderr, "key not found: %s\n", argv[1]);
        return 1;
    }

    // Skip past "key":
    pos += strlen(pattern);
    while (*pos == ' ' || *pos == ':') pos++;

    // Extract value
    if (*pos == '"') {
        pos++;
        char *end = strchr(pos, '"');
        if (end) {
            *end = '\0';
            printf("\"%s\"\n", pos);
        }
    } else {
        // Number or literal
        char *end = pos;
        while (*end && *end != ',' && *end != '}' && *end != ' ') end++;
        *end = '\0';
        printf("%s\n", pos);
    }

    fflush(stdout);
    return 0;
}
