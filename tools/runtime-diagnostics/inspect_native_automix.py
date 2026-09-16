#!/usr/bin/env python3
"""Read-only bounded native disassembly around named Automix diagnostics.

Dependencies: pefile, capstone. No process injection or memory/credential reads.
Only private analysis output is intended; do not commit disassembled client code.
"""
import argparse
import bisect
import hashlib
import json
import re
import struct
from pathlib import Path

import capstone
import pefile

NEEDLES = [
    "Calculating auto transition for", "Transition recipe mismatch:",
    "Stored transition has unknown preset:", "Stored transition has preset NONE",
    "Failed to decode transition recipe", "Transition recipe has unknown preset:",
    "automix.auto_transition_recipe", "automix.backend_auto_transition",
    "use_backend_auto_transition", "Using cuepoints-based fallback transition",
    "AutomixBundleImpl locale:", "spotify:core-auto-transition",
]


def dump_function(dll, output, target):
    raw = dll.read_bytes()
    pe = pefile.PE(data=raw)
    base = pe.OPTIONAL_HEADER.ImageBase
    functions = sorted((x.struct.BeginAddress, x.struct.EndAddress) for x in pe.DIRECTORY_ENTRY_EXCEPTION)
    starts = [x[0] for x in functions]
    start, end = functions[bisect.bisect_right(starts, target)-1]
    if not start <= target < end:
        raise ValueError("no enclosing unwind function")
    md = capstone.Cs(capstone.CS_ARCH_X86, capstone.CS_MODE_64)
    md.skipdata = True
    pattern = re.compile(r"\[rip ([+-]) (0x[0-9a-f]+)\]")
    lines = []
    for addr, size, mnemonic, operands in md.disasm_lite(pe.get_data(start,end-start),base+start):
        annotation = ""
        match = pattern.search(operands)
        if match:
            pointer = addr+size+int(match[2],16)*(1 if match[1]=="+" else -1)-base
            value = pe.get_data(pointer,300).split(b"\x00",1)[0]
            if len(value)>=5 and all(32<=x<127 for x in value):
                annotation = " ; " + value.decode("ascii")
        lines.append(f"{addr-base:08x}  {mnemonic:10s} {operands}{annotation}")
    output.mkdir(parents=True,exist_ok=True)
    path = output / f"function-{start:08x}.asm"
    path.write_text("\n".join(lines)+"\n",encoding="utf-8")
    callers = []
    for section in pe.sections:
        if not section.Characteristics & 0x20000000:
            continue
        data = section.get_data()
        offset = 0
        while (offset := data.find(b"\xe8",offset)) >= 0:
            if offset+5 <= len(data):
                addr = section.VirtualAddress+offset
                if addr+5+struct.unpack_from("<i",data,offset+1)[0]==start:
                    caller_start, caller_end = functions[bisect.bisect_right(starts,addr)-1]
                    # Report candidates; a caller must be confirmed on instruction
                    # boundaries before treating the byte pattern as a reference.
                    callers.append(dict(call_rva=hex(addr),function_rva=hex(caller_start)))
            offset += 1
    print(json.dumps(dict(path=str(path),start=hex(start),end=hex(end),callers=callers)))


def inspect(dll, output):
    raw = dll.read_bytes()
    pe = pefile.PE(data=raw)
    base = pe.OPTIONAL_HEADER.ImageBase
    wanted = {}
    for needle in NEEDLES:
        pos = 0
        while (pos := raw.find(needle.encode(), pos)) >= 0:
            wanted[base + pe.get_rva_from_offset(pos)] = needle
            pos += 1
    md = capstone.Cs(capstone.CS_ARCH_X86, capstone.CS_MODE_64)
    md.skipdata = True
    refs = []
    pattern = re.compile(r"\[rip ([+-]) (0x[0-9a-f]+)\]")
    for section in pe.sections:
        if not section.Characteristics & 0x20000000:
            continue
        for addr, size, mnemonic, operands in md.disasm_lite(section.get_data(), base + section.VirtualAddress):
            match = pattern.search(operands)
            if match:
                target = addr + size + int(match[2], 16) * (1 if match[1] == "+" else -1)
                if target in wanted:
                    refs.append(dict(needle=wanted[target], address=addr, rva=addr-base, target=target))
    functions = sorted((x.struct.BeginAddress, x.struct.EndAddress) for x in pe.DIRECTORY_ENTRY_EXCEPTION)
    starts = [start for start, _ in functions]
    output.mkdir(parents=True, exist_ok=True)
    for ref in refs:
        index = bisect.bisect_right(starts, ref["rva"]) - 1
        start, end = functions[index]
        if not start <= ref["rva"] < end:
            continue
        ref["function_rva"] = start
        ref["function_end_rva"] = end
        body = pe.get_data(start, min(end-start, 100000))
        lines = [f"{addr-base:08x}  {mnemonic:10s} {operands}" for addr, _, mnemonic, operands in md.disasm_lite(body, base+start)]
        (output / f"function-{start:08x}.asm").write_text("\n".join(lines)+"\n", encoding="utf-8")
    report = dict(dll_sha256=hashlib.sha256(raw).hexdigest(), image_base=base, references=refs)
    (output / "references.json").write_text(json.dumps(report,indent=2),encoding="utf-8")
    print(json.dumps(report))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("dll",type=Path)
    parser.add_argument("output",type=Path)
    parser.add_argument("--function-rva",type=lambda value:int(value,0))
    args = parser.parse_args()
    if args.function_rva is not None:
        dump_function(args.dll,args.output,args.function_rva)
    else:
        inspect(args.dll,args.output)
