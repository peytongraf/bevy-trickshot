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
CLIP_NAMES = ("prone", "proneCrawl", "proneCrawlBack")

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
