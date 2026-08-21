import frida, sys, io, time, re
sys.stdout = io.TextIOWrapper(sys.stdout.buffer, encoding="utf-8", errors="replace")
session = frida.attach("WizardGraphicalClient.exe")
js = """
var base = Process.getModuleByName("WizardGraphicalClient.exe").base;
var fn = base.add(0x24dcf0);
Interceptor.attach(fn, {
  onEnter: function(args){
    var p = args[3];
    var c; try { c = p.readU16(); } catch(e){ return; }
    if(c !== 0x3c) return;                 // 只看 '<' 開頭(markup)
    var s; try { s = p.readUtf16String(400); } catch(e){ return; }
    if(!s || s.indexOf("[") < 0) return;   // 只要含 [發送者] 的正式聊天行
    send({msg: s});
  }
});
send({status:"hooked, 過濾含[發送者]的行"});
"""
script = session.create_script(js)
n=[0]
def on_msg(msg, data):
    if msg.get('type')=='error': print("[JS錯誤]", msg.get('description')); return
    p = msg.get('payload', {})
    if 'status' in p: print("[frida]", p['status'], flush=True)
    elif 'msg' in p:
        n[0]+=1
        clean = re.sub(r"<[^>]*>", "", p['msg']).strip()
        print(f"  #{n[0]} {clean[:50]!r}", flush=True)
script.on('message', on_msg)
script.load(); print("[py] loaded 55s", flush=True)
time.sleep(55)
session.detach(); print(f"done, 共 {n[0]} 則", flush=True)
