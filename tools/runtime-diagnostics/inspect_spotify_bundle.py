#!/usr/bin/env python3
"""Extract the narrow Automix bundle/descriptors from an owned Spotify DLL.

Requires protobuf in an isolated diagnostic environment. Output stays private
until reviewed. Does not load/execute the DLL or extract unrelated resources.
"""
import argparse
import hashlib
import io
import json
import struct
import zipfile
from pathlib import Path

from google.protobuf import descriptor_pb2, descriptor_pool, message_factory, json_format


def read_varint(data, pos):
    value = 0
    for shift in range(0, 70, 7):
        byte = data[pos]
        pos += 1
        value |= (byte & 127) << shift
        if byte < 128:
            return value, pos
    raise ValueError("invalid varint")


def descriptor_at(data, start):
    pos = start
    while pos < min(len(data), start + 200000):
        tag, after = read_varint(data, pos)
        if tag >> 3 == 0 or tag >> 3 > 14:
            break
        wire = tag & 7
        if wire == 2:
            size, after = read_varint(data, after)
            end = after + size
        elif wire == 0:
            _, end = read_varint(data, after)
        else:
            break
        if end > len(data):
            break
        pos = end
        if tag == 98 and data[after:end] in (b"proto2", b"proto3"):
            break
    value = descriptor_pb2.FileDescriptorProto.FromString(data[start:pos])
    if not value.name or not value.message_type:
        raise ValueError("not a message file descriptor")
    return value, data[start:pos]


def find_descriptor(data, name):
    encoded = name.encode()
    if len(encoded) >= 128:
        raise ValueError("descriptor name too long for narrow probe")
    needle = b"\x0a" + bytes([len(encoded)]) + encoded
    offset = 0
    while (offset := data.find(needle, offset)) >= 0:
        try:
            descriptor, raw = descriptor_at(data, offset)
            if descriptor.name == name:
                return descriptor, raw, offset
        except (ValueError, IndexError):
            pass
        offset += 1
    raise ValueError(f"descriptor not found: {name}")


def extract(dll, output):
    data = dll.read_bytes()
    name_at = data.index(b"bundle-proto.bin")
    local_start = data.rfind(b"PK\x03\x04", 0, name_at + 1)
    # The first textual filename may precede the embedded archive. Find the
    # actual local-file header whose filename is bundle-proto.bin.
    if local_start < 0 or name_at - local_start != 30:
        local_start = data.index(b"PK\x03\x04", name_at)
    end = data.index(b"PK\x05\x06", local_start)
    comment_len = struct.unpack_from("<H", data, end + 20)[0]
    archive = data[local_start:end + 22 + comment_len]
    with zipfile.ZipFile(io.BytesIO(archive)) as z:
        bundle = z.read("bundle-proto.bin")
        entries = z.namelist()
    output.mkdir(parents=True, exist_ok=True)
    (output / "bundle-proto.bin").write_bytes(bundle)
    pending = ["mixing_bundle.proto", "es_automix_bundle.proto", "es_automix.proto"]
    descriptors = {}
    provenance = []
    while pending:
        name = pending.pop(0)
        if name in descriptors:
            continue
        descriptor, raw, offset = find_descriptor(data, name)
        descriptors[name] = descriptor
        pending.extend(descriptor.dependency)
        (output / (Path(name).name + ".descriptor.bin")).write_bytes(raw)
        provenance.append(dict(name=name, offset=offset, bytes=len(raw), sha256=hashlib.sha256(raw).hexdigest()))
    pool = descriptor_pool.DescriptorPool()
    remaining = dict(descriptors)
    while remaining:
        progress = False
        for name, descriptor in list(remaining.items()):
            if all(dep not in remaining for dep in descriptor.dependency):
                pool.Add(descriptor)
                del remaining[name]
                progress = True
        if not progress:
            raise ValueError("unresolved descriptor dependencies")
    cls = message_factory.GetMessageClass(pool.FindMessageTypeByName("spotify.automix.bundle.proto.MixingBundle"))
    decoded = cls.FromString(bundle)
    (output / "mixing-bundle.json").write_text(json_format.MessageToJson(decoded), encoding="utf-8")
    collection = descriptor_pb2.FileDescriptorSet()
    collection.file.extend(descriptors.values())
    (output / "automix-descriptors.pb").write_bytes(collection.SerializeToString())
    (output / "automix-descriptors.json").write_text(json_format.MessageToJson(collection), encoding="utf-8")
    report = dict(dll_sha256=hashlib.sha256(data).hexdigest(), archive_offset=local_start,
                  archive_bytes=len(archive), entries=entries, bundle_bytes=len(bundle),
                  bundle_sha256=hashlib.sha256(bundle).hexdigest(), descriptors=provenance)
    (output / "provenance.json").write_text(json.dumps(report, indent=2), encoding="utf-8")
    print(json.dumps(report))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("dll", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    extract(args.dll, args.output)
