#include <unistd.h>

void _exit(int) __attribute__((noreturn));

int main(int argc, char **argv) {
    const char msg[] = "Hello from real C!\n";
    write(1, msg, sizeof(msg) - 1);
    _exit(0);
}
