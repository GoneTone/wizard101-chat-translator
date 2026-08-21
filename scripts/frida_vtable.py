import frida, sys, io, time
sys.stdout = io.TextIOWrapper(sys.stdout.buffer, encoding="utf-8", errors="replace")
NEEDLE = sys.argv[1] if len(sys.argv)>1 else "VTHK9"
session = frida.attach("WizardGraphicalClient.exe")
js = """
var base = Process.getModuleByName("WizardGraphicalClient.exe").base;
var NEEDLE = 'NEEDLE_PH';
var obj = base.add(0x33b0fc0);          // ControlRichEdit 單例
var vt = obj.readPointer();
send({status: "ControlRichEdit vtable @ " + vt.sub(base)});
var seen = {};
function tryStr(p){
  try {
    // p 可能是 wchar*,或 wstring 物件(+0 是 buf/ptr)
    var s = p.readUtf16String(80);
    if(s && s.indexOf(NEEDLE)>=0) return s;
    var s2 = p.readPointer().readUtf16String(80);
    if(s2 && s2.indexOf(NEEDLE)>=0) return s2;
  } catch(e){}
  return null;
}
for(var i=0; i<80; i++){
  var fn;
  try { fn = vt.add(i*8).readPointer(); } catch(e){ break; }
  if(fn.compare(base)<0 || fn.compare(base.add(0x2951000))>=0) continue;
  (function(idx, fnp){
    try {
      Interceptor.attach(fnp, {
        onEnter: function(args){
          for(var k=1;k<=4;k++){
            var s = tryStr(args[k]);
            if(s){
              var bt = Thread.backtrace(this.context, Backtracer.ACCURATE)
                        .map(function(a){return a.sub(base).toString();});
              var key = idx+":"+k;
              if(seen[key]) return; seen[key]=1;
              send({method: idx, arg: k, rva: fnp.sub(base).toString(),
                    txt: s.substring(0,70), bt: bt.slice(0,8)});
              return;
            }
          }
        }
      });
    } catch(e){}
  })(i, fn);
}
send({status: "已 hook vtable 方法, needle=" + NEEDLE});
""".replace("NEEDLE_PH", NEEDLE)
script = session.create_script(js)
def on_msg(msg, data):
    if msg.get('type')=='error': print("[JS錯誤]", msg.get('description')); return
    p = msg.get('payload', {})
    if 'status' in p: print("[frida]", p['status'], flush=True)
    elif 'method' in p:
        print(f"\n★ vtable方法[{p['method']}] (RVA {p['rva']}) 的 arg{p['arg']} 含訊息: {p['txt']!r}", flush=True)
        print("  堆疊:", " ".join(p['bt']), flush=True)
script.on('message', on_msg)
script.load()
print("[py] loaded, 等 80s", flush=True)
time.sleep(80)
session.detach(); print("done", flush=True)
