#!/usr/bin/env python3
"""Extract preview payloads from private journal JSONL, without transport secrets."""
import argparse
import base64
import hashlib
import json
from pathlib import Path
import re


def varint(data, pos):
    value = 0
    for shift in range(0, 70, 7):
        byte = data[pos]
        pos += 1
        value |= (byte & 127) << shift
        if not byte & 128:
            return value, pos
    raise ValueError("invalid varint")


def fields(data):
    pos = 0
    result = []
    while pos < len(data):
        tag, pos = varint(data, pos)
        number, wire = tag >> 3, tag & 7
        if not number:
            raise ValueError("zero field")
        if wire == 0:
            value, pos = varint(data, pos)
        elif wire in (1, 5):
            size = 8 if wire == 1 else 4
            value = data[pos:pos + size]
            pos += size
        elif wire == 2:
            size, pos = varint(data, pos)
            value = data[pos:pos + size]
            pos += size
        else:
            raise ValueError(f"wire {wire}")
        if pos > len(data):
            raise ValueError("truncated")
        result.append((number, wire, value))
    return result


def extract(journal, output):
    output.mkdir(parents=True, exist_ok=True)
    messages = []
    for line in journal.read_text(encoding="utf-8-sig", errors="replace").splitlines():
        try:
            row = json.loads(line)
            message = row.get("MESSAGE", "")
            if isinstance(message, list):
                message = bytes(message).decode(errors="replace")
            messages.append((row.get("__REALTIME_TIMESTAMP"), message))
        except json.JSONDecodeError:
            continue
    # Existing fern multiline errors can span several journal records.
    text = "\n".join(message for _, message in messages)
    payloads = re.findall(r'"parameters": String\("([A-Za-z0-9+/=]+)"\)', text)
    payloads += re.findall(r'preview_parameters=([A-Za-z0-9+/=]+)', text)
    for index, encoded in enumerate(payloads):
        data = base64.b64decode(encoded, validate=True)
        stem = output / f"preview-{index:03d}"
        stem.with_suffix(".bin").write_bytes(data)
        inventory = []
        for number, wire, value in fields(data):
            item = dict(field=number, wire=wire)
            if isinstance(value, bytes):
                item["bytes"] = len(value)
                if wire == 2:
                    try:
                        decoded = value.decode("utf-8")
                        if all(c.isprintable() for c in decoded):
                            item["text"] = decoded
                        else:
                            raise ValueError("binary")
                    except (UnicodeDecodeError, ValueError):
                        (output / f"preview-{index:03d}-field-{number}.bin").write_bytes(value)
                else:
                    item["hex"] = value.hex()
            else:
                item["value"] = value
            inventory.append(item)
        report = dict(index=index, bytes=len(data), sha256=hashlib.sha256(data).hexdigest(), fields=inventory)
        stem.with_suffix(".json").write_text(json.dumps(report, indent=2))
        print(json.dumps(report))
    print(f"captures={len(payloads)}")


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("journal", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    extract(args.journal, args.output)
