#!/usr/bin/env python3
"""Generate the zombies' shambling walks and write them into zombie.glb.

    python3 client/tools/zombie_walk_clips.py

Appends them to `client/assets/models/characters/zombie.glb`, after its own
11 animations (whose indices, nodes and meshes are left exactly as they are
— `avatars::zombie` looks them up by index), in [`VARIANTS`]' order:

* `ShambleWalk` (11) — hunched, limping, head lolled, one arm dead and one
  reaching;
* `ShambleWalkHeavy` (12) — the same, further gone: long, slow, lurching
  strides, the right leg dragging short, bent right over;
* `StumbleWalk` (13) — four steps long, not two, no two of them alike: off
  balance, swaying, every step its own length and timing, a stagger early
  on and a real stumble later (it lurches forward and over, flings its arms
  out, its head snaps back, then it slowly rights itself); arms and head
  hanging loose and trailing the body. See `stumble_at` / `STUMBLE`;
* `ScaryRun` (14) — four steps long too, off the model's own `Run_InPlace`
  (3): lunging, bent hard forward with its eyes up on you, ragged frantic
  strides, the back heaving each footfall, the shoulders thrashing, the
  head twitching, both arms out in front clawing and flailing, out of step
  with each other. See `scary_run_at` / `SCARY_RUN`;
* `BrokenRun` (15) — four steps, off the run too, but wrong rather than
  wild: its neck broken, the head lying over on its shoulder and jerking;
  its torso wrenched round and over, running half sideways; the right leg
  locked stiff, so it gallops on a hitch; a stuttering, stop-start pace;
  one arm trailing dead behind it, the other bent back on itself and
  twitching. See `broken_run_at` / `BROKEN_RUN`.

Each is the model's own `Walk1_InPlace` (1) — its legs and footfalls — made
a Call of Duty zombie's shamble on top:

* a limp: the cycle's time warped so one step drags and the next lurches
  (`limp`), the hips dropping to one side and back with each stride;
* longer strides (`stride_left` / `stride_right`): each leg's swing scaled
  up about the middle of its cycle, the hips lowered as far as that lifts
  the planted foot, so it stays on the ground — the game plays such a clip
  slower for the same speed, so the legs move slower too;
* hunched over and listing to its left: the spine bent forward and to the
  side, the shoulders twisting against the stride, a heave of the back each
  step;
* the head lolled over to its right, held up to look ahead, nodding with
  each step and twitching;
* the right arm hanging dead from a dropped shoulder, swinging late like a
  pendulum; the left reaching out ahead, elbow bent, hand drooping.

Everything added repeats exactly once a cycle (or a whole number of times),
so each loops as cleanly as the walk it's built on. Re-running it replaces
them (it drops the ones it wrote before) — tune `VARIANTS` and run it again.
Needs numpy. Preview them in Blender (or with `client/tools/render_clip.py`).
"""
import json
import math
import os
import struct

import numpy as np

SRC = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "assets", "models", "characters", "zombie.glb")
BASE_CLIP = 1  # Walk1_InPlace

# --- the shambles (degrees unless said) -------------------------------------
SHAMBLE = dict(
    limp=0.22,               # how uneven the steps are: the time warp's depth (0 = even)
    stride_left=1.0,         # each leg's swing (× the walk's) — longer strides
    stride_right=1.0,
    hip_drop=6.0,            # the hips dropping side to side, once a cycle
    hunch=(12.0, 10.0, 8.0), # forward bend of Spine1, Spine2, the ribcage
    heave=4.0,               # more hunch at each step
    list=8.0,                # the spine leaning to its left
    shoulder_twist=6.0,      # the shoulders turning against the stride
    head_up=16.0,            # the neck held back, so it looks ahead despite the hunch
    head_loll=24.0,          # the head tipped over to its right
    head_nod=4.0,            # the head nodding with each step
    twitches=(               # (when in the cycle, 0..1; how far; about what axis — x left, y up, z forward)
        (0.33, 14.0, (0.3, 1.0, 0.6)),
        (0.81, 10.0, (-0.5, 0.2, 1.0)),
    ),
    twitch_width=0.03,       # how long a twitch lasts (share of the cycle)
    dead_shoulder=10.0,      # the right shoulder dropped
    dead_swing=12.0,         # the right arm's pendulum swing...
    dead_lag=1.3,            # ...lagging the stride by this (radians)
    reach=70.0,              # the left arm raised out ahead
    reach_bob=6.0,           # ...bobbing with each step
    reach_elbow=30.0,        # its elbow bent
    reach_wrist=20.0,        # its hand drooping
)
HEAVY = dict(
    SHAMBLE,
    limp=0.38,
    # (The game plays it at the walk's rate ÷ about their mean — 1.5,
    # `avatars::zombie::HEAVY_STRIDE`.)
    stride_left=1.7,
    stride_right=1.3,
    hip_drop=10.0,
    hunch=(16.0, 13.0, 10.0),
    heave=8.0,
    list=12.0,
    shoulder_twist=10.0,
    head_up=22.0,
    head_loll=30.0,
    head_nod=7.0,
    twitches=(
        (0.18, 18.0, (0.3, 1.0, 0.6)),
        (0.55, 12.0, (1.0, 0.1, 0.2)),
        (0.86, 16.0, (-0.5, 0.2, 1.0)),
    ),
    dead_shoulder=14.0,
    dead_swing=22.0,
    reach=45.0,
    reach_bob=12.0,
    reach_elbow=35.0,
    reach_wrist=30.0,
)
VARIANTS = (("ShambleWalk", SHAMBLE), ("ShambleWalkHeavy", HEAVY))

