#!/usr/bin/env python3
"""Offline, presence-preserving inventory of already captured Transition recipes.

Reads a sanitized stage-trace export. Does not request music, control playback,
or access credentials. Field names come from automix_transition.proto; unknown
fields remain wire-numbered. Nested preset overrides remain opaque here.
"""
import argparse
import base64
import hashlib
import json
import struct
from pathlib import Path

from inspect_preview import fields


OVERLAP = {
    1: ("track_a_row_id", "string"), 2: ("track_b_row_id", "string"),
    3: ("start_a_ms", "int32"), 4: ("start_b_ms", "int32"),
    5: ("duration_ms", "int32"), 6: ("speed_a", "float"),
    7: ("speed_b", "float"), 8: ("duration_bars", "int32"),
    9: ("is_beatmatched", "bool"), 10: ("track_a_uri", "string"),
    11: ("track_b_uri", "string"), 12: ("track_a_playable_uri", "string"),
    13: ("track_b_playable_uri", "string"),
    14: ("track_a_playable_beats_hash", "string"),
    15: ("track_b_playable_beats_hash", "string"),
    16: ("bpm_a", "float"), 17: ("bpm_b", "float"),
    18: ("item_speed_a", "double"), 19: ("item_speed_b", "double"),
}
PRESET = {1: ("id", "int32"), 2: ("type", "enum")}
TRANSITION = {
    1: ("overlap", OVERLAP), 2: ("preset", PRESET),
    3: ("is_overlap_override", "bool"),
    4: ("is_preset_id_override", "bool"),
    5: ("beatmatch_preference", "enum"),
}


def inventory(data, schema):
    result = []
    for number, wire, value in fields(data):
        name, kind = schema.get(number, (f"unknown_{number}", "opaque"))
        expected = 2 if isinstance(kind, dict) else {
            "string": 2, "int32": 0, "enum": 0, "bool": 0,
            "float": 5, "double": 1,
        }.get(kind)
        if expected is not None and expected != wire:
            raise ValueError(f"field {number}: expected wire {expected}, got {wire}")
        if isinstance(kind, dict):
            value = inventory(value, kind)
            kind = "message"
        elif kind == "string":
            value = value.decode("utf-8", errors="strict")
        elif kind in ("float", "double"):
            value = struct.unpack("<f" if kind == "float" else "<d", value)[0]
        elif kind == "int32":
            value &= 0xffffffff
            if value >= 0x80000000:
                value -= 0x100000000
        elif kind == "bool":
            value = bool(value)
        elif isinstance(value, bytes):
            value = {"base64": base64.b64encode(value).decode("ascii")}
        result.append(dict(field=number, name=name, type=kind, wire=wire, value=value))
    return result


def extract(source, output):
    root = json.loads(source.read_text(encoding="utf-8-sig"))
    records = root.get("trace", root)["records"]
    samples = {}
    for record in records:
        # Native queue stream is the observation boundary; avoid duplicated UI
        # projections and transient PlayerAPI old-context/new-track combinations.
        if record.get("stage") != "context-player-queue-stream":
            continue
        edge = record.get("edge", {})
        metadata = (edge.get("current") or {}).get("metadata", {})
        for key in ("automix.auto_transition_recipe", "automix.backend_auto_transition"):
            encoded = metadata.get(key)
            if not encoded:
                continue
            if len(encoded) > 16384:
                raise ValueError("recipe exceeds diagnostic size bound")
            raw = base64.b64decode(encoded, validate=True)
            digest = hashlib.sha256(raw).hexdigest()
            if digest not in samples:
                samples[digest] = dict(
                    sha256=digest, bytes=len(raw), base64=encoded,
                    inventory=inventory(raw, TRANSITION), observations=[],
                )
            samples[digest]["observations"].append(dict(
                utc=record.get("capturedAt"), stage=record["stage"], key=key,
                outgoing_uri=edge.get("outgoingUri"), incoming_uri=edge.get("incomingUri"),
                transition_uri=metadata.get("automix.transition_uri"),
            ))
    output.mkdir(parents=True, exist_ok=True)
    report = dict(
        source_sha256=hashlib.sha256(source.read_bytes()).hexdigest(),
        boundary="official native ContextPlayer.GetQueue -> XPUI", samples=list(samples.values()),
    )
    (output / "recipes.json").write_text(json.dumps(report, indent=2, allow_nan=False), encoding="utf-8")
    for digest, sample in samples.items():
        (output / f"{digest}.bin").write_bytes(base64.b64decode(sample["base64"]))
    print(json.dumps(dict(samples=len(samples), output=str(output))))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    extract(args.source, args.output)
