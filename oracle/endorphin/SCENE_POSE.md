## Scene pose (h43, measured on Balcony.ens)

Where a scene character stands and how it is posed at frame 0 (`endorphin_geo`, `Scene::posed_locals`). Three layers, later wins, per joint:

1. **Rig rest**: each joint's own `<TransformMatrix>` (parent-local). T-pose-like; all four Balcony characters carry the same one.
2. **`<Poses><Pose character="cNN" refFrame="Parent">`**: one `<Relationship name="<joint>">` per posed joint holding the FULL parent-local matrix (rotation AND translation). `name="root"` is the skeleton root. Rig-only joints (`*VirtualMuscle`, props) are absent. Same data sits inside each `DesiredPoseEvent/Pose` (Balcony: Sitting for character02..04, max diff vs `<Poses>` 0.005).
3. **`<Animation>` channels** `cNN.Joints.<joint>.{quat,tx,ty,tz}` / `cNN.root.{quat,tx,ty,tz}` (FBX imports: `cNN.<joint>.*`): key `t` seconds (1/30 s spacing in the corpus), `type="linear"` is the only type (1513 quat + 1036 scalar). Balcony: ONE key at t=0 on character01/02 only (122 channels) => no keyed animation in this scene.

Quat key order on disk is `(bt, bx, by, bz, bw)` = time, **w, x, y, z** (attr names are misleading). Evidence: character01 `Joints.LeftAnkleJoint.quat` = (0.9721, 0.2043, -0.1155, -0.0026) as (w,x,y,z) gives R10 = 2(xy+wz) = -0.0523, R20 = 2(xz-wy) = 0.2226, matching the Pose matrix column `Vi` = (0.973, -0.0519, 0.2247) of the same joint (read as (x,y,z,w) it would be a 180 deg turn).

**Character placement** = the `<Character>`'s own child `<TransformMatrix>` (scene root transform, basis columns Vi Vj Vk + Vp), ignored before h43 (=> all characters stacked at origin). `world(joint) = Character.TransformMatrix x chain(local)`. Balcony Vp: character01 (0, 3.0, -0.3704) rotated 19 deg about Y; character02 (0.6737, 0, -3.7131) Ry; character03 (-0.9771, 0, -3.8064); character04 (-0.1106, 0, -4.8246). `<Environment>` (`background`) has none (identity).

Worked joint, character02 `root`: placement basis columns (0,0,1) (0,1,0) (-1,0,0), Vp (0.6737, 0, -3.7131); pose `root` Vp (0.0027, 0.5531, -0.4440) (== channel `character02.root.{tx,ty,tz}`). World = Vi*0.0027 + Vj*0.5531 + Vk*(-0.444) + Vp = (0.444, 0.5531, 0.0027) + (0.6737, 0, -3.7131) = **(1.1177, 0.5531, -3.7104)**. Test: `endorphin_tests::balcony_characters_are_separated_and_posed`.

Caveat: character01's `<Poses>` root (0.0301, 0.849, -0.2603) differs from its channel root (0, 0.8478, 0.0001); channel wins (timeline), and the two quats differ by < 1 deg. Pose application is kinematic only: `.nmb` behaviours are opaque, nothing is simulated (the DesiredPoseEvent frame range 0..64-73 is the sim's blend window, not evaluated).