# The stumble (`stumble_at`): four steps a loop — the walk read twice over.
STUMBLE_NAME = "StumbleWalk"
STUMBLE = dict(
    # The timing: three rhythms over the four steps, so none is like another
    # (their sum stays under the walk's own pace, so it never runs backwards).
    warp=((0.55, 1, 0.0), (0.35, 3, 1.1), (0.2, 5, 2.4)),  # (depth, times a loop, phase)
    # Each step's length (× the walk's): a mean, wandering over the loop.
    # (The game plays it at the walk's rate ÷ the mean — `STUMBLE_STRIDE`.)
    stride=1.35, stride_wander=0.25,
    hip_drop=7.0,
    hunch=(12.0, 9.0, 7.0),
    sway=10.0,                # the upper body swaying side to side, once a loop...
    sway_wobble=4.0,          # ...wobbling on top, three times
    twist=7.0,
    head_up=14.0,
    head_loll=14.0, head_wander=10.0,
    head_lag=0.6,             # how far (radians) the head trails the sway
    # The falls: (when in the loop, 0..1; how big, 0..1). Each lurches it
    # forward and over to its right, arms flung out, head snapped back —
    # quick in, slow out.
    stumbles=((0.12, 0.45), (0.58, 1.0)),
    stumble_rise=0.035, stumble_fall=0.13,
    stumble_bend=20.0, stumble_list=14.0, stumble_head=-18.0, stumble_flail=40.0, stumble_reach=25.0,
    # The arms hanging loose, forward a little, swinging late — each its own.
    arm_forward=(18.0, 28.0),  # (right, left)
    arm_swing=(14.0, 10.0),
    arm_lag=(1.4, 0.9),
    elbow=(15.0, 25.0),
)

# --- maths -------------------------------------------------------------------
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
def slerp(a, b, t):
    a=np.asarray(a,float); b=np.asarray(b,float); d=float(np.dot(a,b))
    if d<0: b=-b; d=-d
    if d>0.9995:
        q=a+(b-a)*t; return q/np.linalg.norm(q)
    th=math.acos(min(1.0,d)); s=math.sin(th)
    return (math.sin((1-t)*th)*a+math.sin(t*th)*b)/s
deg = math.radians

# The model's frame (its scene root's): up +Y, forward +Z, its right -X.
UP = np.array([0, 1.0, 0]); FWD = np.array([0, 0, 1.0]); LEFT = np.array([1.0, 0, 0])
BEND = LEFT                 # turning about this bends forward (up toward forward)
LEAN_LEFT = -FWD            # ...this leans over to its left
TIP_RIGHT = FWD             # ...this tips the top over to its right
SWING_FWD = -LEFT           # ...this swings a hanging arm forward

# --- the model ---------------------------------------------------------------
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

class Clip:
    """A clip's channels, sampled anywhere in it (linear / slerp, looping)."""
    def __init__(self, glb, index):
        an=glb.json['animations'][index]; self.channels=[]
        for ch in an['channels']:
            s=an['samplers'][ch['sampler']]
            self.channels.append((ch['target']['node'], ch['target']['path'], glb.acc(s['input'])[:,0], glb.acc(s['output'])))
        self.duration=max(c[2][-1] for c in self.channels)
        # Each bone's middle turn over the cycle (the keys averaged, all on
        # the same side) — what a stride's swing is scaled about.
        self.mean={}
        for node, path, _, values in self.channels:
            if path=='rotation':
                ref=values[0]; acc=np.zeros(4)
                for q in values: acc+=q if np.dot(q,ref)>=0 else -q
                self.mean[node]=acc/np.linalg.norm(acc)
    def sample(self, times, values, t, rotation):
        if len(times)==1 or t<=times[0]: return values[0]
        if t>=times[-1]: return values[-1]
        k=int(np.searchsorted(times, t, side='right'))-1
        f=(t-times[k])/(times[k+1]-times[k])
        return slerp(values[k], values[k+1], f) if rotation else values[k]+(values[k+1]-values[k])*f

