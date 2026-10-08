"""Offline analysis of a lab memory dump (local/lab/graph.kctd, written by the mod's F7 lab key).

Finds Havok skeletons (hkaSkeleton-like: parentIndices + bones arrays whose entries name bones) and
prints the pointer path from each root to them, plus where bone-name strings live.
usage: python -I tools/analyze_dump.py [dump]
"""
import bisect
import struct
import sys
from collections import defaultdict, deque

path = sys.argv[1] if len(sys.argv) > 1 else r"V:\EldenCome2\local\lab\graph.kctd"
data = open(path, "rb").read()
assert data[:4] == b"KCTD"
nroots = struct.unpack_from("<I", data, 4)[0]
roots, o = [], 8
for _ in range(nroots):
    a = struct.unpack_from("<Q", data, o)[0]
    roots.append((data[o + 8:o + 24].rstrip(b"\0").decode(), a))
    o += 24
nodes = {}
while o < len(data):
    a, ln = struct.unpack_from("<QI", data, o)
    nodes[a] = data[o + 12:o + 12 + ln]
    o += 12 + ln
starts = sorted(nodes)
print(f"{len(nodes)} nodes, roots {[(n, hex(a)) for n, a in roots]}")


def rd(addr, n):
    i = bisect.bisect_right(starts, addr) - 1
    if i < 0:
        return None
    s = starts[i]
    b = nodes[s]
    if addr + n <= s + len(b):
        return b[addr - s:addr - s + n]
    return None


def q(addr):
    b = rd(addr, 8)
    return struct.unpack("<Q", b)[0] if b else None


def i32(addr):
    b = rd(addr, 4)
    return struct.unpack("<i", b)[0] if b else None


def cstr(addr):
    addr &= ~1
    out = b""
    for k in range(64):
        b = rd(addr + k, 1)
        if b is None:
            return None
        if b == b"\0":
            break
        out += b
    try:
        s = out.decode("ascii")
    except UnicodeDecodeError:
        return None
    return s if len(s) >= 2 and all(c.isalnum() or c in "_.- " for c in s) else None


# 1) where do bone-like names live?
needles = [b"R_Hand\0", b"Pelvis\0", b"R_Weapon\0", b"Master\0", b"RootPos\0"]
hits = defaultdict(list)
for a, b in nodes.items():
    for nd in needles:
        k = b.find(nd)
        while k >= 0:
            if k == 0 or b[k - 1] == 0:
                hits[nd].append(a + k)
            k = b.find(nd, k + 1)
for nd, hs in hits.items():
    print(f"string {nd[:-1].decode()}: {len(hs)} hits, first {[hex(h) for h in hs[:4]]}")

# 2) arrays of {char*, x} with stride 0x10 whose names are bone-like
cands = []
str_targets = set()
for nd, hs in hits.items():
    str_targets.update(hs)
pointers_to = defaultdict(list)
for a, b in nodes.items():
    for k in range(0, len(b) - 7, 8):
        p = struct.unpack_from("<Q", b, k)[0]
        if p in str_targets:
            pointers_to[p].append(a + k)
for s, refs in list(pointers_to.items())[:6]:
    print(f"  {cstr(s)} @ {s:#x} referenced from {[hex(r) for r in refs[:4]]}")

# For each reference to a bone name, walk back to the array start (entries stride 0x10 or 0x8)
arrays = {}
for s, refs in pointers_to.items():
    for r in refs:
        for stride in (0x10, 0x8):
            st = r
            while True:
                p = q(st - stride)
                if p is None or cstr(p) is None:
                    break
                st -= stride
            n = 0
            while True:
                p = q(st + n * stride)
                if p is None or cstr(p) is None:
                    break
                n += 1
            if n >= 8:
                arrays[st] = (stride, n, [cstr(q(st + i * stride)) for i in range(min(n, 6))])
for st, (stride, n, first) in sorted(arrays.items()):
    print(f"bone-name array @ {st:#x}: stride {stride:#x}, {n} entries, first {first}")

# 3) who points at those arrays (hkArray data pointers) -> owning object + offset
owners = []
for st, (stride, n, _) in arrays.items():
    for a, b in nodes.items():
        for k in range(0, len(b) - 15, 8):
            if struct.unpack_from("<Q", b, k)[0] == st:
                size = struct.unpack_from("<i", b, k + 8)[0]
                owners.append((a, k, st, size, n, stride))
for a, k, st, size, n, stride in owners[:20]:
    print(f"  object {a:#x} +{k:#x} -> array {st:#x} (hkArray size field {size}, counted {n}, stride {stride:#x})")

# 4) BFS paths from roots to each owner object
parent = {}
dq = deque()
for name, a in roots:
    parent[a] = (None, name)
    dq.append(a)
while dq:
    a = dq.popleft()
    b = nodes.get(a)
    if b is None:
        continue
    for k in range(0, len(b) - 7, 8):
        p = struct.unpack_from("<Q", b, k)[0]
        if p in nodes and p not in parent:
            parent[p] = (a, k)
            dq.append(p)


def path_to(a):
    out = []
    while a in parent and parent[a][0] is not None:
        pa, off = parent[a]
        out.append(off)
        a = pa
    return parent.get(a, (None, "?"))[1], list(reversed(out))


for a, k, st, size, n, stride in owners[:20]:
    root, offs = path_to(a)
    print(f"  path {root} -> {[hex(x) for x in offs]} -> object {a:#x}; field +{k:#x}")
