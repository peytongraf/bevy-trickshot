#!/usr/bin/env python3
"""Generate the remote soldier's prone clips and write them into soldier.glb.

    python3 client/tools/soldier_prone_clips.py

Appends three animations to `client/assets/models/characters/soldier.glb`,
after its own 18 (whose indices, nodes and meshes are left exactly as they
are — `avatars::soldier` looks them up by index and name):

* `prone` (18) — lying flat, propped on the elbows, rifle forward, a slow
  breath; looped.
* `proneCrawl` (19) — the crawl forward: each knee in turn drawn up and out,
  the hips swaying to it, the shoulders reaching forward in turn; one cycle
  `CRAWL_SECS` long, looped.
* `proneCrawlBack` (20) — the same, played backwards.
* `throw` (21) — an overhand throw, for the upper body (the game plays it
  over whatever the legs are doing): cocked back ready (held there while a
  throw's being cooked), the arm over the top to the release at
  `THROW_RELEASE_SECS` (the first-person throw's), and the follow-through.
* `drink` (22) — drinking a perk, for the upper body: the bottle drawn up
  from the right hip, its neck to the mouth, tipped up as it's drunk with
  the head back, and put back at the hip; `DRINK_SECS` long (the
  first-person drink's). The bottle's the game's own, in the hand where
  `DRINK_HOLD_*` (printed) says — the game's `WeaponGrips::drink_hold`.

Every pose is built off the model's own `shooting` clip (its aimed rifle)
by turning bones — whole-body forward onto the ground, the spine arched up
on the elbows, the shoulders and head brought back up level — so the hands
hold a rifle the way every other clip does. Re-running it replaces the three
(it drops any it wrote before). Needs numpy. Preview a clip with
`client/tools/render_clip.py`.
"""
import json
import math
import os
import struct
import sys

import numpy as np

SRC = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "assets", "models", "characters", "soldier.glb")
CLIP_NAMES = ("prone", "proneCrawl", "proneCrawlBack", "throw", "drink")

AV = 21.0
def qmat(q):
    x,y,z,w=q
    return np.array([[1-2*(y*y+z*z),2*(x*y-z*w),2*(x*z+y*w)],[2*(x*y+z*w),1-2*(x*x+z*z),2*(y*z-x*w)],[2*(x*z-y*w),2*(y*z+x*w),1-2*(x*x+y*y)]])
def mquat(R):
    t=np.trace(R)
    if t>0:
        s=math.sqrt(t+1)*2; w=0.25*s; x=(R[2,1]-R[1,2])/s; y=(R[0,2]-R[2,0])/s; z=(R[1,0]-R[0,1])/s
    elif R[0,0]>R[1,1] and R[0,0]>R[2,2]:
        s=math.sqrt(1+R[0,0]-R[1,1]-R[2,2])*2; w=(R[2,1]-R[1,2])/s; x=0.25*s; y=(R[0,1]+R[1,0])/s; z=(R[0,2]+R[2,0])/s
    elif R[1,1]>R[2,2]:
        s=math.sqrt(1+R[1,1]-R[0,0]-R[2,2])*2; w=(R[0,2]-R[2,0])/s; x=(R[0,1]+R[1,0])/s; y=0.25*s; z=(R[1,2]+R[2,1])/s
    else:
        s=math.sqrt(1+R[2,2]-R[0,0]-R[1,1])*2; w=(R[1,0]-R[0,1])/s; x=(R[0,2]+R[2,0])/s; y=(R[1,2]+R[2,1])/s; z=0.25*s
    q=np.array([x,y,z,w]); return q/np.linalg.norm(q)
def aa(axis, ang):
    axis=np.asarray(axis,float); axis=axis/np.linalg.norm(axis)
    K=np.array([[0,-axis[2],axis[1]],[axis[2],0,-axis[0]],[-axis[1],axis[0],0]])
    return np.eye(3)+math.sin(ang)*K+(1-math.cos(ang))*K@K
def arc(u,v):
    u=np.asarray(u,float); v=np.asarray(v,float); u=u/np.linalg.norm(u); v=v/np.linalg.norm(v)
    c=np.cross(u,v); s=np.linalg.norm(c); d=float(np.dot(u,v))
    if s<1e-9:
        if d>0: return np.eye(3)
        o=np.cross(u,[1,0,0]);
        if np.linalg.norm(o)<1e-6: o=np.cross(u,[0,1,0])
        return aa(o, math.pi)
    return aa(c, math.atan2(s,d))

