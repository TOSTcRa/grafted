// Minimal: calls _write and _exit through libSystem, no CRT
void _exit(int) __attribute__((noreturn));
long write(int, const void *, unsigned long);

// LC_MAIN enters here directly (not _start → main)
int main(void) {
    const char msg[] = "Hello from nostdlib!\n";
    write(1, msg, sizeof(msg) - 1);
    _exit(0);
}
