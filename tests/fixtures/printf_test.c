#include <stdio.h>
#include <string.h>
#include <stdlib.h>

int main(int argc, char **argv) {
    printf("argc=%d\n", argc);
    for (int i = 0; i < argc; i++)
        printf("argv[%d]=%s\n", i, argv[i]);
    printf("strlen(\"grafted\")=%zu\n", strlen("grafted"));

    char *p = malloc(64);
    if (p) {
        sprintf(p, "malloc works!");
        printf("%s\n", p);
        free(p);
    }

    printf("done\n");
    fflush(stdout);
    return 0;
}
