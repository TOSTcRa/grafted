#!/usr/bin/env python3
"""
Generate a dynamically-linked Mach-O that echoes its argv.

Reads argc/argv from the stack (LC_MAIN convention), then for each arg:
  _write(1, argv[i], strlen(argv[i]))
  _write(1, " ", 1)  -- space separator
Finishes with _write(1, "\\n", 1) then _exit(0).

This tests that the executor correctly builds the Darwin process stack.
"""

import struct
import sys

MH_MAGIC_64     = 0xfeedfacf
CPU_TYPE_X86_64 = 0x01000007
CPU_SUBTYPE_ALL = 0x03
MH_EXECUTE      = 0x02

LC_SEGMENT_64       = 0x19
LC_LOAD_DYLIB       = 0x0c
LC_DYLD_INFO_ONLY   = 0x80000022
LC_MAIN             = 0x80000028

BIND_TYPE_POINTER = 1

TEXT_VMADDR = 0x100000000
TEXT_FILEOFF = 0x1000
TEXT_VMSIZE = 0x1000

DATA_VMADDR = TEXT_VMADDR + TEXT_VMSIZE
DATA_FILEOFF = 0x2000
DATA_VMSIZE = 0x1000

BIND_FILEOFF = 0x3000

GOT_WRITE = DATA_VMADDR + 0
GOT_EXIT  = DATA_VMADDR + 8

# --- Machine code ---
# On entry (LC_MAIN): rsp → argc, then argv[0..argc], then NULL, envp...
#
# Algorithm:
#   push rbx; push r12; push r13; push r14  (save callee-saved)
#   mov r12, [rsp+32]      ; argc (offset 32 = 4 pushes * 8)
#   lea r13, [rsp+40]      ; argv[0] ptr (offset 40)
#   xor r14d, r14d         ; i = 0
# .loop:
#   cmp r14, r12
#   jge .done
#   mov rdi, [r13 + r14*8] ; argv[i]
#   ; strlen inline: scan for null
#   mov rsi, rdi            ; save start
#   xor ecx, ecx
# .strlen:
#   cmp byte [rdi + rcx], 0
#   je .strlen_done
#   inc ecx
#   jmp .strlen
# .strlen_done:
#   ; _write(1, argv[i], len)
#   mov edi, 1
#   mov rdx, rcx            ; count = strlen
#   ; rsi already has the string pointer
#   call [rip + got_write]
#   ; _write(1, " ", 1)
#   lea rsi, [rip + space]
#   mov edi, 1
#   mov edx, 1
#   call [rip + got_write]
#   inc r14
#   jmp .loop
# .done:
#   ; _write(1, "\n", 1)
#   lea rsi, [rip + newline]
#   mov edi, 1
#   mov edx, 1
#   call [rip + got_write]
#   ; _exit(0)
#   xor edi, edi
#   call [rip + got_exit]
#   int3

code = bytearray()

# Save callee-saved registers
code += b'\x53'                     # push rbx
code += b'\x41\x54'                 # push r12
code += b'\x41\x55'                 # push r13
code += b'\x41\x56'                 # push r14

# r12 = argc (at rsp+32 after 4 pushes)
code += b'\x4c\x8b\x64\x24\x20'    # mov r12, [rsp+0x20]

# r13 = &argv[0] (at rsp+40)
code += b'\x4c\x8d\x6c\x24\x28'    # lea r13, [rsp+0x28]

# r14 = 0 (loop counter)
code += b'\x45\x31\xf6'             # xor r14d, r14d

# .loop:
loop_offset = len(code)

# cmp r14, r12
code += b'\x4d\x39\xe6'             # cmp r14, r12

# jge .done (placeholder, patch later)
jge_pos = len(code)
code += b'\x0f\x8d\x00\x00\x00\x00'  # jge rel32

# mov rdi, [r13 + r14*8]  -- argv[i]
code += b'\x4b\x8b\x7c\xf5\x00'    # mov rdi, [r13 + r14*8]

