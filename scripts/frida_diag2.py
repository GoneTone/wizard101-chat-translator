import frida, sys, io, time, re
sys.stdout = io.TextIOWrapper(sys.stdout.buffer, encoding="utf-8", errors="replace")
session = frida.attach("WizardGraphicalClient.exe")
js = """
var base = Process.getModuleByName("WizardGraphicalClient.exe").base;
var fn = base.add(0x24dcf0);
var counts = {};
Interceptor.attach(fn, {
  onEnter: function(args){
    var p = args[3];
    var c; try { c = p.readU16(); } catch(e){ return; }
    if(c !== 0x3c) return;
    var s; try { s = p.readUtf16String(300); } catch(e){ return; }
    if(!s) return;
    counts[s] = (counts[s]||0) + 1;
  }
});
setTimeout(function(){
  var arr = Object.keys(counts).map(function(k){return [k,counts[k]];});
  arr.sort(function(a,b){return b[1]-a[1];});
  send({top: arr.slice(0,40)});
}, 50000);
send({status:"hooked, 收集所有 < 開頭 markup"});
"""
script = session.create_script(js)
def on_msg(msg, data):
    if msg.get('type')=='error': print("[JS錯誤]", msg.get('description')); return
    p = msg.get('payload', {})
    if 'status' in p: print("[frida]", p['status'], flush=True)
    elif 'top' in p:
        print(f"\n=== 所有 < 開頭 markup(去重, 次數) 共 {len(p['top'])} 種 ===", flush=True)
        for s,cnt in p['top']:
            clean = re.sub(r"<[^>]*>","",s).strip()
            print(f"  x{cnt:<4} clean={clean[:35]!r}  raw={s[:55]!r}", flush=True)
script.on('message', on_msg)
script.load(); print("[py] loaded 55s", flush=True)
time.sleep(55)
session.detach(); print("done", flush=True)
