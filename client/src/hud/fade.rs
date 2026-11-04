//! Fading the whole in-game HUD out and back in ([`HudFade`]) — for the
//! easter egg's cut scene (`easter_egg::cinematic`), as its black bars
//! slide in and out.
//!
//! Bevy's UI has no opacity for a whole tree, so every UI tree on screen
//! (each root `Node`, and everything under it) has its colours' alpha
//! scaled for the frame being drawn ([`fade_out`], in `Last`, after every
//! system's set them) and put back as they were straight after
//! ([`put_back`], in `First`) — so none of the HUD's own systems ever see a
//! faded colour, and it's back exactly as it was the moment the fade's
//! over. Its shader-drawn pieces have no colour to scale: the minimap fades
//! in its shader (`minimap`), and the few small shader icons (the Aether
//! Shroud's, exfil's, Rampage's) are hidden for the frame instead once it's
//! mostly faded ([`FADE_HIDE_BELOW`]).
//!
//! Left alone: anything marked [`KeepThroughHudFade`] (the egg's caption,
//! the radio, the debug readout, the cut scene's bars) and every overlay at
//! [`OVERLAY_Z`] or above — the menus, PAUSED, the results — so a menu opened
//! mid-fade is as it always is. The egui debug panel isn't Bevy UI, so it's
//! untouched too. [`HudFade`] goes back to fully shown on leaving a game.

use bevy::prelude::*;

use crate::AppState;

pub(crate) struct HudFadePlugin;

impl Plugin for HudFadePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<HudFade>()
            .init_resource::<Faded>()
            .add_systems(OnExit(AppState::InGame), |mut fade: ResMut<HudFade>| *fade = default())
            .add_systems(
                PostUpdate,
                hide_shader_icons.before(bevy::render::view::VisibilitySystems::VisibilityPropagate),
            )
            .add_systems(Last, fade_out)
            .add_systems(First, put_back);
    }
}

/// How much of the HUD shows (0 gone … 1 as it is). Whoever's fading it
/// sets it each frame.
#[derive(Resource)]
pub(crate) struct HudFade(pub(crate) f32);

impl Default for HudFade {
    fn default() -> Self {
        Self(1.0)
    }
}

/// A UI tree (or a part of one) the HUD's fade leaves alone.
#[derive(Component)]
pub(crate) struct KeepThroughHudFade;

/// Root UI at this `GlobalZIndex` or above is an overlay — a menu, PAUSED,
/// the results — never faded.
const OVERLAY_Z: i32 = 40;
/// Below this much shown, the shader-drawn icons are hidden for the frame.
const FADE_HIDE_BELOW: f32 = 0.5;

/// What this frame's fade changed, to put back: the colours, and the
/// shader icons hidden (and how they were).
#[derive(Resource, Default)]
struct Faded(Vec<Saved>, Vec<(Entity, Visibility)>);

struct Saved {
    entity: Entity,
    background: Option<Color>,
    border: Option<Color>,
    text: Option<Color>,
    shadow: Option<Color>,
    image: Option<Color>,
    outline: Option<Color>,
}

fn scaled(c: Color, by: f32) -> Color {
    c.with_alpha(c.alpha() * by)
}

