#!/usr/bin/env python3
"""Regenerates methods.zip: the same text stored with each compression method.

Needs the `zstd` command. Run from this directory.
"""
import bz2
import lzma
import struct
import subprocess
import zlib

TEXT = b"Every compression method should extract to exactly this text.\n" * 40


def deflate(data):
    compressor = zlib.compressobj(9, zlib.DEFLATED, -15)
    return compressor.compress(data) + compressor.flush()


def zip_lzma(data):
    # ZIP's method 14: a version, the size of the properties, the properties, then raw LZMA.
    filters = [{"id": lzma.FILTER_LZMA1, "preset": 6}]
    body = lzma.compress(data, format=lzma.FORMAT_RAW, filters=filters)
    return struct.pack("<BBH", 9, 20, 5) + lzma._encode_filter_properties(filters[0]) + body


def zstd(data):
    return subprocess.run(["zstd", "-c", "-q"], input=data, stdout=subprocess.PIPE, check=True).stdout


METHODS = [
    ("stored.txt", 0, lambda data: data),
    ("deflate.txt", 8, deflate),
    ("bzip2.txt", 12, bz2.compress),
    ("lzma.txt", 14, zip_lzma),
    ("zstd.txt", 93, zstd),
    ("xz.txt", 95, lzma.compress),
]

body, directory = b"", b""
crc = zlib.crc32(TEXT)
for name, method, compress in METHODS:
    data = compress(TEXT)
    common = struct.pack("<HHHHHIIIHH", 20, 0x0800, method, 0, 0, crc, len(data), len(TEXT), len(name), 0)
    directory += b"PK\x01\x02" + struct.pack("<H", 20) + common
    directory += struct.pack("<HHHII", 0, 0, 0, 0, len(body)) + name.encode()
    body += b"PK\x03\x04" + common + name.encode() + data

end = struct.pack(
    "<4sHHHHIIH", b"PK\x05\x06", 0, 0, len(METHODS), len(METHODS), len(directory), len(body), 0
)
with open("methods.zip", "wb") as output:
    output.write(body + directory + end)
