#include <unistd.h>
#include <string.h>

void _exit(int) __attribute__((noreturn));

int main(int argc, char **argv) {
    // Write argv[0]
    write(1, argv[0], strlen(argv[0]));
    write(1, ": ", 2);

    // Cat stdin to stdout
    char buf[256];
    ssize_t n;
    while ((n = read(0, buf, sizeof(buf))) > 0) {
        write(1, buf, n);
    }
    _exit(0);
}
