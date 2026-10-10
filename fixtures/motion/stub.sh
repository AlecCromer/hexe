#!/bin/sh
# Fake `python` for the sop-motion tests: called as `stub.sh -m gaia_motion.cli run --scene S --frames N --out DIR ...`.
# Writes DIR/geo/frame.%04d.geo (N tiny 2-point .geo files, point 0 = (idx,0,0), point 1 = (idx,1,0)) + DIR/argv.txt.
# `--push-body=FAIL`-style failure: body FAIL -> stderr message + exit 3 (nothing written).
out=""; frames=3; prev=""; body=""
for a in "$@"; do
  case "$prev" in
    --out) out="$a" ;;
    --frames) frames="$a" ;;
    --push-body) body="$a" ;;
  esac
  prev="$a"
done
if [ "$body" = "FAIL" ]; then echo "stub: boom (push-body FAIL)" >&2; exit 3; fi
[ -n "$out" ] || { echo "stub: --out missing" >&2; exit 2; }
mkdir -p "$out/geo" || exit 2
printf '%s\n' "$@" > "$out/argv.txt"
i=1
while [ "$i" -le "$frames" ]; do
  x=$((i - 1))
  f=$(printf '%s/geo/frame.%04d.geo' "$out" "$i")
  printf '%s' '["fileversion","20.5.487","hasindex",false,"pointcount",2,"vertexcount",0,"primitivecount",0,"info",{"software":"stub"},"topology",["pointref",["indices",[]]],"attributes",["pointattributes",[[["scope","public","type","numeric","name","P","options",{"type":{"type":"string","value":"point"}}],["size",4,"storage","fpreal32","values",["size",4,"storage","fpreal32","tuples",[['"$x"',0.0,0.0,1],['"$x"',1.0,0.0,1]]]]]],"vertexattributes",[],"primitiveattributes",[],"globalattributes",[]],"primitives",[]]' > "$f"
  i=$((i + 1))
done
echo '{"scene":"stub","frames":'"$frames"'}'
