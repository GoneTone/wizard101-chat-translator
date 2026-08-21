import frida, sys, io, time
sys.stdout = io.TextIOWrapper(sys.stdout.buffer, encoding="utf-8", errors="replace")
session = frida.attach("WizardGraphicalClient.exe")
js = """
var base = Process.getModuleByName("WizardGraphicalClient.exe").base;
var fn = base.add(0x24dcf0);
var total=0, samples=[];
function grab(p){
  try{
    var s=p.readUtf16String(50);
    if(s && s.length>=3){ var a=0; for(var i=0;i<s.length;i++){var c=s.charCodeAt(i); if(c>=0x20&&c<0x7f)a++;} if(a>=3) return s; }
  }catch(e){}
  try{ return grab2(p.readPointer()); }catch(e){}
  return null;
}
function grab2(p){ try{var s=p.readUtf16String(50); if(s&&s.length>=3){var a=0;for(var i=0;i<s.length;i++){var c=s.charCodeAt(i);if(c>=0x20&&c<0x7f)a++;}if(a>=3)return s;}}catch(e){} return null;}
Interceptor.attach(fn, {
  onEnter: function(args){
    total++;
    if(samples.length<25){
      for(var k=1;k<=4;k++){
        var s=grab(args[k]);
        if(s){ var tag="arg"+k+": "+s; if(samples.indexOf(tag)<0){ samples.push(tag); } }
      }
    }
  }
});
setTimeout(function(){ send({total:total, samples:samples}); }, 45000);
send({status:"hooked, 45s 收集樣本"});
"""
script = session.create_script(js)
def on_msg(msg, data):
    if msg.get('type')=='error': print("[JS錯誤]", msg.get('description')); return
    p = msg.get('payload', {})
    if 'status' in p: print("[frida]", p['status'], flush=True)
    elif 'total' in p:
        print(f"\n總呼叫次數: {p['total']}", flush=True)
        print("arg 內容樣本:", flush=True)
        for s in p['samples']: print(f"    {s[:75]!r}", flush=True)
script.on('message', on_msg)
script.load(); print("[py] loaded", flush=True)
time.sleep(50)
session.detach(); print("done", flush=True)