class Pose:
    """Local TRS of every node; FK in the scene root's frame."""
    def __init__(self, glb):
        self.g=glb; n=glb.nodes
        self.T=[np.array(x.get('translation',[0,0,0]),float) for x in n]
        self.R=[qmat(x.get('rotation',[0,0,0,1])) for x in n]
        self.S=[np.array(x.get('scale',[1,1,1]),float) for x in n]
    def local(self,i):
        M=np.eye(4); M[:3,:3]=self.R[i]*self.S[i]; M[:3,3]=self.T[i]; return M
    def glob(self,i):
        M=self.local(i); par=self.g.parent
        while i in par: i=par[i]; M=self.local(i)@M
        return M
    def pos(self,name):
        return self.glob(self.g.idx[name])[:3,3]
    def shift(self,name,d_world):
        """Move bone `name` by `d_world` (the scene root's frame)."""
        i=self.g.idx[name]; Mp=self.glob(self.g.parent[i])[:3,:3]
        self.T[i]=self.T[i]+np.linalg.solve(Mp, d_world)
    def rot(self,name):
        M=self.glob(self.g.idx[name])[:3,:3]; return M/np.linalg.norm(M[:,0])
    def turn(self, name, R_world):
        """Turn bone `name` (and everything on it) by world rotation `R_world` about its own joint."""
        i=self.g.idx[name]; Rp=self.glob(self.g.parent[i])[:3,:3]; Rp=Rp/np.linalg.norm(Rp[:,0])
        self.R[i]=Rp.T@(R_world@self.rot(name))

# --- the shamble -------------------------------------------------------------
def wrapped_pulse(x, at, width):
    """A bump at `at` (0..1 round the cycle), `width` wide — the same at 0 and 1."""
    d=(x-at+0.5)%1.0-0.5
    return math.exp(-(d/width)**2)

LEGS = (
    ('Base HumanRThigh_02', 'Base HumanRCalf_03', 'Base HumanRFoot_04', 'stride_right'),
    ('Base HumanLLegThigh_051', 'Base HumanLLegCalf_052', 'Base HumanLLegFoot_053', 'stride_left'),
)
SOLES = ('Base HumanRFoot_04', 'Base HumanRDigit11_05', 'Base HumanLLegFoot_053', 'Base HumanLLegDigit11_054')

def lowest(p):
    return min(p.pos(n)[1] for n in SOLES)

def swing(p, clip, name, k):
    """Scale bone `name`'s turn away from its middle one by `k`."""
    i=p.g.idx[name]; m=qmat(clip.mean[i]); q=mquat(m.T@p.R[i])
    if q[3]<0: q=-q
    ang=2*math.atan2(np.linalg.norm(q[:3]), q[3])
    if ang<1e-6: return
    p.R[i]=m@aa(q[:3], ang*k)

