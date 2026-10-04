#!/bin/bash
# usage: cmp.sh <case-dir> <name> [args...]: builds once, runs interp and native in fresh copies
set -u
R=$(CDPATH='' cd -- "$(dirname -- "$0")/../../../../../.." && pwd)
S=${UNA_REVIEW_SCRATCH:-$(mktemp -d)}
export PATH="$R/target/z3/bin:$PATH"
CASE=${1:?}; NAME=${2:?}; shift 2
B=$S/build-$NAME
rm -rf "${B:?}"; cp -R "$CASE" "$B"
(cd "$B" && $R/target/debug/una build . >/dev/null 2>$S/$NAME.build.err) || { echo BUILD FAILED; cat $S/$NAME.build.err; exit 1; }
for mode in interp native; do
  W=$S/run-$NAME-$mode
  rm -rf "${W:?}"; cp -R "$CASE" "$W"
  [ -n "${SETUP:-}" ] && $SETUP "$W"
  cd "$W"
  if [ $mode = interp ]; then
    $R/target/debug/una run . "$@" >$S/$NAME.$mode.out 2>$S/$NAME.$mode.err; echo $? > $S/$NAME.$mode.rc
    $R/target/debug/una --json run . "$@" > $S/$NAME.$mode.json 2>/dev/null
  else
    $B/target/una/$(grep '^name' una.toml | sed -E 's/.*"(.*)"/\1/') --trace $S/$NAME.native.trace "$@" >$S/$NAME.$mode.out 2>$S/$NAME.$mode.err; echo $? > $S/$NAME.$mode.rc
  fi
  cd /
done
echo "rc interp=$(cat $S/$NAME.interp.rc) native=$(cat $S/$NAME.native.rc)"
diff $S/$NAME.interp.out $S/$NAME.native.out > /dev/null && echo "stdout SAME" || { echo "stdout DIFF"; diff $S/$NAME.interp.out $S/$NAME.native.out; }
diff $S/$NAME.interp.err $S/$NAME.native.err > /dev/null && echo "stderr SAME" || { echo "stderr DIFF"; diff $S/$NAME.interp.err $S/$NAME.native.err; }
python3 - "$S/$NAME.interp.json" "$S/$NAME.native.trace" <<'PY'
import json,sys
try:
  a=json.load(open(sys.argv[1]))["trace"]
except Exception as e:
  print("no interp trace", e); sys.exit()
try:
  b=json.load(open(sys.argv[2]))
except Exception as e:
  print("no native trace", e); sys.exit()
if a==b: print("trace SAME (%d events)"%len(a))
else:
  print("trace DIFF")
  for x,y in zip(a,b):
    if x!=y: print(" I:",json.dumps(x)); print(" N:",json.dumps(y))
  if len(a)!=len(b): print(" lengths",len(a),len(b))
PY
