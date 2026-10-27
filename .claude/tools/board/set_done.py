"""Set board Status Done for merged una issues: python3 set_done.py N [N...]."""
import os
import sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from lib import set_select, STATUS_FIELD, STATUS, items

ids = items()
for n in map(int, sys.argv[1:]):
    set_select(ids[("una", n)], STATUS_FIELD, STATUS["Done"])
    print("done una#%d" % n)