def shamble_at(glb, clip, x, v):
    """Shamble `v` `x` (0..1) of the way through its cycle."""
    T=clip.duration; phi=2*math.pi*x
    # The limp: the base walk read at a warped time — slow through one step,
    # quick through the next — that still starts and ends where it does.
    src=(x+v['limp']/(2*math.pi)*math.sin(phi))%1.0*T
    p=Pose(glb)
    for node, path, times, values in clip.channels:
        val=clip.sample(times, values, src, path=='rotation')
        if path=='rotation': p.R[node]=qmat(val/np.linalg.norm(val))
        elif path=='translation': p.T[node]=np.array(val,float)
        elif path=='scale': p.S[node]=np.array(val,float)
    step=math.sin(2*phi)  # once each footfall

    # Longer strides: each leg swung further, the hips lowered as far as
    # that lifted the planted foot.
    ground=lowest(p)
    for thigh, calf, foot, key in LEGS:
        k=v[key]
        if k!=1.0:
            swing(p, clip, thigh, k); swing(p, clip, calf, k); swing(p, clip, foot, math.sqrt(k))
    p.shift('Base HumanPelvis_01', np.array([0.0, ground-lowest(p), 0.0]))

    # Hips dropping side to side — the legs kept as they were, so the feet stay put.
    hip=aa(FWD, deg(v['hip_drop'])*math.sin(phi))
    thighs=[p.rot(n) for n in ('Base HumanRThigh_02','Base HumanLLegThigh_051')]
    p.turn('Base HumanPelvis_01', hip)
    for n, was in zip(('Base HumanRThigh_02','Base HumanLLegThigh_051'), thighs):
        p.turn(n, was@p.rot(n).T)

    # Hunched over, listing to its left, the back heaving each step and the
    # shoulders twisting against the stride.
    hunch=v['hunch']
    heave=deg(v['heave'])*(0.5+0.5*math.sin(2*phi-0.5))
    p.turn('Base HumanSpine1_06', aa(BEND, deg(hunch[0]))@aa(LEAN_LEFT, deg(v['list'])))
    p.turn('Base HumanSpine2_00', aa(BEND, deg(hunch[1])+heave)@aa(UP, deg(v['shoulder_twist'])*math.sin(phi)))
    p.turn('Base HumanRibcage_07', aa(BEND, deg(hunch[2])))

    # The head: held up to look ahead, lolled to its right, nodding, twitching.
    p.turn('Base HumanNeck_08', aa(BEND, -deg(v['head_up'])))
    head=aa(TIP_RIGHT, deg(v['head_loll']))@aa(BEND, deg(v['head_nod'])*step)
    for at, amount, axis in v['twitches']:
        head=aa(axis, deg(amount)*wrapped_pulse(x, at, v['twitch_width']))@head
    p.turn('Base HumanHead_09', head)

    # The right arm hanging dead from a dropped shoulder, swinging late.
    p.turn('Base HumanRArmCollarbone_015', aa(FWD, deg(v['dead_shoulder'])))
    p.turn('Base HumanRArmUpperarm_016', aa(SWING_FWD, deg(v['dead_swing'])*math.sin(phi-v['dead_lag'])))

    # The left reaching out ahead, elbow bent, hand drooping.
    p.turn('Base HumanLArmUpperarm_034', aa(SWING_FWD, deg(v['reach'])+deg(v['reach_bob'])*math.sin(2*phi+0.4)))
    p.turn('Base HumanLArmForearm_035', aa(SWING_FWD, deg(v['reach_elbow'])))
    p.turn('Base HumanLArmPalm_036', aa(BEND, deg(v['reach_wrist'])))
    return p

def stumble_envelope(x, at, rise, fall):
    """A stumble's strength `x` (0..1) through the loop: quick to `at`, slow to fade — wrapped."""
    d=(x-at+0.5)%1.0-0.5
    w=rise if d<0 else fall
    return math.exp(-(d/w)**2)

