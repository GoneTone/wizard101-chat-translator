import frida, sys, io, time
sys.stdout = io.TextIOWrapper(sys.stdout.buffer, encoding="utf-8", errors="replace")
NEEDLE = sys.argv[1] if len(sys.argv)>1 else "WZQK83"
session = frida.attach("WizardGraphicalClient.exe")
js = """
var base = Process.getModuleByName("WizardGraphicalClient.exe").base;
var NEEDLE = 'NEEDLE_PH';
var seen = {};
var f = Module.getExportByName("ucrtbase.dll", "memmove");
send({status: "memmove @ " + f});
Interceptor.attach(f, {
  onEnter: function(args){
    var n = args[2].toInt32();
    if(n < 10 || n > 400) return;
    var s = null;
    try { s = args[1].readUtf16String(n/2); } catch(e){ return; }
    if(!s || s.indexOf(NEEDLE) < 0) return;
    var bt = Thread.backtrace(this.context, Backtracer.ACCURATE).map(function(a){return a.sub(base);});
    var key = bt.slice(0,3).join(",");
    if(seen[key]) return; seen[key]=1;
    send({txt: s.substring(0,60), bt: bt.slice(0,12).map(function(a){return a.toString();})});
  }
});
send({status: "hook 完成, needle=" + NEEDLE});
""".replace("NEEDLE_PH", NEEDLE)
script = session.create_script(js)
def on_msg(msg, data):
    if msg.get('type') == 'error':
        print("[JS錯誤]", msg.get('description')); print(msg.get('stack','')[:300]); return
    p = msg.get('payload', {})
    if 'status' in p: print("[frida]", p['status'], flush=True)
    elif 'bt' in p:
        print(f"\n★ 複製含訊息: {p['txt']!r}", flush=True)
        for r in p['bt']: print(f"    {r}", flush=True)
script.on('message', on_msg)
script.load()
print("[py] script loaded, 等 80s", flush=True)
time.sleep(80)
session.detach(); print("done", flush=True)