# inline strlen: rsi = start, ecx = 0
code += b'\x48\x89\xfe'             # mov rsi, rdi
code += b'\x31\xc9'                 # xor ecx, ecx

# .strlen:
strlen_offset = len(code)
# cmp byte [rdi + rcx], 0
code += b'\x80\x3c\x0f\x00'        # cmp byte [rdi + rcx], 0

# je .strlen_done (short jump, placeholder)
je_pos = len(code)
code += b'\x74\x00'                  # je rel8

# inc ecx
code += b'\xff\xc1'                 # inc ecx

# jmp .strlen (short jump back)
code += b'\xeb'
code += bytes([(strlen_offset - (len(code) + 1)) & 0xff])

# .strlen_done:
strlen_done_offset = len(code)
code[je_pos + 1] = (strlen_done_offset - (je_pos + 2)) & 0xff

# _write(1, argv[i], len): rsi already has string, ecx has length
# mov rdx, rcx (count)
code += b'\x48\x89\xca'             # mov rdx, rcx
# mov edi, 1 (stdout)
code += b'\xbf\x01\x00\x00\x00'    # mov edi, 1

# call [rip + got_write_offset]
call_write1_pos = len(code)
code += b'\xff\x15\x00\x00\x00\x00'  # call [rip + disp32]

# _write(1, " ", 1)
# lea rsi, [rip + space_offset]
lea_space_pos = len(code)
code += b'\x48\x8d\x35\x00\x00\x00\x00'  # lea rsi, [rip + disp32]
# mov edi, 1
code += b'\xbf\x01\x00\x00\x00'
# mov edx, 1
code += b'\xba\x01\x00\x00\x00'

# call [rip + got_write_offset]
call_write2_pos = len(code)
code += b'\xff\x15\x00\x00\x00\x00'

# inc r14
code += b'\x49\xff\xc6'             # inc r14

# jmp .loop
code += b'\xe9'
rel = loop_offset - (len(code) + 4)
code += struct.pack('<i', rel)

# .done:
done_offset = len(code)
# Patch jge
struct.pack_into('<i', code, jge_pos + 2, done_offset - (jge_pos + 6))

# _write(1, "\n", 1)
lea_newline_pos = len(code)
code += b'\x48\x8d\x35\x00\x00\x00\x00'  # lea rsi, [rip + disp32]
code += b'\xbf\x01\x00\x00\x00'           # mov edi, 1
code += b'\xba\x01\x00\x00\x00'           # mov edx, 1

call_write3_pos = len(code)
code += b'\xff\x15\x00\x00\x00\x00'       # call [rip + got_write]

# _exit(0)
code += b'\x31\xff'                        # xor edi, edi
call_exit_pos = len(code)
code += b'\xff\x15\x00\x00\x00\x00'       # call [rip + got_exit]

code += b'\xcc'                            # int3

# Data embedded in __TEXT after code
space_offset = len(code)
code += b' '
newline_offset = len(code)
code += b'\n'

# --- Patch RIP-relative offsets ---
def patch_rip(code, insn_pos, insn_len, target_vmaddr):
    rip = TEXT_VMADDR + insn_pos + insn_len
    rel = target_vmaddr - rip
    struct.pack_into('<i', code, insn_pos + (insn_len - 4), rel)

# call [rip + got_write] (6 byte insn)
patch_rip(code, call_write1_pos, 6, GOT_WRITE)
patch_rip(code, call_write2_pos, 6, GOT_WRITE)
patch_rip(code, call_write3_pos, 6, GOT_WRITE)
patch_rip(code, call_exit_pos, 6, GOT_EXIT)

# lea rsi, [rip + space/newline] (7 byte insn)
patch_rip(code, lea_space_pos, 7, TEXT_VMADDR + space_offset)
patch_rip(code, lea_newline_pos, 7, TEXT_VMADDR + newline_offset)

