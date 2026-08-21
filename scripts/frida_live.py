import frida, sys, io, time, re
sys.stdout = io.TextIOWrapper(sys.stdout.buffer, encoding="utf-8", errors="replace")
session = frida.attach("WizardGraphicalClient.exe")
js = """
var base = Process.getModuleByName("WizardGraphicalClient.exe").base;
var fn = base.add(0x24dcf0);
var seen = {};
function bufOf(p){
  // args[3] 可能直接是 wchar*('<' 開頭),或 wstring 物件(*p 才是 buffer)
  var c; try { c = p.readU16(); } catch(e){ return null; }
  if(c === 0x3c) return p;
  try { var q = p.readPointer(); if(q.readU16() === 0x3c) return q; } catch(e){}
  return null;
}
Interceptor.attach(fn, {
  onEnter: function(args){
    var b = bufOf(args[3]);
    if(!b) return;
    var s; try { s = b.readUtf16String(300); } catch(e){ return; }
    if(!s) return;
    if(seen[s]) return; seen[s]=1;
    send({msg: s});
  }
});
send({status:"監聽中(已修:多解一層指標)"});
"""
script = session.create_script(js)
def on_msg(msg, data):
    if msg.get('type')=='error': print("[JS錯誤]", msg.get('description')); return
    p = msg.get('payload', {})
    if 'status' in p: print("[frida]", p['status'], flush=True)
    elif 'msg' in p:
        clean = re.sub(r"<[^>]*>","",p['msg']).strip()
        print(f"  clean={clean[:40]!r}  raw={p['msg'][:60]!r}", flush=True)
script.on('message', on_msg)
script.load(); print("[py] 監聽中,你打好說一聲", flush=True)
time.sleep(600)
session.detach(); print("done", flush=True)
