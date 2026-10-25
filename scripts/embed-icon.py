"""Embed the application icon into a Windows PE executable.

`lld-link` (used by cargo-xwin) does not accept .res files and there is no
resource compiler in the WSL toolchain, so the resource section is added after
linking with LIEF:

    python3 -m venv /tmp/liefenv
    /tmp/liefenv/bin/pip install lief
    /tmp/liefenv/bin/python scripts/embed-icon.py target/.../octocaris.exe assets/octocaris.ico out.exe

The script builds a standard .rsrc tree (RT_ICON + RT_GROUP_ICON with id 1),
adds it as a new section, points the resource data directory at it, and patches
SizeOfRawData (LIEF leaves it at 0 for the new section).
"""
import struct, sys
from pathlib import Path
import lief

def parse_ico(data):
    reserved, typ, count = struct.unpack_from('<HHH', data, 0)
    assert reserved == 0 and typ == 1, 'not an ico'
    images=[]
    for i in range(count):
        w,h,colors,resv,planes,bpp,size,offset = struct.unpack_from('<BBBBHHII', data, 6+i*16)
        images.append((w,h,colors,resv,planes,bpp,data[offset:offset+size]))
    return images

def align(n, a=4): return (n + a - 1) & ~(a - 1)

def build_rsrc(images, base):
    n=len(images)
    root_off=0; root_size=16+2*8
    dir3_off=root_size; dir3_size=16+n*8
    lang3_off=dir3_off+dir3_size; lang3_size=n*24
    dir14_off=lang3_off+lang3_size; dir14_size=16+8
    lang14_off=dir14_off+dir14_size; lang14_size=24
    dataent_off=lang14_off+lang14_size; dataent_size=(n+1)*16
    data_start=align(dataent_off+dataent_size)
    offsets=[]; cur=data_start
    for w,h,c,r,p,b,img in images:
        offsets.append(cur); cur=align(cur+len(img))
    group=struct.pack('<HHH',0,1,n)
    for i,(w,h,c,r,p,b,img) in enumerate(images,1):
        group+=struct.pack('<BBBBHHIH',w,h,c,r,p,b,len(img),i)
    group_off=cur; total=align(group_off+len(group))
    buf=bytearray(total)
    def put_dir(off, entries):
        struct.pack_into('<IIHHHH', buf, off, 0,0,0,0,0,len(entries))
        for i,(rid,target,is_dir) in enumerate(entries):
            struct.pack_into('<II', buf, off+16+i*8, rid, (0x80000000|target) if is_dir else target)
    def put_data(off, rva, size):
        struct.pack_into('<IIII', buf, off, rva, size, 0, 0)
    put_dir(root_off, [(3,dir3_off,True),(14,dir14_off,True)])
    put_dir(dir3_off, [(i+1, lang3_off+i*24, True) for i in range(n)])
    for i in range(n):
        put_dir(lang3_off+i*24, [(0x0409, dataent_off+i*16, False)])
    put_dir(dir14_off, [(1, lang14_off, True)])
    put_dir(lang14_off, [(0x0409, dataent_off+n*16, False)])
    for i,(w,h,c,r,p,b,img) in enumerate(images):
        put_data(dataent_off+i*16, base+offsets[i], len(img))
        buf[offsets[i]:offsets[i]+len(img)]=img
    put_data(dataent_off+n*16, base+group_off, len(group))
    buf[group_off:group_off+len(group)]=group
    return bytes(buf)

def main(exe, ico, out):
    images=parse_ico(Path(ico).read_bytes())
    binary=lief.PE.parse(exe)
    if binary is None: raise SystemExit('parse failed')
    if binary.has_resources: raise SystemExit('already has resources')
    sec=lief.PE.Section('.rsrc')
    sec.characteristics = (lief.PE.Section.CHARACTERISTICS.CNT_INITIALIZED_DATA |
                           lief.PE.Section.CHARACTERISTICS.MEM_READ)
    sec = binary.add_section(sec)
    if sec is None: raise SystemExit('add_section failed')
    base=sec.virtual_address
    blob=build_rsrc(images, base)
    sec.content=list(blob)
    sec.virtual_size=len(blob)
    dd=binary.data_directory(lief.PE.DataDirectory.TYPES.RESOURCE_TABLE)
    dd.rva=base
    dd.size=len(blob)
    binary.write(out)

    # LIEF writes the resource bytes at the end of the file and updates
    # VirtualSize/DataDirectory, but leaves SizeOfRawData at 0. Patch the
    # section header and pad the file to the aligned raw size so the Windows
    # loader maps the resource section.
    data = bytearray(Path(out).read_bytes())
    e_lfanew = struct.unpack_from('<I', data, 0x3c)[0]
    coff = e_lfanew + 4
    section_count = struct.unpack_from('<H', data, coff + 2)[0]
    optional_size = struct.unpack_from('<H', data, coff + 16)[0]
    section_table = coff + 20 + optional_size
    for index in range(section_count):
        header = section_table + index * 40
        virtual_address = struct.unpack_from('<I', data, header + 12)[0]
        if virtual_address != base:
            continue
        raw_pointer = struct.unpack_from('<I', data, header + 20)[0]
        raw_size = align(len(blob), 0x200)
        struct.pack_into('<I', data, header + 16, raw_size)
        end = raw_pointer + raw_size
        if len(data) < end:
            data.extend(b'\0' * (end - len(data)))
        Path(out).write_bytes(data)
        break

    print('wrote', out, 'section rva', hex(base), 'blob', len(blob))

if __name__=='__main__':
    main(sys.argv[1], sys.argv[2], sys.argv[3])