def stumble_at(glb, clip, x):
    """The stumble `x` (0..1) of the way through its four-step loop."""
    v=STUMBLE; T=clip.duration; psi=2*math.pi*x
    # Two passes of the walk, warped by the three rhythms.
    src=2*x+sum(a/(2*math.pi*n)*math.sin(n*psi+ph) for a, n, ph in v['warp'])
    p=Pose(glb)
    for node, path, times, values in clip.channels:
        val=clip.sample(times, values, (src%1.0)*T, path=='rotation')
        if path=='rotation': p.R[node]=qmat(val/np.linalg.norm(val))
        elif path=='translation': p.T[node]=np.array(val,float)
        elif path=='scale': p.S[node]=np.array(val,float)
    walk=2*math.pi*src  # where in the walk's own stride it is (twice a loop)
    trip=sum(amount*stumble_envelope(x, at, v['stumble_rise'], v['stumble_fall']) for at, amount in v['stumbles'])

    # Every step its own length, the hips lowered to keep the planted foot down.
    k=v['stride']+v['stride_wander']*math.sin(psi*3+0.6)
    ground=lowest(p)
    for thigh, calf, foot, _ in LEGS:
        swing(p, clip, thigh, k); swing(p, clip, calf, k); swing(p, clip, foot, math.sqrt(k))
    p.shift('Base HumanPelvis_01', np.array([0.0, ground-lowest(p), 0.0]))

    # The hips dropping on the off-beat, the legs kept where they were.
    hip=aa(FWD, deg(v['hip_drop'])*math.sin(walk+0.8))
    thighs=[p.rot(n) for n in ('Base HumanRThigh_02','Base HumanLLegThigh_051')]
    p.turn('Base HumanPelvis_01', hip)
    for n, was in zip(('Base HumanRThigh_02','Base HumanLLegThigh_051'), thighs):
        p.turn(n, was@p.rot(n).T)

    # Off balance: swaying over to one side and the other, wobbling, and
    # pitched forward and over when it stumbles.
    sway=deg(v['sway'])*math.sin(psi)+deg(v['sway_wobble'])*math.sin(3*psi+1.0)
    h=v['hunch']
    p.turn('Base HumanSpine1_06', aa(BEND, deg(h[0])+deg(v['stumble_bend'])*trip*0.5)@aa(LEAN_LEFT, sway-deg(v['stumble_list'])*trip))
    p.turn('Base HumanSpine2_00', aa(BEND, deg(h[1])+deg(v['stumble_bend'])*trip*0.5)@aa(UP, deg(v['twist'])*math.sin(walk-0.4)))
    p.turn('Base HumanRibcage_07', aa(BEND, deg(h[2])))

    # The head: lolling about loose, trailing the sway, snapped back as it stumbles.
    p.turn('Base HumanNeck_08', aa(BEND, -deg(v['head_up'])+deg(v['stumble_head'])*trip))
    loll=deg(v['head_loll'])+deg(v['head_wander'])*math.sin(2*psi+0.3)
    lag=-(deg(v['sway'])*math.sin(psi-v['head_lag']))*0.6
    p.turn('Base HumanHead_09', aa(TIP_RIGHT, loll+lag)@aa(BEND, deg(4.0)*math.sin(walk+1.9)))

    # The arms: hanging loose and a little forward, each swinging late in its
    # own way — flung out for balance as it stumbles.
    for side, (collar, upper, fore) in enumerate((
        ('Base HumanRArmCollarbone_015', 'Base HumanRArmUpperarm_016', 'Base HumanRArmForearm_017'),
        ('Base HumanLArmCollarbone_033', 'Base HumanLArmUpperarm_034', 'Base HumanLArmForearm_035'),
    )):
        out=-FWD if side==0 else FWD  # turning about this swings that arm out to its side
        swing_fwd=deg(v['arm_forward'][side])+deg(v['arm_swing'][side])*math.sin(walk-v['arm_lag'][side]+side*math.pi)
        p.turn(upper, aa(out, deg(v['stumble_flail'])*trip)@aa(SWING_FWD, swing_fwd+deg(v['stumble_reach'])*trip))
        p.turn(fore, aa(SWING_FWD, deg(v['elbow'][side])*(1.0-0.5*trip)))
    return p

RUN_CLIP = 3  # Run_InPlace
SCARY_RUN_NAME = "ScaryRun"
SCARY_RUN = dict(
    # Ragged timing over the four steps (depth, times a loop, phase).
    warp=((0.45, 1, 0.4), (0.3, 3, 2.0), (0.15, 7, 0.9)),
    # Each stride's length (× the run's): a mean, wandering.
    # (The game plays it at the run's rate ÷ the mean — `SCARY_RUN_STRIDE`.)
    stride=1.2, stride_wander=0.12,
    hip_drop=5.0,
    hunch=(18.0, 14.0, 10.0),  # Spine1, Spine2, ribcage: lunging forward
    heave=9.0,                 # more at each footfall
    list_wobble=5.0,           # side to side, three times a loop
    thrash=14.0,               # the shoulders twisting with the stride
    head_up=34.0,              # held right up — its eyes on you
    twitches=(                 # (when 0..1; how far; about what axis)
        (0.07, 16.0, (0.2, 1.0, 0.4)),
        (0.31, 22.0, (0.6, 0.3, 1.0)),
        (0.52, 14.0, (-0.3, 1.0, -0.5)),
        (0.78, 24.0, (-0.6, 0.2, 1.0)),
        (0.93, 12.0, (1.0, 0.4, 0.0)),
    ),
    twitch_width=0.018,
    # The arms out in front, clawing: (right, left) — forward, flail and its
    # speed (times a loop) and phase, out to the side, elbow, wrist hooked.
    reach=(30.0, 24.0),
    flail=(22.0, 26.0), flail_rate=(2, 3), flail_phase=(0.0, 1.7),
    arm_ceiling=8.0,           # the highest (° above level) an upper arm rises — out at you, never up...
    forearm_ceiling=20.0,      # ...and a forearm
    spread=(14.0, 20.0), spread_rate=(3, 5),
    elbow=(35.0, 28.0),
    claw=(40.0, 45.0),
)

