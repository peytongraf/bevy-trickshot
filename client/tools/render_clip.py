# blender -b -P render.py -- <glb> <action name> <time s> <out.png> [view: side|front|top|three]
import bpy, sys, math, mathutils
argv = sys.argv[sys.argv.index('--')+1:]
glb, action_name, t, out = argv[0], argv[1], float(argv[2]), argv[3]
views = argv[4].split(',') if len(argv) > 4 else ['three']
bpy.ops.wm.read_factory_settings(use_empty=True)
bpy.ops.import_scene.gltf(filepath=glb)
# (The game draws the soldier 21x its file size.)
for o in list(bpy.data.objects):
    if o.parent is None:
        o.scale = (o.scale[0] * 21, o.scale[1] * 21, o.scale[2] * 21)
arm = next(o for o in bpy.data.objects if o.type == 'ARMATURE')
act = bpy.data.actions.get(action_name) or next((a for a in bpy.data.actions if a.name.startswith(action_name)), None)
assert act, [a.name for a in bpy.data.actions]
arm.animation_data_create(); arm.animation_data.action = act
# Blender NLA may already have tracks from the importer: mute them
for tr in arm.animation_data.nla_tracks: tr.mute = True
scene = bpy.context.scene
fps = scene.render.fps
scene.frame_set(int(round(t * fps)))
# floor
bpy.ops.mesh.primitive_plane_add(size=6, location=(0, 0, 0))
floor = bpy.context.object
mat = bpy.data.materials.new('floor'); mat.diffuse_color = (0.35, 0.37, 0.4, 1); floor.data.materials.append(mat)
# grid lines every 0.5 m
for i in range(-6, 7):
    for along in (0, 1):
        bpy.ops.mesh.primitive_cube_add(size=1, location=((i*0.5, 0, 0.001) if along == 0 else (0, i*0.5, 0.001)))
        c = bpy.context.object; c.scale = ((0.005, 3, 0.001) if along == 0 else (3, 0.005, 0.001))
        m = bpy.data.materials.new('g'); m.diffuse_color = (0.15, 0.15, 0.15, 1); c.data.materials.append(m)
light = bpy.data.lights.new('sun', 'SUN'); light.energy = 3
lo = bpy.data.objects.new('sun', light); scene.collection.objects.link(lo); lo.rotation_euler = (math.radians(40), 0, math.radians(30))
scene.render.engine = 'BLENDER_WORKBENCH'
scene.display.shading.light = 'STUDIO'; scene.display.shading.color_type = 'MATERIAL'
scene.render.resolution_x = 640; scene.render.resolution_y = 480
cam_data = bpy.data.cameras.new('cam'); cam_data.lens = 35
cam = bpy.data.objects.new('cam', cam_data); scene.collection.objects.link(cam); scene.camera = cam
# Blender is Z-up: glTF +Y up -> Blender +Z; glTF +Z (model forward) -> Blender -Y
target = mathutils.Vector((0, 0.5, 0.25))
spots = {
    'side': mathutils.Vector((3.2, 0, 0.9)),     # from the model's left (+X in Blender? glTF -X is right -> Blender -X right)
    'front': mathutils.Vector((0, -3.2, 0.35)),
    'low': mathutils.Vector((3.0, 0.4, 0.12)),
    'top': mathutils.Vector((0.01, -0.3, 4.0)),
    'three': mathutils.Vector((2.4, -2.4, 1.6)),
}
import os
base, ext = os.path.splitext(out)
for v in views:
    cam.location = spots[v]
    d = target - cam.location
    cam.rotation_euler = d.to_track_quat('-Z', 'Y').to_euler()
    scene.render.filepath = f"{base}_{v}{ext}" if len(views) > 1 else out
    bpy.ops.render.render(write_still=True)
