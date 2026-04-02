#!/usr/bin/env python3
"""
Generate a minimal dynamically-linked x86_64 Mach-O binary.

Calls _write(1, msg, len) and _exit(0) through function pointers
resolved from libSystem.B.dylib via LC_DYLD_INFO_ONLY bind opcodes.

Layout:
  [Mach-O header]
  [LC_SEGMENT_64 __PAGEZERO]
  [LC_SEGMENT_64 __TEXT]
  [LC_SEGMENT_64 __DATA]       -- contains got[] with function pointers
  [LC_LOAD_DYLIB libSystem.B.dylib]
  [LC_DYLD_INFO_ONLY]          -- points to bind opcodes
  [LC_MAIN]                    -- entry offset into __TEXT
  [padding to 0x1000]
  [__TEXT content at 0x1000]   -- code that calls through got[]
  [__DATA content at 0x2000]   -- got[0]=_write, got[1]=_exit
  [bind opcodes at 0x3000]
"""

import struct
import sys

# --- Constants ---
MH_MAGIC_64     = 0xfeedfacf
CPU_TYPE_X86_64 = 0x01000007
CPU_SUBTYPE_ALL = 0x03
MH_EXECUTE      = 0x02
MH_NOUNDEFS     = 0x01
MH_DYLDLINK     = 0x04
MH_PIE          = 0x00200000

LC_SEGMENT_64       = 0x19
LC_LOAD_DYLIB       = 0x0c
LC_DYLD_INFO_ONLY   = 0x80000022
LC_MAIN             = 0x80000028

# Bind opcodes
BIND_OPCODE_SET_DYLIB_ORDINAL_IMM   = 0x10
BIND_OPCODE_SET_SYMBOL_TRAILING     = 0x11  # actually just the opcode byte; flags in low nibble
BIND_OPCODE_SET_TYPE_IMM            = 0x50
BIND_OPCODE_SET_SEGMENT_AND_OFFSET  = 0x70  # | segment_index
BIND_OPCODE_DO_BIND                 = 0x00
BIND_OPCODE_DONE                    = 0x00

BIND_TYPE_POINTER = 1

# --- Addresses ---
TEXT_VMADDR = 0x100000000
TEXT_FILEOFF = 0x1000
TEXT_VMSIZE = 0x1000

DATA_VMADDR = TEXT_VMADDR + TEXT_VMSIZE  # 0x100001000
DATA_FILEOFF = 0x2000
DATA_VMSIZE = 0x1000

BIND_FILEOFF = 0x3000  # bind opcodes location in file

# GOT layout in __DATA: got[0] = _write, got[1] = _exit
GOT_WRITE_ADDR = DATA_VMADDR + 0   # 0x100001000
GOT_EXIT_ADDR  = DATA_VMADDR + 8   # 0x100001008

# --- Machine code ---
# RIP-relative addressing: code at TEXT_VMADDR, GOT at DATA_VMADDR = TEXT_VMADDR + 0x1000.

msg = b"Hello from Grafted!\n"
msg_len = len(msg)

code = bytearray()

# --- _write(1, msg, 20) ---
# lea rsi, [rip + msg_offset]
lea_msg_pos = len(code)
code += b'\x48\x8d\x35\x00\x00\x00\x00'  # placeholder, patch later

# mov edi, 1
code += b'\xbf\x01\x00\x00\x00'

# mov edx, msg_len
code += b'\xba' + struct.pack('<I', msg_len)

# call [rip + got_write_offset]
call_write_pos = len(code)
code += b'\xff\x15\x00\x00\x00\x00'  # placeholder, patch later

# --- _exit(0) ---
# xor edi, edi
code += b'\x31\xff'

# call [rip + got_exit_offset]
call_exit_pos = len(code)
code += b'\xff\x15\x00\x00\x00\x00'  # placeholder, patch later

# int3
code += b'\xcc'

msg_offset_in_text = len(code)
code += msg

# --- Patch RIP-relative offsets ---
# lea rsi, [rip + msg_offset]: RIP after this instruction = lea_msg_pos + 7
# target = TEXT_VMADDR + msg_offset_in_text
# but since both are in the same segment, relative offset = msg_offset_in_text - (lea_msg_pos + 7)
lea_rip = lea_msg_pos + 7
struct.pack_into('<i', code, lea_msg_pos + 3, msg_offset_in_text - lea_rip)