class Glb:
    def __init__(self, path):
        b=open(path,'rb').read(); l=struct.unpack('<I',b[12:16])[0]
        self.json=json.loads(b[20:20+l]); bo=20+l; bl=struct.unpack('<I',b[bo:bo+4])[0]
        self.bin=bytearray(b[bo+8:bo+8+bl])
        g=self.json; self.nodes=g['nodes']
        self.idx={n.get('name'):i for i,n in enumerate(self.nodes)}
        self.parent={c:i for i,n in enumerate(self.nodes) for c in n.get('children',[])}
    def acc(self,i):
        g=self.json; a=g['accessors'][i]; bv=g['bufferViews'][a['bufferView']]
        n={'SCALAR':1,'VEC3':3,'VEC4':4,'MAT4':16}[a['type']]
        off=bv.get('byteOffset',0)+a.get('byteOffset',0); st=bv.get('byteStride',4*n)
        return np.array([struct.unpack_from('<'+'f'*n,self.bin,off+k*st) for k in range(a['count'])])

class Pose:
    """Local TRS of every node; FK in the scene root's frame."""
    def __init__(self, glb, clip=None, t=0.0):
        self.g=glb; n=glb.nodes
        self.T=[np.array(x.get('translation',[0,0,0]),float) for x in n]
        self.R=[qmat(x.get('rotation',[0,0,0,1])) for x in n]
        self.S=[np.array(x.get('scale',[1,1,1]),float) for x in n]
        self.M0={i:np.array(x['matrix']).reshape(4,4).T for i,x in enumerate(n) if 'matrix' in x}
        if clip is not None:
            an=glb.json['animations'][clip]
            for ch in an['channels']:
                s=an['samplers'][ch['sampler']]; tt=glb.acc(s['input'])[:,0]; v=glb.acc(s['output'])
                k=int(np.searchsorted(tt, t, side='right'))-1; k=max(0,min(k,len(v)-1))
                i=ch['target']['node']; p=ch['target']['path']
                if p=='rotation': self.R[i]=qmat(v[k])
                elif p=='translation': self.T[i]=v[k].copy()
                elif p=='scale': self.S[i]=v[k].copy()
    def copy(self):
        p=Pose.__new__(Pose); p.g=self.g; p.T=[x.copy() for x in self.T]; p.R=[x.copy() for x in self.R]; p.S=[x.copy() for x in self.S]; p.M0=self.M0; return p
    def local(self,i):
        if i in self.M0: return self.M0[i]
        M=np.eye(4); M[:3,:3]=self.R[i]*self.S[i]; M[:3,3]=self.T[i]; return M
    def glob(self,i):
        M=self.local(i); par=self.g.parent
        while i in par: i=par[i]; M=self.local(i)@M
        return M
    def pos(self,name):  # metres
        return self.glob(self.g.idx[name])[:3,3]*AV
    def rot(self,name):
        M=self.glob(self.g.idx[name])[:3,:3]; return M/np.linalg.norm(M[:,0])
    def set_global_rot(self,name,Rg):
        i=self.g.idx[name]; Rp=self.glob(self.g.parent[i])[:3,:3]; Rp=Rp/np.linalg.norm(Rp[:,0])
        self.R[i]=Rp.T@Rg
    def set_global_pos(self,name,p_m):
        i=self.g.idx[name]; Mp=self.glob(self.g.parent[i]); inv=np.linalg.inv(Mp)
        self.T[i]=(inv@np.append(np.asarray(p_m)/AV,1))[:3]

g = None  # the model (main)

RIGHT = np.array([-1.0, 0, 0]); UP = np.array([0, 1.0, 0]); FWD = np.array([0, 0, 1.0])
deg = math.radians

def turn_about(p, name, R_world):
    """Turn bone `name` (and everything on it) by world rotation R about its own joint."""
    p.set_global_rot(name, R_world @ p.rot(name))

