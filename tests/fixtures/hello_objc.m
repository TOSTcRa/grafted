// Forward declarations for ObjC runtime functions
extern void *objc_msgSend(void *self, void *op, ...);
extern void *objc_getClass(const char *name);
extern void *sel_registerName(const char *str);

@interface TinyObject
@end

@implementation TinyObject
@end

@interface Greeter : TinyObject
@end

@implementation Greeter
- (void)sayHello {
    unsigned long rax = 0x2000004; // write
    unsigned long rdi = 1;
    const char *msg = "Hello from full ObjC runtime allocation!\n";
    unsigned long rdx = 41;
    __asm__ volatile ("syscall" : : "a"(rax), "D"(rdi), "S"(msg), "d"(rdx));
}
@end

void _start() {
    void *cls = objc_getClass("Greeter");
    void *alloc_sel = sel_registerName("alloc");
    void *init_sel = sel_registerName("init");
    void *say_sel = sel_registerName("sayHello");

    // [Greeter alloc]
    void *obj = objc_msgSend(cls, alloc_sel);
    // [obj init]
    obj = objc_msgSend(obj, init_sel);
    // [obj sayHello]
    objc_msgSend(obj, say_sel);

    unsigned long rax = 0x2000001; // exit
    unsigned long rdi = 0;
    __asm__ volatile ("syscall" : : "a"(rax), "D"(rdi));
}