def scary_run_at(glb, clip, x):
    """The scary run `x` (0..1) of the way through its four-step loop."""
    v=SCARY_RUN; T=clip.duration; psi=2*math.pi*x
    src=2*x+sum(a/(2*math.pi*n)*math.sin(n*psi+ph) for a, n, ph in v['warp'])
    p=Pose(glb)
    for node, path, times, values in clip.channels:
        val=clip.sample(times, values, (src%1.0)*T, path=='rotation')
        if path=='rotation': p.R[node]=qmat(val/np.linalg.norm(val))
        elif path=='translation': p.T[node]=np.array(val,float)
        elif path=='scale': p.S[node]=np.array(val,float)
    stride=2*math.pi*src  # where in the run's own stride it is (twice a loop)
    footfall=0.5+0.5*math.sin(2*stride-0.6)

    k=v['stride']+v['stride_wander']*math.sin(3*psi+0.2)
    ground=lowest(p)
    for thigh, calf, foot, _ in LEGS:
        swing(p, clip, thigh, k); swing(p, clip, calf, k); swing(p, clip, foot, math.sqrt(k))
    p.shift('Base HumanPelvis_01', np.array([0.0, ground-lowest(p), 0.0]))

    hip=aa(FWD, deg(v['hip_drop'])*math.sin(stride+0.5))
    thighs=[p.rot(n) for n in ('Base HumanRThigh_02','Base HumanLLegThigh_051')]
    p.turn('Base HumanPelvis_01', hip)
    for n, was in zip(('Base HumanRThigh_02','Base HumanLLegThigh_051'), thighs):
        p.turn(n, was@p.rot(n).T)

    h=v['hunch']
    p.turn('Base HumanSpine1_06', aa(BEND, deg(h[0]))@aa(LEAN_LEFT, deg(v['list_wobble'])*math.sin(3*psi+0.8)))
    p.turn('Base HumanSpine2_00', aa(BEND, deg(h[1])+deg(v['heave'])*footfall)@aa(UP, deg(v['thrash'])*math.sin(stride)))
    p.turn('Base HumanRibcage_07', aa(BEND, deg(h[2])))

    p.turn('Base HumanNeck_08', aa(BEND, -deg(v['head_up'])))
    head=aa(BEND, -deg(v['heave'])*0.6*footfall)  # (held level through the heave)
    for at, amount, axis in v['twitches']:
        head=aa(axis, deg(amount)*wrapped_pulse(x, at, v['twitch_width']))@head
    p.turn('Base HumanHead_09', head)

    for side, (upper, fore, palm) in enumerate((
        ('Base HumanRArmUpperarm_016', 'Base HumanRArmForearm_017', 'Base HumanRArmPalm_018'),
        ('Base HumanLArmUpperarm_034', 'Base HumanLArmForearm_035', 'Base HumanLArmPalm_036'),
    )):
        out=-FWD if side==0 else FWD
        fwd=deg(v['reach'][side])+deg(v['flail'][side])*math.sin(v['flail_rate'][side]*psi+v['flail_phase'][side])
        spread=deg(v['spread'][side])*(0.6+0.4*math.sin(v['spread_rate'][side]*psi+side))
        p.turn(upper, aa(out, spread)@aa(SWING_FWD, fwd))
        # (Out at you, never up over its head.)
        def cap(bone, tip, ceiling):
            d=p.pos(tip)-p.pos(bone); d=d/np.linalg.norm(d)
            over=math.asin(max(-1.0, min(1.0, d[1])))-deg(ceiling)
            if over>0:
                p.turn(bone, aa(np.cross(d, UP), -over))
        cap(upper, fore, v['arm_ceiling'])
        p.turn(fore, aa(SWING_FWD, deg(v['elbow'][side])*(0.8+0.4*math.sin(2*v['flail_rate'][side]*psi))))
        cap(fore, palm, v['forearm_ceiling'])
        p.turn(palm, aa(BEND, deg(v['claw'][side])))
    return p

BROKEN_RUN_NAME = "BrokenRun"
BROKEN_RUN = dict(
    # Stop-start: deep, quick rhythms — near-halts, then bursts (their depths
    # summed stay under the run's own pace, so it never goes backwards).
    warp=((0.5, 2, 0.3), (0.45, 5, 1.9), (0.25, 9, 0.6)),
    # The legs: the left striding long, the right locked stiff — its knee
    # barely bending. (The game plays it at the run's rate ÷ `stride` —
    # `BROKEN_RUN_STRIDE`.)
    stride=1.15, stiff_knee=0.35,
    hip_drop=11.0,             # the hitch: the hips dropping hard onto the good leg
    hunch=(10.0, 8.0, 6.0),
    wrench=32.0,               # the shoulders turned round from the hips (to its right)...
    wrench_wobble=8.0,         # ...wobbling
    list=16.0,                 # leaning hard over to its left
    neck_snap=70.0,            # the head lying over on its right shoulder
    head_up=10.0,
    jerks=(                    # (when 0..1; how far; about what axis)
        (0.11, 20.0, (0.0, 1.0, 0.3)),
        (0.37, 26.0, (1.0, 0.2, 0.0)),
        (0.63, 18.0, (0.2, 1.0, -0.4)),
        (0.84, 28.0, (-0.4, 0.3, 1.0)),
    ),
    jerk_width=0.015,
    drag_back=35.0,            # the right arm trailing dead behind...
    drag_swing=10.0,           # ...swinging a little
    bent_back=-25.0,           # the left arm held back and down...
    bent_elbow=-35.0,          # ...its elbow bent back the wrong way
    bent_twitch=18.0,          # ...twitching
)