def prone_pose(params):
    p = Pose(g, 4, 0.0)  # `shooting`, first frame: the aimed rifle
    # 1. Lie down: whole body forward 90° (head toward +Z, belly down).
    turn_about(p, 'Bind_Root_00_02', aa(RIGHT, -deg(90)))  # about the body's right, forward
    # 2. Chest up on the elbows: the spine bends back.
    for b, a in zip(['Bind_Spine_010_011', 'Bind_Spine1_011_012', 'Bind_Spine2_012_013'], params['spine']):
        turn_about(p, b, aa(RIGHT, deg(a)))
    # 3. Arms (and the rifle) and head back up level with the ground.
    lift = params['lift']
    for b in ['Bind_LeftShoulder_015_016', 'Bind_RightShoulder_034_035']:
        turn_about(p, b, aa(RIGHT, deg(lift)))
    turn_about(p, 'Bind_Neck_013_014', aa(RIGHT, deg(lift * 0.45)))
    turn_about(p, 'Bind_Head_014_015', aa(RIGHT, deg(lift * 0.55)))
    # 4. Legs a little apart.
    for b, s in [('Bind_LeftUpLeg_02_03', 1), ('Bind_RightUpLeg_01_07', -1)]:
        turn_about(p, b, aa(UP, deg(params['spread']) * s * -1))
    return p

# --- writing clips into the glb ----------------------------------------------------
KEYED = None
def keyed_channels():
    global KEYED
    if KEYED is None:
        an = g.json['animations'][0]
        KEYED = [(c['target']['node'], c['target']['path']) for c in an['channels'] if c['target']['path'] in ('rotation', 'translation')]
    return KEYED

def add_clip(name, times, poses, out_json, out_bin):
    """Append an animation keying every channel `idleWgun` keys, from `poses` at `times`."""
    def push(arr, type_, minmax=False):
        data = np.asarray(arr, dtype='<f4')
        while len(out_bin) % 4: out_bin.append(0)
        off = len(out_bin); out_bin.extend(data.tobytes())
        out_json['bufferViews'].append({'buffer': 0, 'byteOffset': off, 'byteLength': data.nbytes})
        acc = {'bufferView': len(out_json['bufferViews']) - 1, 'componentType': 5126, 'count': int(data.shape[0]), 'type': type_}
        if minmax:
            acc['min'] = [float(data.min())]; acc['max'] = [float(data.max())]
        out_json['accessors'].append(acc)
        return len(out_json['accessors']) - 1
    t_acc = push(np.array(times, float), 'SCALAR', True)
    samplers, channels = [], []
    for node, path in keyed_channels():
        if path == 'rotation':
            vals = []
            for p in poses:
                q = mquat(p.R[node])
                if vals and np.dot(q, vals[-1]) < 0: q = -q  # (shortest way round between keys)
                vals.append(q)
            a = push(vals, 'VEC4')
        else:
            a = push([p.T[node] for p in poses], 'VEC3')
        samplers.append({'input': t_acc, 'output': a, 'interpolation': 'LINEAR'})
        channels.append({'sampler': len(samplers) - 1, 'target': {'node': node, 'path': path}})
    out_json['animations'].append({'name': name, 'samplers': samplers, 'channels': channels})

def write_glb(path, j, b):
    while len(b) % 4: b.append(0)
    j['buffers'][0]['byteLength'] = len(b)
    js = json.dumps(j, separators=(',', ':')).encode()
    while len(js) % 4: js += b' '
    out = struct.pack('<III', 0x46546C67, 2, 12 + 8 + len(js) + 8 + len(b))
    out += struct.pack('<II', len(js), 0x4E4F534A) + js + struct.pack('<II', len(b), 0x004E4942) + bytes(b)
    open(path, 'wb').write(out)

def fresh():
    j = json.loads(json.dumps(g.json)); b = bytearray(g.bin)
    return j, b

def aim_bone(p, name, child, direction):
    """Turn bone `name` (shortest way) so it points from its joint toward `direction` (its child's way)."""
    now = p.pos(child) - p.pos(name)
    turn_about(p, name, arc(now, direction))

def lay_legs(p, spread_deg, toe_down_deg):
    for side, s in (('Left', 1.0), ('Right', -1.0)):
        up = {'Left': 'Bind_LeftUpLeg_02_03', 'Right': 'Bind_RightUpLeg_01_07'}[side]
        leg = {'Left': 'Bind_LeftLeg_04_04', 'Right': 'Bind_RightLeg_07_08'}[side]
        foot = {'Left': 'Bind_LeftFoot_05_05', 'Right': 'Bind_RightFoot_08_09'}[side]
        toe = {'Left': 'Bind_LeftToeBase_06_06', 'Right': 'Bind_RightToeBase_09_010'}[side]
        out = -RIGHT * s  # left leg toward the body's left
        a = deg(spread_deg)
        thigh = -FWD * math.cos(a) + out * math.sin(a) + UP * -0.05
        aim_bone(p, up, leg, thigh)
        aim_bone(p, leg, foot, thigh + UP * 0.02)
        # toes dug in: the foot points down and a bit back
        t = deg(toe_down_deg)
        aim_bone(p, foot, toe, -UP * math.cos(t) - FWD * math.sin(t))

