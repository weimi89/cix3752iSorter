"""看廠商 Go 執行檔（main_proj/ecs1000）某個函式在做什麼：把 go tool objdump 的輸出加上字串註解——
LEAQ 到唯讀資料的位址時印出該處的文字，log 訊息、URL、指令字串就看得出流程；只印呼叫、跳轉與有註解的行。
正式機執行檔比交付的原始碼新，原始碼看不到的功能（GetChuteFromApi、PushHardwareAlarmToNode、~I 建件…）靠這個確認。
需要 Go 工具鏈（brew install go）。
用法：python3 scripts/legacy-disasm.py ../main_proj/ecs1000 'logic.GetChuteFromApi$' [最多行數]
列函式：go tool nm -size ../main_proj/ecs1000 | grep main_proj"""
import re, struct, subprocess, sys
elf = sys.argv[1]; pat = sys.argv[2]; maxlines = int(sys.argv[3]) if len(sys.argv) > 3 else 400
data = open(elf, 'rb').read()
# ELF64 program headers → (vaddr, offset, filesz)
phoff = struct.unpack_from('<Q', data, 0x20)[0]; phentsize, phnum = struct.unpack_from('<HH', data, 0x36)
segs = []
for i in range(phnum):
    p_type, p_flags, p_offset, p_vaddr, _, p_filesz = struct.unpack_from('<IIQQQQ', data, phoff + i * phentsize)
    if p_type == 1: segs.append((p_vaddr, p_offset, p_filesz))
def read_at(va, n=96):
    for v, o, sz in segs:
        if v <= va < v + sz:
            b = data[o + (va - v): o + (va - v) + n]
            s = b.decode('utf-8', 'replace')
            s = ''.join(ch if (ch.isprintable() and ch != '�') else '·' for ch in s)
            return s
    return None
out = subprocess.run(['go', 'tool', 'objdump', '-s', pat, elf], capture_output=True, text=True).stdout
lines = out.splitlines()
n = 0
for i, l in enumerate(lines):
    if l.startswith('TEXT '): print('\n' + l); continue
    m = re.search(r'LEAQ 0x([0-9a-f]+)\(IP\), (\w+)', l)
    ann = ''
    if m:
        # 目標 = 下一條指令位址 + 位移；objdump 已算好絕對位址在 0x... 形式？Go objdump 印相對位址，需自算
        off = int(m.group(1), 16)
        pc_m = re.match(r'\s*\S+\s+0x([0-9a-f]+)\s+([0-9a-f]+)', l)
        if pc_m:
            pc = int(pc_m.group(1), 16); ilen = len(pc_m.group(2)) // 2
            target = pc + ilen + off
            # 長度常在接下來幾行 MOVL $n, reg
            ln = None
            for j in range(i + 1, min(i + 4, len(lines))):
                mm = re.search(r'MOV[LQ] \$0x([0-9a-f]+), (\w+)', lines[j])
                if mm: ln = int(mm.group(1), 16); break
            s = read_at(target, ln if (ln and ln < 200) else 80)
            if s and s.strip('·'): ann = f'   ; "{s}"'
    # 只印有意義的行：呼叫、跳轉、有註解的、比較
    if ann or re.search(r'\bCALL\b|\bJ[A-Z]+\b|CMP', l):
        print(re.sub(r'^\s*\S+\s+0x[0-9a-f]+\s+[0-9a-f]+\s+', '  ', l) + ann)
        n += 1
        if n >= maxlines: print('  …（截斷）'); break
