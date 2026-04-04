#!/usr/bin/env python3
import struct
import sys

# Mach trap 26 is mach_reply_port.
# Darwin Mach trap convention: rax = 0x1000000 | trap_nr
# Result is in rax.

code = bytearray()

# mov rax, 0x100001a (mach_reply_port)
code += b'\x48\xc7\xc0\x1a\x00\x00\x01'
# syscall
code += b'\x0f\x05'

# mov rdi, rax (pass result to exit)
code += b'\x48\x89\xc7'
# mov rax, 0x2000001 (exit)
code += b'\x48\xc7\xc0\x01\x00\x00\x02'
# syscall
code += b'\x0f\x05'

# Mach-O boilerplate
MH_MAGIC_64 = 0xfeedfacf
CPU_TYPE_X86_64 = 0x01000007
MH_EXECUTE = 0x02
LC_SEGMENT_64 = 0x19
LC_UNIXTHREAD = 0x05

TEXT_VMADDR = 0x100000000
TEXT_FILEOFF = 0x1000
ENTRY_ADDR = TEXT_VMADDR

header = struct.pack('<IIIIIIII', MH_MAGIC_64, CPU_TYPE_X86_64, 0x03, MH_EXECUTE, 2, 72+200, 1, 0)
text_seg = struct.pack('<II', LC_SEGMENT_64, 72)
text_seg += b'__TEXT\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00'
text_seg += struct.pack('<QQQQIIII', TEXT_VMADDR, 0x1000, TEXT_FILEOFF, len(code), 7, 7, 0, 0)

regs = [0] * 42
regs[32] = ENTRY_ADDR & 0xFFFFFFFF
regs[33] = (ENTRY_ADDR >> 32) & 0xFFFFFFFF
regs[14] = 0x7fff5fbff000 & 0xFFFFFFFF
regs[15] = (0x7fff5fbff000 >> 32) & 0xFFFFFFFF

thread_cmd = struct.pack('<IIII', LC_UNIXTHREAD, 200, 4, 42)
for r in regs: thread_cmd += struct.pack('<I', r)

out = header + text_seg + thread_cmd
out += b'\x00' * (TEXT_FILEOFF - len(out))
out += code
out += b'\x00' * (0x1000 - (len(out) % 0x1000))

with open("tests/fixtures/mach_trap_test.macho", 'wb') as f: f.write(out)