def place_hips(p, hips_y, head_z):
    hips = p.pos('Bind_Hips_03_00'); head = p.pos('Bind_Head_014_015'); root = p.pos('Bind_Root_00_02')
    p.set_global_pos('Bind_Root_00_02', root + np.array([0, hips_y - hips[1], head_z - head[2]]))

BASE = dict(spine=[8, 8, 9], lift=65, spread=8)
HIPS_Y, HEAD_Z = 0.16, 0.15

def prone_at(breath=0.0):
    params = dict(BASE); params['spine'] = [a + breath for a in BASE['spine']]
    p = prone_pose(params)
    lay_legs(p, 9, 30)
    place_hips(p, HIPS_Y, HEAD_Z)
    return p

def crawl_at(phase):
    """The crawl, `phase` (0..1) of the way through its cycle: each knee in turn drawn up and out,
    the hips swaying toward it, the shoulders reaching forward in turn."""
    s = math.sin(2 * math.pi * phase)
    k_left, k_right = max(0.0, s), max(0.0, -s)
    p = prone_pose(BASE)
    # hips and spine: sway toward the drawn knee, counter-turned up the spine
    sway = s  # + toward the left knee
    turn_about(p, 'Bind_Root_00_02', aa(UP, deg(7) * sway) @ aa(FWD, deg(-5) * sway))
    for b in ['Bind_Spine1_011_012', 'Bind_Spine2_012_013']:
        turn_about(p, b, aa(UP, deg(-4.5) * sway))
    # shoulders reach forward in turn (left with the right knee — a crawl's diagonal)
    c = math.cos(2 * math.pi * phase)
    turn_about(p, 'Bind_LeftShoulder_015_016', aa(UP, deg(-6) * c))
    turn_about(p, 'Bind_RightShoulder_034_035', aa(UP, deg(-6) * c))
    # legs: the drawing one's knee goes up and out, its shin back toward the middle
    for side, sgn, k in (('Left', 1.0, k_left), ('Right', -1.0, k_right)):
        up = {'Left': 'Bind_LeftUpLeg_02_03', 'Right': 'Bind_RightUpLeg_01_07'}[side]
        leg = {'Left': 'Bind_LeftLeg_04_04', 'Right': 'Bind_RightLeg_07_08'}[side]
        foot = {'Left': 'Bind_LeftFoot_05_05', 'Right': 'Bind_RightFoot_08_09'}[side]
        toe = {'Left': 'Bind_LeftToeBase_06_06', 'Right': 'Bind_RightToeBase_09_010'}[side]
        out = -RIGHT * sgn
        th = deg(9 + 55 * k); sh = deg(9 - 35 * k)
        thigh = -FWD * math.cos(th) + out * math.sin(th) + UP * -0.05
        shin = -FWD * math.cos(sh) + out * math.sin(sh) + UP * (0.02 + 0.1 * k)
        aim_bone(p, up, leg, thigh)
        aim_bone(p, leg, foot, shin)
        t = deg(30 - 20 * k)
        aim_bone(p, foot, toe, -UP * math.cos(t) - FWD * math.sin(t) + out * 0.3 * k)
    place_hips(p, HIPS_Y + 0.02 * abs(s), HEAD_Z)
    return p


def two_bone_reach(p, upper, fore, hand, target, pole):
    """Turn the upper arm and forearm so the wrist's at `target` (m) — or as near as the arm reaches —
    the elbow bent toward `pole`; the hand keeps its turn on the forearm."""
    a, b, c = p.pos(upper), p.pos(fore), p.pos(hand)
    lab, lbc = np.linalg.norm(b - a), np.linalg.norm(c - b)
    to = np.asarray(target, float) - a
    dist = min(max(np.linalg.norm(to), abs(lab - lbc) + 1e-4), (lab + lbc) * 0.999)
    d = to / np.linalg.norm(to)
    side = np.asarray(pole, float) - d * np.dot(pole, d); side /= np.linalg.norm(side)
    along = (lab * lab - lbc * lbc + dist * dist) / (2 * dist)
    elbow = a + d * along + side * math.sqrt(max(lab * lab - along * along, 0))
    turn_about(p, upper, arc(b - a, elbow - a))
    turn_about(p, fore, arc(p.pos(hand) - p.pos(fore), a + d * dist - elbow))

