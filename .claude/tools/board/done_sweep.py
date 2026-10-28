"""Set Done on every board item whose issue is closed but whose Status is not Done. Usage: done_sweep.py [--dry]"""
import json
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from lib import STATUS, STATUS_FIELD, gh, set_select

dry = "--dry" in sys.argv
out = gh(["project", "item-list", "1", "--owner", "una-lang", "--limit", "900", "--format", "json"])
changed = 0
for item in out["items"]:
    content = item.get("content") or {}
    if content.get("state") == "CLOSED" and item.get("status") != "Done":
        print(content.get("repository", "?"), content.get("number"), item.get("status"), "->", "Done")
        if not dry:
            set_select(item["id"], STATUS_FIELD, STATUS["Done"])
        changed += 1
print("changed" if not dry else "would change", changed)
