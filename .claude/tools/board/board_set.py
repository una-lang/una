"""Add an issue to the board if missing, then set fields. Usage: board_set.py repo#N status prio agent ("-" keeps a field)"""
import sys
import os
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from lib import gh, graphql, set_select, STATUS_FIELD, STATUS, PRIO_FIELD, PRIO, PROJECT, AGENT_FIELD
def set_agent(item, text):
    graphql("mutation($p:ID!,$i:ID!,$f:ID!,$t:String!){updateProjectV2ItemFieldValue(input:{projectId:$p,itemId:$i,fieldId:$f,value:{text:$t}}){projectV2Item{id}}}", p=PROJECT, i=item, f=AGENT_FIELD, t=text)
ref, status, prio, agent = sys.argv[1:5]
repo, n = ref.split("#")
out = gh(["api","graphql","-f",'query=query($r:String!,$n:Int!){repository(owner:"una-lang",name:$r){issue(number:$n){id title projectItems(first:5){nodes{id project{id}}}}}}',"-f",f"r={repo}","-F",f"n={n}"])
iss = out["data"]["repository"]["issue"]
item = next((x["id"] for x in iss["projectItems"]["nodes"] if x["project"]["id"] == PROJECT), None)
if not item:
    item = graphql("mutation($p:ID!,$c:ID!){addProjectV2ItemById(input:{projectId:$p,contentId:$c}){item{id}}}", p=PROJECT, c=iss["id"])["data"]["addProjectV2ItemById"]["item"]["id"]
if status != "-": set_select(item, STATUS_FIELD, STATUS[status])
if prio != "-": set_select(item, PRIO_FIELD, PRIO[prio])
if agent != "-": set_agent(item, agent)
print(ref, "|", iss["title"][:60], "|", status, prio, agent)
