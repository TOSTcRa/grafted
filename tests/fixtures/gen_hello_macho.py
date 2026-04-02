#!/usr/bin/env python3
"""
Generate a minimal x86_64 Mach-O MH_EXECUTE binary that writes
"Hello from Grafted!\n" to stdout and exits with code 0.

Uses Darwin BSD syscall convention:
  - syscall number in rax (0x2000000 | unix_syscall_nr)
  - args in rdi, rsi, rdx, r10, r8, r9
  - syscall instruction

This creates a statically-linked binary with no dylib dependencies,
using LC_UNIXTHREAD to set the entry point.
"""

import struct
import sys

# --- x86_64 machine code ---
# The message is embedded right after the code.

# Code:
#   lea rsi, [rip + msg_offset]   ; pointer to message
#   mov rdi, 1                     ; stdout
#   mov rdx, 20                    ; length
#   mov rax, 0x2000004             ; write (BSD syscall 4)
#   syscall
#   xor rdi, rdi                   ; exit code 0
#   mov rax, 0x2000001             ; exit (BSD syscall 1)
#   syscall
# msg:
#   "Hello from Grafted!\n"

msg = b"Hello from Grafted!\n"
msg_len = len(msg)

# Assemble by hand
code = bytearray()

# lea rsi, [rip + offset]  — offset will be patched
# 48 8d 35 XX XX XX XX
lea_pos = len(code)
code += b'\x48\x8d\x35\x00\x00\x00\x00'  # placeholder offset

# mov rdi, 1
code += b'\x48\xc7\xc7\x01\x00\x00\x00'

# mov rdx, msg_len
code += b'\x48\xc7\xc2' + struct.pack('<I', msg_len)

# mov rax, 0x2000004 (write)
code += b'\x48\xc7\xc0\x04\x00\x00\x02'

# syscall
code += b'\x0f\x05'

# xor rdi, rdi
code += b'\x48\x31\xff'

# mov rax, 0x2000001 (exit)
code += b'\x48\xc7\xc0\x01\x00\x00\x02'

# syscall
code += b'\x0f\x05'

# Patch the lea rsi, [rip + offset]
# RIP at lea is lea_pos + 7 (size of lea instruction)
# msg starts at len(code)
msg_offset = len(code) - (lea_pos + 7)
struct.pack_into('<i', code, lea_pos + 3, msg_offset)

# Append message
code += msg

# --- Mach-O construction ---

# Constants
MH_MAGIC_64 = 0xfeedfacf
CPU_TYPE_X86_64 = 0x01000007
CPU_SUBTYPE_ALL = 0x03
MH_EXECUTE = 0x02
MH_NOUNDEFS = 0x01

# Load commands
LC_SEGMENT_64 = 0x19
LC_UNIXTHREAD = 0x05

# Thread state for x86_64
x86_THREAD_STATE64 = 4
x86_THREAD_STATE64_COUNT = 42  # number of uint32_t's in the state

# Layout:
#   Mach-O header (32 bytes)
#   LC_SEGMENT_64 __PAGEZERO (72 bytes)
#   LC_SEGMENT_64 __TEXT (72 bytes)
#   LC_UNIXTHREAD (32 + 42*4 = 200 bytes)
#   [padding to page boundary]
#   [code + data at page-aligned offset]

HEADER_SIZE = 32
SEG_CMD_SIZE = 72  # segment_command_64 with no sections
THREAD_STATE_SIZE = 42 * 4  # 168 bytes of register state
THREAD_CMD_SIZE = 16 + THREAD_STATE_SIZE  # cmd(4) + cmdsize(4) + flavor(4) + count(4) + state

ncmds = 3
sizeofcmds = SEG_CMD_SIZE + SEG_CMD_SIZE + THREAD_CMD_SIZE

# __TEXT segment starts at the next page (0x1000)
TEXT_VMADDR = 0x100000000  # standard macOS base address
TEXT_FILEOFF = 0x1000       # page-aligned file offset
TEXT_SIZE = (len(code) + 0xfff) & ~0xfff  # round up to page

# Entry point is at the start of __TEXT
ENTRY_ADDR = TEXT_VMADDR

