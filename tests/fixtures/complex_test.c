#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

int main(int argc, char **argv) {
    write(1, "start\n", 6);

    int n = 10;
    if (argc > 1) n = atoi(argv[1]);

    int a = 0, b = 1;
    for (int i = 2; i <= n; i++) { int t = a + b; a = b; b = t; }

    char buf[64];
    int len = snprintf(buf, sizeof(buf), "fib(%d) = %d\n", n, b);
    write(1, buf, len);

    char *p = malloc(128);
    if (p) {
        len = snprintf(buf, sizeof(buf), "malloc=%p\n", (void*)p);
        write(1, buf, len);
        free(p);
        write(1, "free ok\n", 8);
    }

    write(1, "done\n", 5);
    return 0;
}