def broken_run_at(glb, clip, x):
    """The broken run `x` (0..1) of the way through its four-step loop."""
    v=BROKEN_RUN; T=clip.duration; psi=2*math.pi*x
    src=2*x+sum(a/(2*math.pi*n)*math.sin(n*psi+ph) for a, n, ph in v['warp'])
    p=Pose(glb)
    for node, path, times, values in clip.channels:
        val=clip.sample(times, values, (src%1.0)*T, path=='rotation')
        if path=='rotation': p.R[node]=qmat(val/np.linalg.norm(val))
        elif path=='translation': p.T[node]=np.array(val,float)
        elif path=='scale': p.S[node]=np.array(val,float)
    stride=2*math.pi*src

    # The good leg long, the right locked stiff; the planted foot kept down.
    ground=lowest(p)
    swing(p, clip, 'Base HumanLLegThigh_051', v['stride'])
    swing(p, clip, 'Base HumanLLegCalf_052', v['stride'])
    swing(p, clip, 'Base HumanRThigh_02', v['stride'])
    swing(p, clip, 'Base HumanRCalf_03', v['stiff_knee'])
    p.shift('Base HumanPelvis_01', np.array([0.0, ground-lowest(p), 0.0]))

    # The hitch: the hips dropping hard onto the good leg each stride.
    hip=aa(FWD, deg(v['hip_drop'])*math.sin(stride))
    thighs=[p.rot(n) for n in ('Base HumanRThigh_02','Base HumanLLegThigh_051')]
    p.turn('Base HumanPelvis_01', hip)
    for n, was in zip(('Base HumanRThigh_02','Base HumanLLegThigh_051'), thighs):
        p.turn(n, was@p.rot(n).T)

    # Wrenched round and over: running half sideways.
    h=v['hunch']
    wrench=deg(v['wrench'])+deg(v['wrench_wobble'])*math.sin(3*psi+0.5)
    p.turn('Base HumanSpine1_06', aa(BEND, deg(h[0]))@aa(LEAN_LEFT, deg(v['list'])))
    p.turn('Base HumanSpine2_00', aa(BEND, deg(h[1]))@aa(UP, -wrench*0.6))
    p.turn('Base HumanRibcage_07', aa(BEND, deg(h[2]))@aa(UP, -wrench*0.4))

    # The neck broken: the head lying on its right shoulder, jerking.
    p.turn('Base HumanNeck_08', aa(TIP_RIGHT, deg(v['neck_snap'])*0.5)@aa(BEND, -deg(v['head_up'])))
    head=aa(TIP_RIGHT, deg(v['neck_snap'])*0.5)
    for at, amount, axis in v['jerks']:
        head=aa(axis, deg(amount)*wrapped_pulse(x, at, v['jerk_width']))@head
    p.turn('Base HumanHead_09', head)

    # The right arm trailing dead behind; the left held back, its elbow bent
    # the wrong way, twitching.
    p.turn('Base HumanRArmUpperarm_016', aa(SWING_FWD, -deg(v['drag_back'])+deg(v['drag_swing'])*math.sin(stride-1.5)))
    p.turn('Base HumanRArmForearm_017', aa(SWING_FWD, -deg(5.0)))
    twitch=sum(wrapped_pulse(x, at, v['jerk_width']*1.5) for at, _, _ in v['jerks'])
    p.turn('Base HumanLArmUpperarm_034', aa(SWING_FWD, deg(v['bent_back'])+deg(v['bent_twitch'])*twitch))
    p.turn('Base HumanLArmForearm_035', aa(SWING_FWD, deg(v['bent_elbow'])-deg(v['bent_twitch'])*twitch))
    p.turn('Base HumanLArmPalm_036', aa(BEND, deg(50.0)))
    return p