# Build the header
header = struct.pack('<IIIIIIII',
    MH_MAGIC_64,
    CPU_TYPE_X86_64,
    CPU_SUBTYPE_ALL,
    MH_EXECUTE,
    ncmds,
    sizeofcmds,
    MH_NOUNDEFS,
    0,  # reserved
)

# __PAGEZERO segment (guard page, no file content)
pagezero = struct.pack('<II', LC_SEGMENT_64, SEG_CMD_SIZE)
pagezero += b'__PAGEZERO\x00\x00\x00\x00\x00\x00'  # 16-byte segname
pagezero += struct.pack('<QQQQIIII',
    0,                  # vmaddr
    TEXT_VMADDR,        # vmsize (everything below __TEXT)
    0,                  # fileoff
    0,                  # filesize
    0,                  # maxprot
    0,                  # initprot
    0,                  # nsects
    0,                  # flags
)

# __TEXT segment
text_seg = struct.pack('<II', LC_SEGMENT_64, SEG_CMD_SIZE)
text_seg += b'__TEXT\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00'  # 16-byte segname
text_seg += struct.pack('<QQQQIIII',
    TEXT_VMADDR,        # vmaddr
    TEXT_SIZE,          # vmsize
    TEXT_FILEOFF,       # fileoff
    len(code),          # filesize (actual code size)
    5,                  # maxprot (r-x)
    5,                  # initprot (r-x)
    0,                  # nsects
    0,                  # flags
)

# LC_UNIXTHREAD — set RIP to entry point
# x86_64 thread state: 42 uint32_t values
# Register order: rax,rbx,rcx,rdx,rdi,rsi,rbp,rsp,r8-r15,rip,rflags,cs,fs,gs
# We only care about rip (index 16, two uint32_t's for the 64-bit value)
regs = [0] * x86_THREAD_STATE64_COUNT
# rip is at index 32-33 (uint32 pairs) = register index 16
# Actually the layout is: each 64-bit reg takes 2 uint32_t slots
# rax(0-1), rbx(2-3), rcx(4-5), rdx(6-7), rdi(8-9), rsi(10-11),
# rbp(12-13), rsp(14-15), r8(16-17), r9(18-19), r10(20-21), r11(22-23),
# r12(24-25), r13(26-27), r14(28-29), r15(30-31), rip(32-33), rflags(34-35),
# cs(36-37), fs(38-39), gs(40-41)
rip_lo = ENTRY_ADDR & 0xFFFFFFFF
rip_hi = (ENTRY_ADDR >> 32) & 0xFFFFFFFF
regs[32] = rip_lo
regs[33] = rip_hi

# Set rsp to something reasonable
rsp = 0x7fff5fbff000
regs[14] = rsp & 0xFFFFFFFF
regs[15] = (rsp >> 32) & 0xFFFFFFFF

thread_cmd = struct.pack('<IIII',
    LC_UNIXTHREAD,
    THREAD_CMD_SIZE,
    x86_THREAD_STATE64,
    x86_THREAD_STATE64_COUNT,
)
for r in regs:
    thread_cmd += struct.pack('<I', r)

# Assemble the file
out = bytearray()
out += header
out += pagezero
out += text_seg
out += thread_cmd

# Pad to TEXT_FILEOFF
if len(out) > TEXT_FILEOFF:
    print(f"Error: headers too large ({len(out)} > {TEXT_FILEOFF})", file=sys.stderr)
    sys.exit(1)
out += b'\x00' * (TEXT_FILEOFF - len(out))

# Write code
out += code

# Pad to page boundary
remainder = len(out) % 0x1000
if remainder:
    out += b'\x00' * (0x1000 - remainder)

output_path = sys.argv[1] if len(sys.argv) > 1 else "tests/fixtures/hello_x86_64.macho"
with open(output_path, 'wb') as f:
    f.write(out)

print(f"Generated {output_path} ({len(out)} bytes)")
print(f"  Code size: {len(code)} bytes")
print(f"  Entry point: {ENTRY_ADDR:#x}")
print(f"  __TEXT: vmaddr={TEXT_VMADDR:#x} fileoff={TEXT_FILEOFF:#x}")
