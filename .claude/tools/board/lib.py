"""Shared helpers for the "Una roadmap" project board (org una-lang, project 1)."""

import json
import os
import subprocess
import time

# Same constants and helpers as create.py, which runs on import and so is not imported.
ORG = "una-lang"
PROJECT = "PVT_kwDOE__3nc4BlRCK"
STATUS_FIELD = "PVTSSF_lADOE__3nc4BlRCKzhj-jvk"
STATUS = {"Backlog": "3fdb8f71", "Needs spec": "9dd56126", "Ready": "46b5ba35",
          "In progress": "e35a0558", "In review": "3d8c549d", "Done": "6b163203"}
PRIO_FIELD = "PVTSSF_lADOE__3nc4BlRCKzhj-jyE"
PRIO = {"P0": "5ecfaa4b", "P1": "73a84e80", "P2": "207414b9"}
MILESTONE = {("una", "v0.1.0"): 1, ("una", "v0.1.x"): 2, ("una", "v0.2"): 3, ("una", "v0.3"): 4,
             ("una", "v0.4"): 5, ("una", "v1"): 6, ("una-bench", "Gate 1"): 1}


def gh(args, payload=None):
    out = subprocess.run(["gh", *args], input=json.dumps(payload) if payload else None,
                         capture_output=True, text=True)
    if out.returncode:
        raise SystemExit(f"gh {' '.join(args)} failed: {out.stderr}")
    return json.loads(out.stdout) if out.stdout.strip() else None


def graphql(query, **v):
    args = ["api", "graphql", "-f", f"query={query}"]
    for k, val in v.items():
        args += ["-f", f"{k}={val}"]
    return gh(args)


def set_select(item, field, option):
    graphql("mutation($p:ID!,$i:ID!,$f:ID!,$o:String!){updateProjectV2ItemFieldValue(input:"
            "{projectId:$p,itemId:$i,fieldId:$f,value:{singleSelectOptionId:$o}}){projectV2Item{id}}}",
            p=PROJECT, i=item, f=field, o=option)

AGENT_FIELD = "PVTF_lADOE__3nc4BlRCKzhj-j6o"

def set_agent(item, text):
    graphql("mutation($p:ID!,$i:ID!,$f:ID!,$t:String!){updateProjectV2ItemFieldValue(input:"
            "{projectId:$p,itemId:$i,fieldId:$f,value:{text:$t}}){projectV2Item{id}}}",
            p=PROJECT, i=item, f=AGENT_FIELD, t=text)



def items():
    out = {}
    q = ("query($c:String){organization(login:\"una-lang\"){projectV2(number:1){items(first:100,after:$c){"
         "pageInfo{hasNextPage endCursor} nodes{id content{... on Issue{number repository{name}}}}}}}}")
    cursor = None
    while True:
        args = ["api", "graphql", "-f", f"query={q}"] + (["-f", f"c={cursor}"] if cursor else [])
        page = gh(args)["data"]["organization"]["projectV2"]["items"]
        for n in page["nodes"]:
            c = n["content"]
            if c:
                out[(c["repository"]["name"], c["number"])] = n["id"]
        if not page["pageInfo"]["hasNextPage"]:
            return out
        cursor = page["pageInfo"]["endCursor"]



