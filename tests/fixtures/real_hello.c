void _start() {
    const char *msg = "Real Darwin binary running on Linux via Grafted!\n";
    // Darwin write syscall: rax=0x2000004, rdi=1, rsi=msg, rdx=50
    long ret;
    __asm__ volatile (
        "syscall"
        : "=a"(ret)
        : "0"(0x2000004), "D"(1), "S"(msg), "d"(50)
        : "rcx", "r11", "memory"
    );
    // Darwin exit syscall: rax=0x2000001, rdi=0
    __asm__ volatile (
        "syscall"
        :
        : "a"(0x2000001), "D"(0)
        : "rcx", "r11", "memory"
    );
}
