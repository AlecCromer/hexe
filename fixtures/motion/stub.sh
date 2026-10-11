#!/bin/sh
# Fake `python` for the sop-motion tests.
# push : `stub.sh -m gaia_motion.cli run --scene S --frames N --out DIR ...` -> DIR/geo/frame.%04d.geo (N tiny 2-point .geo files,
#        point 0 = (idx,0,0), point 1 = (idx,1,0)) + DIR/argv.txt. `--push-body FAIL` -> stderr message + exit 3 (nothing written).
# behaviour: `stub.sh -m gaia_motion.cli behaviour --name N --out DIR --run-s D --fps F [--seed S]` -> same frames as chain (no --ns; `--name FAIL` -> exit 5).
# chain: `stub.sh -m gaia_motion.cli chain --out DIR --ns X --run-s D --fps F [--seed S]` -> floor(D*F)+1 frames of 24 named SMPL points
#        (point i = (i/10, y, frame-1)), y of Pelvis = (frame-1)/100; + DIR/argv.txt. `--ns 13` -> exit 4.
out=""; name=""; frames=3; prev=""; body=""; chain=0; runs=12; fps=30; ns=0
for a in "$@"; do
  case "$prev" in
    --out) out="$a" ;;
    --frames) frames="$a" ;;
    --push-body) body="$a" ;;
    --run-s) runs="$a" ;;
    --fps) fps="$a" ;;
    --ns) ns="$a" ;;
    --name) name="$a" ;;
  esac
  [ "$a" = chain ] && chain=1
  [ "$a" = behaviour ] && chain=1
  prev="$a"
done
if [ "$body" = "FAIL" ]; then echo "stub: boom (push-body FAIL)" >&2; exit 3; fi
if [ "$name" = FAIL ]; then echo "stub: behaviour boom (name FAIL)" >&2; exit 5; fi
if [ "$chain" = 1 ] && [ "$ns" = 13 ]; then echo "stub: chain boom (ns 13)" >&2; exit 4; fi
[ -n "$out" ] || { echo "stub: --out missing" >&2; exit 2; }
mkdir -p "$out/geo" || exit 2
printf '%s\n' "$@" > "$out/argv.txt"
if [ "$chain" = 1 ]; then
  frames=$(awk -v d="$runs" -v f="$fps" 'BEGIN { printf "%d", int(d * f) + 1 }')
  echo '{"stub":"chain"}' > "$out/chain.json"
fi
i=1
while [ "$i" -le "$frames" ]; do
  x=$((i - 1))
  f=$(printf '%s/geo/frame.%04d.geo' "$out" "$i")
  if [ "$chain" = 1 ]; then
    awk -v x="$x" 'BEGIN {
      split("Pelvis L_Hip L_Knee L_Ankle L_Toe R_Hip R_Knee R_Ankle R_Toe Torso Spine Chest Neck Head L_Thorax L_Shoulder L_Elbow L_Wrist L_Hand R_Thorax R_Shoulder R_Elbow R_Wrist R_Hand", n, " ")
      t = ""; s = ""; ix = ""
      for (k = 1; k <= 24; k++) {
        y = (k == 1) ? x / 100 : 0.5
        t = t (k > 1 ? "," : "") sprintf("[%g,%g,%d,1]", (k - 1) / 10, y, x)
        s = s (k > 1 ? "," : "") "\"" n[k] "\""
        ix = ix (k > 1 ? "," : "") "[" (k - 1) "]"
      }
      printf "[\"fileversion\",\"20.5.487\",\"hasindex\",false,\"pointcount\",24,\"vertexcount\",0,\"primitivecount\",0,\"info\",{\"software\":\"stub\"},\"topology\",[\"pointref\",[\"indices\",[]]],\"attributes\",[\"pointattributes\",[[[\"scope\",\"public\",\"type\",\"numeric\",\"name\",\"P\",\"options\",{\"type\":{\"type\":\"string\",\"value\":\"point\"}}],[\"size\",4,\"storage\",\"fpreal32\",\"values\",[\"size\",4,\"storage\",\"fpreal32\",\"tuples\",[%s]]]],[[\"scope\",\"public\",\"type\",\"string\",\"name\",\"name\",\"options\",{}],[\"size\",1,\"storage\",\"int32\",\"strings\",[%s],\"indices\",[\"size\",1,\"storage\",\"int32\",\"tuples\",[%s]]]]],\"vertexattributes\",[],\"primitiveattributes\",[],\"globalattributes\",[]],\"primitives\",[]]", t, s, ix
    }' > "$f"
  else
    printf '%s' '["fileversion","20.5.487","hasindex",false,"pointcount",2,"vertexcount",0,"primitivecount",0,"info",{"software":"stub"},"topology",["pointref",["indices",[]]],"attributes",["pointattributes",[[["scope","public","type","numeric","name","P","options",{"type":{"type":"string","value":"point"}}],["size",4,"storage","fpreal32","values",["size",4,"storage","fpreal32","tuples",[['"$x"',0.0,0.0,1],['"$x"',1.0,0.0,1]]]]]],"vertexattributes",[],"primitiveattributes",[],"globalattributes",[]],"primitives",[]]' > "$f"
  fi
  i=$((i + 1))
done
echo '{"scene":"stub","frames":'"$frames"'}'
