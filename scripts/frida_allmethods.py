import frida, sys, io, time, re
sys.stdout = io.TextIOWrapper(sys.stdout.buffer, encoding="utf-8", errors="replace")
session = frida.attach("WizardGraphicalClient.exe")
js = """
var base = Process.getModuleByName("WizardGraphicalClient.exe").base;
var vt = base.add(0x33b0fc0).readPointer();     // ControlRichEdit vtable(此類所有實例共用)
function bufOf(p){
  var c; try { c = p.readU16(); } catch(e){ return null; }
  if(c === 0x3c) return p;
  try { var q = p.readPointer(); if(q.readU16() === 0x3c) return q; } catch(e){}
  return null;
}
var seen = {};
for(var i=0;i<90;i++){
  var fn; try { fn = vt.add(i*8).readPointer(); } catch(e){ break; }
  if(fn.compare(base)<0 || fn.compare(base.add(0x2951000))>=0) continue;
  (function(idx, fnp){
    try { Interceptor.attach(fnp, { onEnter: function(args){
      for(var k=1;k<=4;k++){
        var b = bufOf(args[k]); if(!b) continue;
        var s; try { s = b.readUtf16String(300); } catch(e){ continue; }
        if(!s || s.indexOf("Art_Chat") < 0) continue;   // 只要含 Art_Chat 的正式聊天行
        var key = "m"+idx+"a"+k;
        if(seen[key]) return; seen[key]=1;
        send({m:idx, a:k, rva: fnp.sub(base).toString(), txt: s.substring(0,70)});
        return;
      }
    }}); } catch(e){}
  })(i, fn);
}
send({status:"hook 全 method + 過濾 Art_Chat, vtable @ "+vt.sub(base)});
"""
script = session.create_script(js)
def on_msg(msg, data):
    if msg.get('type')=='error': print("[JS錯誤]", msg.get('description')); return
    p = msg.get('payload', {})
    if 'status' in p: print("[frida]", p['status'], flush=True)
    elif 'm' in p:
        clean = re.sub(r"<[^>]*>","",p['txt']).strip()
        print(f"\n★ method[{p['m']}] (RVA {p['rva']}) arg{p['a']} = 聊天行: {clean[:45]!r}", flush=True)
script.on('message', on_msg)
script.load(); print("[py] 監聽中,等別人聊天,說一聲", flush=True)
time.sleep(600)
session.detach(); print("done", flush=True)