#[allow(clippy::type_complexity)]
fn fade_out(
    state: Res<State<AppState>>,
    fade: Res<HudFade>,
    mut faded: ResMut<Faded>,
    roots: Query<(Entity, Option<&GlobalZIndex>), (With<Node>, Without<ChildOf>, Without<KeepThroughHudFade>)>,
    children: Query<&Children>,
    keep: Query<(), With<KeepThroughHudFade>>,
    mut looks: Query<(
        Option<&mut BackgroundColor>,
        Option<&mut BorderColor>,
        Option<&mut TextColor>,
        Option<&mut TextShadow>,
        Option<&mut ImageNode>,
        Option<&mut Outline>,
    )>,
) {
    let by = fade.0.clamp(0.0, 1.0);
    if by >= 1.0 || *state.get() != AppState::InGame {
        return;
    }
    let mut stack: Vec<Entity> = roots
        .iter()
        .filter(|(_, z)| z.is_none_or(|z| z.0 < OVERLAY_Z))
        .map(|(e, _)| e)
        .collect();
    while let Some(e) = stack.pop() {
        if keep.contains(e) {
            continue;
        }
        if let Ok(kids) = children.get(e) {
            stack.extend(kids.iter());
        }
        let mut saved = Saved {
            entity: e,
            background: None,
            border: None,
            text: None,
            shadow: None,
            image: None,
            outline: None,
        };
        if let Ok((background, border, text, shadow, image, outline)) = looks.get_mut(e) {
            if let Some(mut c) = background {
                saved.background = Some(c.0);
                c.0 = scaled(c.0, by);
            }
            if let Some(mut c) = border {
                saved.border = Some(c.0);
                c.0 = scaled(c.0, by);
            }
            if let Some(mut c) = text {
                saved.text = Some(c.0);
                c.0 = scaled(c.0, by);
            }
            if let Some(mut s) = shadow {
                saved.shadow = Some(s.color);
                s.color = scaled(s.color, by);
            }
            if let Some(mut i) = image {
                saved.image = Some(i.color);
                i.color = scaled(i.color, by);
            }
            if let Some(mut o) = outline {
                saved.outline = Some(o.color);
                o.color = scaled(o.color, by);
            }
        }
        faded.0.push(saved);
    }
}

/// Mostly faded: the shader-drawn icons hidden for the frame (before
/// visibility's worked out — so in `PostUpdate`, not with the colours). The
/// ones under something kept through the fade (or an overlay) are too —
/// there aren't any.
#[allow(clippy::type_complexity)]
fn hide_shader_icons(
    state: Res<State<AppState>>,
    fade: Res<HudFade>,
    mut faded: ResMut<Faded>,
    mut shader_icons: Query<
        (Entity, &mut Visibility),
        Or<(
            With<MaterialNode<crate::aether_shroud::ShroudIconMaterial>>,
            With<MaterialNode<crate::exfil::ExfilIconMaterial>>,
            With<MaterialNode<crate::rampage::RampageIconMaterial>>,
            With<MaterialNode<crate::rampage::RampageLightningMaterial>>,
        )>,
    >,
) {
    if fade.0 >= FADE_HIDE_BELOW || *state.get() != AppState::InGame {
        return;
    }
    for (e, mut vis) in &mut shader_icons {
        faded.1.push((e, *vis));
        *vis = Visibility::Hidden;
    }
}

fn put_back(
    mut faded: ResMut<Faded>,
    mut looks: Query<(
        Option<&mut BackgroundColor>,
        Option<&mut BorderColor>,
        Option<&mut TextColor>,
        Option<&mut TextShadow>,
        Option<&mut ImageNode>,
        Option<&mut Outline>,
    )>,
    mut visibilities: Query<&mut Visibility>,
) {
    for (e, was) in faded.1.drain(..) {
        if let Ok(mut v) = visibilities.get_mut(e) {
            *v = was;
        }
    }
    for s in faded.0.drain(..) {
        let Ok((background, border, text, shadow, image, outline)) = looks.get_mut(s.entity) else {
            continue;
        };
        if let (Some(mut c), Some(was)) = (background, s.background) {
            c.0 = was;
        }
        if let (Some(mut c), Some(was)) = (border, s.border) {
            c.0 = was;
        }
        if let (Some(mut c), Some(was)) = (text, s.text) {
            c.0 = was;
        }
        if let (Some(mut c), Some(was)) = (shadow, s.shadow) {
            c.color = was;
        }
        if let (Some(mut c), Some(was)) = (image, s.image) {
            c.color = was;
        }
        if let (Some(mut c), Some(was)) = (outline, s.outline) {
            c.color = was;
        }
    }
}