def throw_at(key):
    """One key of the overhand throw (`key`: its numbers, below)."""
    p = Pose(g, 17, 0.0)  # `idleWithoutGun`: no weapon in hand
    yaw, lean = key["yaw"], key["lean"]
    for b, share in zip(["Bind_Spine_010_011", "Bind_Spine1_011_012", "Bind_Spine2_012_013"], [0.3, 0.35, 0.35]):
        turn_about(p, b, aa(UP, deg(yaw * share)) @ aa(RIGHT, deg(lean * share)))
    # (Eyes kept on where it's going.)
    for b, share in (("Bind_Neck_013_014", 0.5), ("Bind_Head_014_015", 0.5)):
        turn_about(p, b, aa(UP, deg(-yaw * share)) @ aa(RIGHT, deg(-lean * share)))
    body = lambda v: RIGHT * v[0] + UP * v[1] + FWD * v[2]
    rs = p.pos("Bind_RightArm_035_036")
    two_bone_reach(p, "Bind_RightArm_035_036", "Bind_RightForeArm_036_037", "Bind_RightHand_037_038",
                   rs + body(key["right"]), body(key["right_elbow"]))
    ls = p.pos("Bind_LeftArm_016_017")
    two_bone_reach(p, "Bind_LeftArm_016_017", "Bind_LeftForeArm_017_018", "Bind_LeftHand_018_019",
                   ls + body(key["left"]), body(key["left_elbow"]))
    return p

# The throw's keys: the chest's turn (yaw, + to the left) and lean (+ back),
# degrees; each wrist from its shoulder and the way its elbow points (m:
# right, up, forward from the body).
THROW_KEYS = [
    # ready: cocked back behind the head, the other arm pointing where it's going
    (0.0, dict(yaw=-30, lean=6, right=(0.12, 0.14, -0.2), right_elbow=(1, -0.2, -0.3),
               left=(0.05, 0.08, 0.5), left_elbow=(-1, -0.5, 0))),
    # wound up further
    (0.12, dict(yaw=-38, lean=10, right=(0.14, 0.12, -0.28), right_elbow=(1, -0.1, -0.4),
                left=(0.0, 0.05, 0.48), left_elbow=(-1, -0.6, 0))),
    # the release: over the top and out in front, the chest coming round
    (0.25, dict(yaw=12, lean=-10, right=(-0.02, 0.3, 0.42), right_elbow=(0.6, 0.2, -0.5),
                left=(-0.1, -0.3, 0.1), left_elbow=(-1, -0.2, -0.6))),
    # the follow-through: down across the body
    (0.42, dict(yaw=28, lean=-16, right=(-0.32, -0.38, 0.28), right_elbow=(1, -0.3, 0.2),
                left=(-0.12, -0.42, -0.08), left_elbow=(-1, 0, -0.5))),
    # easing back up
    (0.667, dict(yaw=12, lean=-6, right=(-0.15, -0.45, 0.12), right_elbow=(1, -0.6, 0),
                 left=(-0.1, -0.5, -0.02), left_elbow=(-1, 0, -0.3))),
]
THROW_RELEASE_SECS = 0.25


DRINK_SECS = 1.667          # the first-person drink: frames 160..410 at 30 fps, played 5x
BOTTLE_HALF = 0.12          # half the bottle's height (m) — the game's `BOTTLE_HEIGHT` / 2
DRINK_HOLD_POS = np.array([0.0, 0.08, 0.035])  # the bottle's middle from the hand bone (m, its axes)

def bottle_frame(axis):
    """A turn whose +Y is `axis` (the bottle's bottom-to-neck), its +X as near the body's right as can be."""
    y = np.asarray(axis, float); y = y / np.linalg.norm(y)
    x = RIGHT - y * np.dot(RIGHT, y); x = x / np.linalg.norm(x)
    return np.c_[x, y, np.cross(x, y)]

def drink_hold():
    """The bottle's turn in the hand: standing upright in the hand as it hangs at the hip."""
    p = Pose(g, 17, 0.0)
    return p.rot("Bind_RightHand_037_038").T @ bottle_frame(UP)