# --- writing it into the glb -------------------------------------------------
def add_clip(glb, clip, name, times, poses, j, b):
    """Append an animation keying every channel `clip` keys, from `poses` at `times`."""
    def push(arr, type_, minmax=False):
        data=np.asarray(arr, dtype='<f4')
        while len(b)%4: b.append(0)
        off=len(b); b.extend(data.tobytes())
        j['bufferViews'].append({'buffer':0,'byteOffset':off,'byteLength':data.nbytes})
        a={'bufferView':len(j['bufferViews'])-1,'componentType':5126,'count':int(data.shape[0]),'type':type_}
        if minmax:
            a['min']=[float(data.min())]; a['max']=[float(data.max())]
        j['accessors'].append(a)
        return len(j['accessors'])-1
    t_acc=push(np.array(times,float),'SCALAR',True)
    samplers,channels=[],[]
    for node, path, _, _ in clip.channels:
        if path=='rotation':
            vals=[]
            for p in poses:
                q=mquat(p.R[node])
                if vals and np.dot(q, vals[-1])<0: q=-q  # (the short way round between keys)
                vals.append(q)
            a=push(vals,'VEC4')
        elif path=='translation':
            a=push([p.T[node] for p in poses],'VEC3')
        else:
            a=push([p.S[node] for p in poses],'VEC3')
        samplers.append({'input':t_acc,'output':a,'interpolation':'LINEAR'})
        channels.append({'sampler':len(samplers)-1,'target':{'node':node,'path':path}})
    j['animations'].append({'name':name,'samplers':samplers,'channels':channels})

def write_glb(path, j, b):
    while len(b)%4: b.append(0)
    j['buffers'][0]['byteLength']=len(b)
    js=json.dumps(j, separators=(',',':')).encode()
    while len(js)%4: js+=b' '
    out=struct.pack('<III',0x46546C67,2,12+8+len(js)+8+len(b))
    out+=struct.pack('<II',len(js),0x4E4F534A)+js+struct.pack('<II',len(b),0x004E4942)+bytes(b)
    open(path,'wb').write(out)

def main():
    glb=Glb(SRC); j=glb.json
    names={name for name, _ in VARIANTS}|{STUMBLE_NAME, SCARY_RUN_NAME, BROKEN_RUN_NAME}
    # (Ours from a previous run go; the model's own 11 stay. Everything they
    # used sits at the end of the file — written last — so cut back to where
    # that starts.)
    ours=[a for a in j['animations'] if a.get('name') in names]
    if ours:
        used=[s[k] for a in ours for s in a['samplers'] for k in ('input','output')]
        first_acc=min(used); first_view=min(j['accessors'][i]['bufferView'] for i in used)
        j['animations']=[a for a in j['animations'] if a.get('name') not in names]
        cut=j['bufferViews'][first_view].get('byteOffset',0)
        del j['accessors'][first_acc:]; del j['bufferViews'][first_view:]; del glb.bin[cut:]
    clip=Clip(glb, BASE_CLIP)
    frames=int(round(clip.duration*30))
    times=[clip.duration*k/frames for k in range(frames+1)]
    for name, v in VARIANTS:
        poses=[shamble_at(glb, clip, k/frames, v) for k in range(frames+1)]
        add_clip(glb, clip, name, times, poses, j, glb.bin)
        print(f"wrote {name} ({len(j['animations'])-1})")
    # The stumble: four steps — two of the walk's loops — 30 frames a second.
    frames=int(round(clip.duration*2*30))
    times=[clip.duration*2*k/frames for k in range(frames+1)]
    poses=[stumble_at(glb, clip, k/frames) for k in range(frames+1)]
    add_clip(glb, clip, STUMBLE_NAME, times, poses, j, glb.bin)
    print(f"wrote {STUMBLE_NAME} ({len(j['animations'])-1})")
    # The scary run: four steps — two of the run's loops.
    run=Clip(glb, RUN_CLIP)
    frames=int(round(run.duration*2*30))
    times=[run.duration*2*k/frames for k in range(frames+1)]
    poses=[scary_run_at(glb, run, k/frames) for k in range(frames+1)]
    add_clip(glb, run, SCARY_RUN_NAME, times, poses, j, glb.bin)
    print(f"wrote {SCARY_RUN_NAME} ({len(j['animations'])-1})")
    poses=[broken_run_at(glb, run, k/frames) for k in range(frames+1)]
    add_clip(glb, run, BROKEN_RUN_NAME, times, poses, j, glb.bin)
    print(f"wrote {BROKEN_RUN_NAME} ({len(j['animations'])-1})")
    write_glb(SRC, j, glb.bin)
    print(f"into {os.path.normpath(SRC)}")

if __name__ == "__main__":
    main()
