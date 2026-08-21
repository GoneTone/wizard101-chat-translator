"""多層反向 pointer scanner(純讀):從一則獨特訊息字串,反向追指標鏈,
找到儲存在『模組固定區(IMAGE .data)』的穩定全域路徑 = 模組基址 + 偏移鏈。
用 numpy 向量化,每層考慮『指標指向物件頭』的偏移(maxoff)。
用法:python ptrscan.py <獨特訊息> [maxoff=0x600] [depth=6]
"""
import ctypes, ctypes.wintypes as wt, struct, sys, io
import numpy as np
sys.path.insert(0, r"E:\Projects\wiz101-chat-translator")
sys.stdout = io.TextIOWrapper(sys.stdout.buffer, encoding="utf-8", errors="replace")
from src.reader.mem_reader import (_find_pid, _MBI, PROCESS_QUERY_INFORMATION,
    PROCESS_VM_READ, _k32, _psapi, MEM_COMMIT, PAGE_GUARD, PAGE_NOACCESS,
    _c_void_p, _c_size_t)

MEM_IMAGE=0x1000000
msg = sys.argv[1] if len(sys.argv)>1 else "QZX99WIZ"
MAXOFF = int(sys.argv[2],0) if len(sys.argv)>2 else 0x600
DEPTH = int(sys.argv[3]) if len(sys.argv)>3 else 6

pid=_find_pid("WizardGraphicalClient.exe")
h=_k32.OpenProcess(PROCESS_QUERY_INFORMATION|PROCESS_VM_READ,False,pid)

# exe 模組基址
mods=(ctypes.c_void_p*1024)(); need=wt.DWORD()
_psapi.EnumProcessModules(h,mods,ctypes.sizeof(mods),ctypes.byref(need))
exe_base=mods[0]
print(f"模組基址 = {exe_base:#x}")

# 讀所有可讀區域
regions=[]
addr=0; mbi=_MBI()
while addr<0x7FFFFFFFFFFF:
    if not _k32.VirtualQueryEx(h,_c_void_p(addr),_c_void_p(ctypes.addressof(mbi)),ctypes.sizeof(mbi)): break
    base,size,ty=mbi.BaseAddress,mbi.RegionSize,mbi.Type
    if mbi.State==MEM_COMMIT and not(mbi.Protect&PAGE_GUARD) and mbi.Protect not in(0,PAGE_NOACCESS) and 0<size<512*1024*1024:
        buf=(ctypes.c_char*size)(); got=_c_size_t(0)
        if _k32.ReadProcessMemory(h,_c_void_p(base),ctypes.cast(buf,_c_void_p),size,ctypes.byref(got)) and got.value:
            regions.append((base,ty,bytes(buf[:got.value])))
    nxt=base+size
    if nxt<=addr: break
    addr=nxt
_k32.CloseHandle(h)
print(f"讀入 {len(regions)} 區域")

# 預備:每區域的 uint64 view(8 對齊)
views=[]
for base,ty,blob in regions:
    n=len(blob)//8
    v=np.frombuffer(blob[:n*8],dtype='<u8')
    views.append((base,ty,v))

def is_image(a):
    for base,ty,blob in regions:
        if base<=a<base+len(blob): return ty==MEM_IMAGE
    return False

# level 0: 純字串位址(非 markup)
needle=msg.encode("utf-16-le")
level0=[]
for base,ty,blob in regions:
    k=blob.find(needle)
    while k>=0:
        ctx=blob[max(0,k-24):k].decode("utf-16-le","replace")
        if not("<" in ctx or "]" in ctx or "color" in ctx):
            level0.append(base+k)
        k=blob.find(needle,k+1)
print(f"純字串位址 {len(level0)} 個\n")

# 反向 BFS:targets = {addr: chain(offset list, 從 IMAGE 端起)}
targets={a:[] for a in level0}
found_paths=[]
for depth in range(1,DEPTH+1):
    if not targets: break
    tarr=np.array(sorted(targets),dtype='<u8')
    next_targets={}
    hits_this=0
    for base,ty,v in views:
        # 對每個值 v[i],找最小 t>=v[i] 且 t-v[i]<=MAXOFF
        idx=np.searchsorted(tarr,v,side='left')
        ok=idx<len(tarr)
        if not ok.any(): continue
        oi=np.where(ok)[0]
        tt=tarr[idx[oi]]
        vv=v[oi]
        diff=tt-vv
        good=diff<=MAXOFF
        for j in np.where(good)[0]:
            i=oi[j]; storage=base+i*8; t=int(tt[j]); off=int(diff[j])
            chain=[off]+targets[t]
            if ty==MEM_IMAGE:
                found_paths.append((storage, chain))
            else:
                # 取物件起點附近當下一層 target(storage 本身)
                if storage not in next_targets or len(chain)<len(next_targets[storage]):
                    next_targets[storage]=chain
            hits_this+=1
    print(f"depth {depth}: 命中 {hits_this},下一層 {len(next_targets)} 個,已達 IMAGE 路徑 {len(found_paths)}")
    if found_paths:
        break
    # 限制分支
    if len(next_targets)>3000:
        next_targets=dict(list(next_targets.items())[:3000])
    targets=next_targets

print(f"\n=== 找到 {len(found_paths)} 條 IMAGE→字串 路徑 ===")
for storage,chain in found_paths[:10]:
    rva=storage-exe_base
    print(f"  [模組基址+{rva:#x}] " + " -> ".join(f"+{o:#x}" for o in chain) + f"  (儲存@{storage:#x})")
