import frida, sys, io, time, re
sys.stdout = io.TextIOWrapper(sys.stdout.buffer, encoding="utf-8", errors="replace")
session = frida.attach("WizardGraphicalClient.exe")
# 追「含 Art_Chat 的聊天行」被哪個函式處理:hook 全記憶體掃描找到的聊天行位址的建立者太難,
# 改用 Frida 掃描 + hook。這裡改策略:hook 大量候選函式不可行,改用「找誰讀含 Art_Chat 的字串」。
# 最實際:hook exe 內部字串建構(wstring assign)不易。改用 backtrace-from-known:
# 先讓別人訊息出現,純讀找到聊天行 markup 位址,Frida MemoryAccessMonitor 監控它的建立。
# 簡化:直接 hook ControlRichEdit 全部 method + 過濾 Art_Chat,但 ControlRichEdit 非聊天框。
# 真正做法:掃所有含聊天訊息的 window 物件的 vtable,找收 Art_Chat 的。太廣。
# 折衷探索:hook 常見文字入口 sprintf 家族的其他變體 + 過濾 Art_Chat。
js = """
var base = Process.getModuleByName("WizardGraphicalClient.exe").base;
// 掃描 .rdata 找 "Art_Chat_Say.dds" 格式字串(RVA 0x2a84cf0 是其中一個),
// 但聊天行組裝可能用 wstring。改用: hook memcpy 家族但極輕量(只比對前 8 bytes,不整段讀)。
var needle = [0x3c,0x00,0x63,0x00,0x6f,0x00,0x6c,0x00]; // "<col" UTF-16LE
function head8(p){ try{ return p.readByteArray(16); }catch(e){ return null; } }
function isChat(p){
  try{
    var b = new Uint8Array(p.readByteArray(16));
    // 比對 "<col"
    if(b[0]===0x3c&&b[2]===0x63&&b[4]===0x6f&&b[6]===0x6c) return true;
  }catch(e){}
  return false;
}
var seenbt = {};
var f = Module.getExportByName("ucrtbase.dll","memcpy");
Interceptor.attach(f, {
  onEnter: function(args){
    var n = args[2].toInt32();
    if(n < 40 || n > 1200) return;          // 聊天行長度範圍
    if(!isChat(args[1])) return;            // src 以 "<col" 開頭
    var s; try{ s = args[1].readUtf16String(200); }catch(e){ return; }
    if(!s || s.indexOf("Art_Chat") < 0) return;   // 確認聊天行
    var bt = Thread.backtrace(this.context, Backtracer.ACCURATE).map(function(a){return a.sub(base);});
    var key = bt.slice(0,4).join(",");
    if(seenbt[key]) return; seenbt[key]=1;
    send({txt: s.substring(0,55), bt: bt.slice(0,10).map(function(a){return a.toString();})});
  }
});
send({status:"hook memcpy + 極輕過濾(前8byte比對 <col),等聊天行"});
"""
script = session.create_script(js)
def on_msg(msg, data):
    if msg.get('type')=='error': print("[JS錯誤]", msg.get('description')); return
    p = msg.get('payload', {})
    if 'status' in p: print("[frida]", p['status'], flush=True)
    elif 'bt' in p:
        clean = re.sub(r"<[^>]*>","",p['txt']).strip()
        print(f"\n★ 聊天行 {clean[:40]!r}", flush=True)
        print("  堆疊(RVA,上游在下): " + " ".join(p['bt']), flush=True)
script.on('message', on_msg)
script.load(); print("[py] 監聽中,你打好+等別人說話,說一聲", flush=True)
time.sleep(600)
session.detach(); print("done", flush=True)