def drink_at(key, hold):
    p = Pose(g, 17, 0.0)  # `idleWithoutGun`
    for b, share in zip(["Bind_Spine_010_011", "Bind_Spine1_011_012", "Bind_Spine2_012_013"], [0.3, 0.35, 0.35]):
        turn_about(p, b, aa(RIGHT, deg(key["lean"] * share)))
    for b, share in (("Bind_Neck_013_014", 0.4), ("Bind_Head_014_015", 0.6)):
        turn_about(p, b, aa(RIGHT, deg(key["head"] * share)))
    if key.get("bottle") is None:
        return p  # at the hip, as the clip has the hand
    head = p.pos("Bind_Head_014_015")
    # (The mouth: forward of the head joint and a little down, tipped back with it.)
    mouth = head + aa(RIGHT, deg(key["head"])) @ (FWD * 0.11 - UP * 0.06)
    axis = np.asarray(key["axis"], float); axis = axis / np.linalg.norm(axis)
    body = lambda v: RIGHT * v[0] + UP * v[1] + FWD * v[2]
    if key["bottle"] == "mouth":
        middle = mouth - axis * BOTTLE_HALF   # its neck at the lips
    else:
        middle = mouth + body(key["bottle"])
    hand_rot = bottle_frame(axis) @ hold.T
    wrist = middle - hand_rot @ DRINK_HOLD_POS
    two_bone_reach(p, "Bind_RightArm_035_036", "Bind_RightForeArm_036_037", "Bind_RightHand_037_038",
                   wrist, RIGHT * 1.0 - UP * 0.8)
    p.set_global_rot("Bind_RightHand_037_038", hand_rot)
    return p

# The drink's keys: the chest's lean and the head's tip back (degrees, +
# back); where the bottle is — at the hip as the hand hangs (None), its neck
# at the lips ("mouth"), or its middle from the mouth (m: right, up,
# forward) — and the way its neck points.
DRINK_KEYS = [
    (0.00, dict(lean=0, head=0, bottle=None)),
    (0.14, dict(lean=0, head=0, bottle=(0.06, -0.3, 0.26), axis=(0, 1, -0.25))),
    (0.28, dict(lean=2, head=12, bottle="mouth", axis=(0, -0.25, -1))),
    (0.48, dict(lean=5, head=26, bottle="mouth", axis=(0, -0.65, -0.75))),
    (0.66, dict(lean=6, head=30, bottle="mouth", axis=(0, -0.85, -0.5))),
    (0.80, dict(lean=1, head=6, bottle=(0.06, -0.3, 0.26), axis=(0, 1, -0.25))),
    (1.00, dict(lean=0, head=0, bottle=None)),
]

CRAWL_SECS = 1.4
def build(path):
    j, b = fresh()
    # prone idle: a slow breath
    times = [0.0, 1.0, 2.0]
    add_clip('prone', times, [prone_at(0.0), prone_at(1.5), prone_at(0.0)], j, b)
    n = 16
    times = [CRAWL_SECS * i / n for i in range(n + 1)]
    fwd = [crawl_at(i / n) for i in range(n + 1)]
    add_clip('proneCrawl', times, fwd, j, b)
    add_clip('proneCrawlBack', times, list(reversed(fwd)), j, b)
    add_clip('throw', [t for t, _ in THROW_KEYS], [throw_at(k) for _, k in THROW_KEYS], j, b)
    hold = drink_hold()
    add_clip('drink', [t * DRINK_SECS for t, _ in DRINK_KEYS], [drink_at(k, hold) for _, k in DRINK_KEYS], j, b)
    x, y, z = (math.degrees(a) for a in (math.atan2(-hold[1, 2], hold[2, 2]), math.asin(max(-1, min(1, hold[0, 2]))), math.atan2(-hold[0, 1], hold[0, 0])))
    print(f"DRINK_HOLD: pos {tuple(DRINK_HOLD_POS)}, turn_deg ({x:.2f}, {y:.2f}, {z:.2f})")
    write_glb(path, j, b)

def main():
    global g
    g = Glb(SRC)
    # (Our own clips from a previous run go; the model's own 18 stay.)
    # Everything they used sits at the end of the file (they were written
    # last), so cut back to where it starts.
    j = g.json
    ours = [a for a in j["animations"] if a.get("name") in CLIP_NAMES]
    if ours:
        used = [s[k] for a in ours for s in a["samplers"] for k in ("input", "output")]
        first_acc = min(used)
        first_view = min(j["accessors"][i]["bufferView"] for i in used)
        j["animations"] = [a for a in j["animations"] if a.get("name") not in CLIP_NAMES]
        cut = j["bufferViews"][first_view].get("byteOffset", 0)
        del j["accessors"][first_acc:]
        del j["bufferViews"][first_view:]
        del g.bin[cut:]
    build(SRC)
    print("wrote", ", ".join(CLIP_NAMES), "into", os.path.normpath(SRC))


if __name__ == "__main__":
    main()
