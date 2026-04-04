#include <stdio.h>
#include <unistd.h>

void _exit(int) __attribute__((noreturn));

int main(int argc, char **argv) {
    write(1, "write ok\n", 9);
    write(2, "stderr ok\n", 10);
    fprintf(stderr, "fprintf stderr ok\n");
    fprintf(stdout, "fprintf stdout ok\n");
    fflush(stdout);
    fflush(stderr);
    _exit(0);
}
