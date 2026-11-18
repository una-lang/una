import json,sys
ev=json.load(open(sys.argv[1]))
bad=[]
bytask={}
for e in ev:
    k=e.get("outcome",{}).get("kind")
    if k=="aborted": bad.append(("aborted",e))
    bytask.setdefault(e.get("task",""),[]).append(e)
canc=0
for t,es in bytask.items():
    for i,e in enumerate(es):
        if e["outcome"]["kind"]=="cancelled":
            canc+=1
            if i!=len(es)-1: bad.append(("cancelled not last",t))
            if "op" in e: bad.append(("cancelled names op",e))
    # descendants of a cancelled task must not have events after? (order is by task index; skip)
print(json.dumps({"events":len(ev),"cancelled":canc,"bad":bad[:3],"nbad":len(bad)}))