# --- Bind opcodes ---
bind_data = bytearray()
bind_data.append(0x11)  # SET_DYLIB_ORDINAL_IMM(1)
bind_data.append(0x50 | BIND_TYPE_POINTER)
bind_data.append(0x40)  # SET_SYMBOL_TRAILING_FLAGS_IMM(0)
bind_data += b'_write\x00'
bind_data.append(0x72)  # SET_SEGMENT_AND_OFFSET_ULEB(seg=2)
bind_data.append(0x00)  # offset=0
bind_data.append(0x90)  # DO_BIND
bind_data.append(0x40)
bind_data += b'_exit\x00'
bind_data.append(0x90)  # DO_BIND (advances by 8)
bind_data.append(0x00)  # DONE
bind_size = len(bind_data)

# --- Assemble Mach-O ---
HEADER_SIZE = 32
SEG_CMD_SIZE = 72

dylib_path = b"/usr/lib/libSystem.B.dylib\x00"
DYLIB_CMD_FIXED = 24
dylib_cmd_size = (DYLIB_CMD_FIXED + len(dylib_path) + 7) & ~7

ncmds = 6
sizeofcmds = SEG_CMD_SIZE * 3 + dylib_cmd_size + 48 + 24

def seg_cmd(name, vmaddr, vmsize, fileoff, filesize, maxprot, initprot):
    n = name.encode().ljust(16, b'\x00')[:16]
    return struct.pack('<II', LC_SEGMENT_64, SEG_CMD_SIZE) + n + struct.pack('<QQQQIIII',
        vmaddr, vmsize, fileoff, filesize, maxprot, initprot, 0, 0)

header = struct.pack('<IIIIIIII',
    MH_MAGIC_64, CPU_TYPE_X86_64, CPU_SUBTYPE_ALL, MH_EXECUTE,
    ncmds, sizeofcmds, 0x200005, 0)

out = bytearray()
out += header
out += seg_cmd("__PAGEZERO", 0, TEXT_VMADDR, 0, 0, 0, 0)
out += seg_cmd("__TEXT", TEXT_VMADDR, TEXT_VMSIZE, TEXT_FILEOFF, len(code), 5, 5)
out += seg_cmd("__DATA", DATA_VMADDR, DATA_VMSIZE, DATA_FILEOFF, 16, 3, 3)

# LC_LOAD_DYLIB
dylib_cmd = struct.pack('<IIIIII', LC_LOAD_DYLIB, dylib_cmd_size, DYLIB_CMD_FIXED, 0, 0x10000, 0x10000)
dylib_cmd += dylib_path
dylib_cmd = dylib_cmd.ljust(dylib_cmd_size, b'\x00')
out += dylib_cmd

# LC_DYLD_INFO_ONLY
out += struct.pack('<II', LC_DYLD_INFO_ONLY, 48)
out += struct.pack('<IIIIIIIIII', 0, 0, BIND_FILEOFF, bind_size, 0, 0, 0, 0, 0, 0)

# LC_MAIN
out += struct.pack('<IIQQ', LC_MAIN, 24, 0, 0)

assert len(out) <= TEXT_FILEOFF
out += b'\x00' * (TEXT_FILEOFF - len(out))
out += code
out += b'\x00' * (TEXT_VMSIZE - len(code))
assert len(out) == DATA_FILEOFF
out += b'\x00' * 16  # GOT
out += b'\x00' * (DATA_VMSIZE - 16)
assert len(out) == BIND_FILEOFF
out += bind_data
r = len(out) % 0x1000
if r: out += b'\x00' * (0x1000 - r)

output_path = sys.argv[1] if len(sys.argv) > 1 else "tests/fixtures/echo_dynamic.macho"
with open(output_path, 'wb') as f:
    f.write(out)
print(f"Generated {output_path} ({len(out)} bytes, code={len(code)} bytes)")