# call [rip + got_write_offset]: RIP after = call_write_pos + 6
# target = GOT_WRITE_ADDR, code is at TEXT_VMADDR
# offset = GOT_WRITE_ADDR - (TEXT_VMADDR + call_write_pos + 6)
call_write_rip = call_write_pos + 6
got_write_rel = (GOT_WRITE_ADDR) - (TEXT_VMADDR + call_write_rip)
struct.pack_into('<i', code, call_write_pos + 2, got_write_rel)

# call [rip + got_exit_offset]
call_exit_rip = call_exit_pos + 6
got_exit_rel = (GOT_EXIT_ADDR) - (TEXT_VMADDR + call_exit_rip)
struct.pack_into('<i', code, call_exit_pos + 2, got_exit_rel)

ENTRY_OFF = 0

# --- Build bind opcodes ---
# Segment indices: __PAGEZERO=0, __TEXT=1, __DATA=2.

# Bind opcode encoding: high nibble = opcode, low nibble = immediate.
# 0x10 = SET_DYLIB_ORDINAL_IMM, 0x40 = SET_SYMBOL_TRAILING_FLAGS_IMM,
# 0x50 = SET_TYPE_IMM, 0x70 = SET_SEGMENT_AND_OFFSET_ULEB (+ ULEB offset),
# 0x90 = DO_BIND (advances address by pointer size), 0x00 = DONE.

bind_data = bytearray()

bind_data.append(0x10 | 1)             # SET_DYLIB_ORDINAL_IMM(1)
bind_data.append(0x50 | BIND_TYPE_POINTER)  # SET_TYPE_IMM(POINTER=1)
bind_data.append(0x40 | 0)             # SET_SYMBOL_TRAILING_FLAGS_IMM(flags=0)
bind_data += b'_write\x00'
bind_data.append(0x70 | 2)             # SET_SEGMENT_AND_OFFSET_ULEB(seg=2)
bind_data.append(0x00)                 # ULEB128 offset = 0
bind_data.append(0x90)                 # DO_BIND

bind_data.append(0x40 | 0)             # SET_SYMBOL_TRAILING_FLAGS_IMM(flags=0)
bind_data += b'_exit\x00'
bind_data.append(0x90)                 # DO_BIND

bind_data.append(0x00)                 # DONE

bind_size = len(bind_data)

# --- Build Mach-O ---

HEADER_SIZE = 32

# Load commands:
# 1. LC_SEGMENT_64 __PAGEZERO (72 bytes)
# 2. LC_SEGMENT_64 __TEXT (72 bytes)
# 3. LC_SEGMENT_64 __DATA (72 bytes)
# 4. LC_LOAD_DYLIB (variable, padded to 8-byte align)
# 5. LC_DYLD_INFO_ONLY (48 bytes)
# 6. LC_MAIN (24 bytes)

SEG_CMD_SIZE = 72

dylib_path = b"/usr/lib/libSystem.B.dylib\x00"
# LC_LOAD_DYLIB: cmd(4) + cmdsize(4) + str_offset(4) + timestamp(4) + current_ver(4) + compat_ver(4) + string
DYLIB_CMD_FIXED = 24  # fixed part
dylib_cmd_size = DYLIB_CMD_FIXED + len(dylib_path)
dylib_cmd_size = (dylib_cmd_size + 7) & ~7  # 8-byte align

DYLD_INFO_CMD_SIZE = 48
MAIN_CMD_SIZE = 24

ncmds = 6
sizeofcmds = (SEG_CMD_SIZE * 3) + dylib_cmd_size + DYLD_INFO_CMD_SIZE + MAIN_CMD_SIZE

header = struct.pack('<IIIIIIII',
    MH_MAGIC_64,
    CPU_TYPE_X86_64,
    CPU_SUBTYPE_ALL,
    MH_EXECUTE,
    ncmds,
    sizeofcmds,
    MH_NOUNDEFS | MH_DYLDLINK | MH_PIE,
    0,  # reserved
)

