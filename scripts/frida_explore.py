import frida, sys, io, time
sys.stdout = io.TextIOWrapper(sys.stdout.buffer, encoding="utf-8", errors="replace")
session = frida.attach("WizardGraphicalClient.exe")
js = """
var m = Process.getModuleByName("WizardGraphicalClient.exe");
var base = m.base;
var sprintf = base.add(0x3dc5e0);
var seen = {};
Interceptor.attach(sprintf, {
  onEnter: function(args) {
    try {
      var fmt = args[1].readUtf16String();
      if (!fmt) return;
      // 只在含聊天標記特徵時記錄
      if (fmt.indexOf("color")>=0 || fmt.indexOf("link")>=0 || fmt.indexOf("image")>=0 ||
          fmt.indexOf("[")>=0 || fmt.indexOf("GID")>=0 || fmt.indexOf("center")>=0) {
        if (seen[fmt]) return; seen[fmt]=1;
        var bt = Thread.backtrace(this.context, Backtracer.ACCURATE)
          .map(function(a){ return a.sub(base).toString(); });
        send({fmt: fmt.substring(0,70), bt: bt.slice(0,12)});
      }
    } catch(e) {}
  }
});
send({status:"hooked, 放寬過濾(color/link/image/[/GID/center)"});
"""
script = session.create_script(js)
def on_msg(msg, data):
    p = msg.get('payload', {})
    if 'status' in p: print("[frida]", p['status'], flush=True)
    elif 'fmt' in p:
        print(f"\n格式字串: {p['fmt']!r}", flush=True)
        print("  堆疊(RVA): " + " ".join(p['bt'][:10]), flush=True)
script.on('message', on_msg)
script.load()
time.sleep(75)
session.detach(); print("done", flush=True)
