import frida, sys, io, time, re
sys.stdout = io.TextIOWrapper(sys.stdout.buffer, encoding="utf-8", errors="replace")
session = frida.attach("WizardGraphicalClient.exe")
js = """
var base = Process.getModuleByName("WizardGraphicalClient.exe").base;
function readStr(p){          // 試讀 wstring/物件裡的可讀文字
  if(p.isNull()) return null;
  try{ var s=p.readUtf16String(60); if(s && /[A-Za-z\u4e00-\u9fff]{2,}/.test(s)) return "W:"+s; }catch(e){}
  try{ var s=p.readAnsiString(60); if(s && /[A-Za-z]{2,}/.test(s)) return "A:"+s; }catch(e){}
  try{ var q=p.readPointer(); var s=q.readUtf16String(60); if(s && /[A-Za-z\u4e00-\u9fff]{2,}/.test(s)) return "W*:"+s; }catch(e){}
  return null;
}
function dumpObj(rcx){         // 掃 this 前 0x60 找欄位裡的字串
  var out=[];
  for(var o=0;o<0x60;o+=8){
    try{ var s=readStr(rcx.add(o)); if(s) out.push("+"+o.toString(16)+" "+s.substring(0,40)); }catch(e){}
  }
  return out;
}
var seen={};
[["A",0x1707da0],["B",0x1708b60]].forEach(function(pair){
  var name=pair[0], fn=base.add(pair[1]);
  Interceptor.attach(fn, {
    onEnter: function(args){
      var fields = dumpObj(this.context.rcx);
      var rdx = readStr(this.context.rdx);
      var info = fields.join(" | ") + (rdx?"  rdx="+rdx:"");
      if(!/[A-Za-z\u4e00-\u9fff]{3,}/.test(info)) return;
      var key = name+":"+info.substring(0,30);
      if(seen[key]) return; seen[key]=1;
      send({acc:name, info: info.substring(0,140)});
    }
  });
});
send({status:"hook accessor A/B"});
"""
script = session.create_script(js)
def on_msg(msg, data):
    if msg.get('type')=='error': print("[JS錯誤]", msg.get('description')); return
    p = msg.get('payload', {})
    if 'status' in p: print("[frida]", p['status'], flush=True)
    elif 'acc' in p: print(f"  [accessor {p['acc']}] {p['info']}", flush=True)
script.on('message', on_msg)
script.load(); print("[py] 監聽中,發訊息+等別人說話,說一聲", flush=True)
time.sleep(600)
session.detach(); print("done", flush=True)