def seg_cmd(name, vmaddr, vmsize, fileoff, filesize, maxprot, initprot):
    name_bytes = name.encode('ascii').ljust(16, b'\x00')[:16]
    return struct.pack('<II', LC_SEGMENT_64, SEG_CMD_SIZE) + name_bytes + struct.pack('<QQQQIIII',
        vmaddr, vmsize, fileoff, filesize, maxprot, initprot, 0, 0)

# __PAGEZERO
pagezero = seg_cmd("__PAGEZERO", 0, TEXT_VMADDR, 0, 0, 0, 0)

# __TEXT (r-x)
text_seg = seg_cmd("__TEXT", TEXT_VMADDR, TEXT_VMSIZE, TEXT_FILEOFF, len(code), 5, 5)

# __DATA (rw-)
data_seg = seg_cmd("__DATA", DATA_VMADDR, DATA_VMSIZE, DATA_FILEOFF, 16, 3, 3)

# LC_LOAD_DYLIB
dylib_cmd = struct.pack('<II', LC_LOAD_DYLIB, dylib_cmd_size)
dylib_cmd += struct.pack('<I', DYLIB_CMD_FIXED)  # str offset from cmd start
dylib_cmd += struct.pack('<I', 0)      # timestamp
dylib_cmd += struct.pack('<I', 0x10000)  # current_version
dylib_cmd += struct.pack('<I', 0x10000)  # compat_version
dylib_cmd += dylib_path
dylib_cmd = dylib_cmd.ljust(dylib_cmd_size, b'\x00')

# LC_DYLD_INFO_ONLY
dyld_info = struct.pack('<II', LC_DYLD_INFO_ONLY, DYLD_INFO_CMD_SIZE)
dyld_info += struct.pack('<II', 0, 0)              # rebase_off, rebase_size
dyld_info += struct.pack('<II', 0, 0)              # bind_off, bind_size (non-lazy)
dyld_info += struct.pack('<II', 0, 0)              # weak_bind_off, weak_bind_size
dyld_info += struct.pack('<II', BIND_FILEOFF, bind_size)  # lazy_bind_off, lazy_bind_size
dyld_info += struct.pack('<II', 0, 0)              # export_off, export_size

# LC_MAIN
main_cmd = struct.pack('<II', LC_MAIN, MAIN_CMD_SIZE)
main_cmd += struct.pack('<QQ', ENTRY_OFF, 0)  # entryoff from __TEXT start, stacksize

out = bytearray()
out += header
out += pagezero
out += text_seg
out += data_seg
out += dylib_cmd
out += dyld_info
out += main_cmd

assert len(out) <= TEXT_FILEOFF, f"headers too large: {len(out)} > {TEXT_FILEOFF}"

out += b'\x00' * (TEXT_FILEOFF - len(out))

# __TEXT content
out += code
out += b'\x00' * (TEXT_VMSIZE - len(code))

# __DATA: GOT slots (patched by linker)
assert len(out) == DATA_FILEOFF
out += b'\x00' * 8  # got[0] = _write
out += b'\x00' * 8  # got[1] = _exit
out += b'\x00' * (DATA_VMSIZE - 16)

assert len(out) == BIND_FILEOFF
out += bind_data
remainder = len(out) % 0x1000
if remainder:
    out += b'\x00' * (0x1000 - remainder)

output_path = sys.argv[1] if len(sys.argv) > 1 else "tests/fixtures/hello_dynamic.macho"
with open(output_path, 'wb') as f:
    f.write(out)

print(f"Generated {output_path} ({len(out)} bytes)")
print(f"  Code size: {len(code)} bytes (includes {msg_len} byte message)")
print(f"  Entry: __TEXT+{ENTRY_OFF:#x} (LC_MAIN entryoff={TEXT_FILEOFF + ENTRY_OFF:#x})")
print(f"  __TEXT: vmaddr={TEXT_VMADDR:#x} fileoff={TEXT_FILEOFF:#x}")
print(f"  __DATA: vmaddr={DATA_VMADDR:#x} fileoff={DATA_FILEOFF:#x}")
print(f"  GOT[0] (_write): {GOT_WRITE_ADDR:#x}")
print(f"  GOT[1] (_exit):  {GOT_EXIT_ADDR:#x}")
print(f"  Bind opcodes: fileoff={BIND_FILEOFF:#x} size={bind_size}")
print(f"  Depends on: /usr/lib/libSystem.B.dylib")
