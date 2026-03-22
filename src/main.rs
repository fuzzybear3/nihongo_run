use bevy::{asset::AssetMetaCheck, prelude::*};
use bevy::anti_alias::smaa::{Smaa, SmaaPreset};
use bevy::render::alpha::AlphaMode;
use bevy_egui::{EguiContexts, EguiPlugin, EguiPrimaryContextPass, egui};
use bevy_rich_text3d::{LoadFonts, Text3d, Text3dPlugin, Text3dStyling, TextAtlas};
use bevy::render::view::screenshot::{save_to_disk, Screenshot};
use rand::{rngs::SmallRng, Rng, SeedableRng};
use std::num::NonZero;

// ─── Window constants ─────────────────────────────────────────────────────────

const DEFAULT_WIDTH: u32 = 430;
const DEFAULT_HEIGHT: u32 = 932;

#[cfg(target_arch = "wasm32")]
const MAX_WEB_WIDTH: u32 = 430;
#[cfg(target_arch = "wasm32")]
const MAX_WEB_HEIGHT: u32 = 932;

fn measure_screen() -> (u32, u32) {
    #[cfg(target_arch = "wasm32")]
    {
        if let Some(win) = web_sys::window() {
            let w = win
                .inner_width()
                .ok()
                .and_then(|v| v.as_f64())
                .unwrap_or(DEFAULT_WIDTH as f64) as u32;
            let h = win
                .inner_height()
                .ok()
                .and_then(|v| v.as_f64())
                .unwrap_or(DEFAULT_HEIGHT as f64) as u32;
            return (w.min(MAX_WEB_WIDTH), h.min(MAX_WEB_HEIGHT));
        }
    }
    (DEFAULT_WIDTH, DEFAULT_HEIGHT)
}

// ─── Game constants ───────────────────────────────────────────────────────────

const PLAYER_SPEED: f32 = 10.0;
const STEER_SMOOTHING: f32 = 8.0;
const MAX_LATERAL: f32 = 2.0;
const DASH_SPEED_MULT: f32 = 4.0;
const SWIPE_UP_VELOCITY: f32 = 800.0; // pixels per second upward
const CAM_OFFSET_DEFAULT: Vec3 = Vec3::new(0.0, 2.5, 12.0);
const CAM_LERP_DEFAULT: f32 = 6.0;

#[derive(Resource)]
struct CameraSettings {
    height: f32,
    distance: f32,
    lerp: f32,
}

const TILE_LENGTH: f32 = 20.0;
const TILE_WIDTH: f32 = 5.0;
const TILES_AHEAD: i32 = 6;
const TILES_BEHIND: i32 = 3;

const GATE_SPACING: f32 = 50.0;
const GATES_AHEAD: u32 = 4;

// ─── N5 vocabulary ────────────────────────────────────────────────────────────
const N5_WORDS: &[(&str, &str)] = &[
    ("みず",      "water"),
    ("ひ",        "fire"),
    ("やま",      "mountain"),
    ("かわ",      "river"),
    ("き",        "tree"),
    ("はな",      "flower"),
    ("いぬ",      "dog"),
    ("ねこ",      "cat"),
    ("さかな",    "fish"),
    ("とり",      "bird"),
    ("たべる",    "eat"),
    ("のむ",      "drink"),
    ("いく",      "go"),
    ("くる",      "come"),
    ("みる",      "see"),
    ("きく",      "hear"),
    ("はなす",    "speak"),
    ("かく",      "write"),
    ("よむ",      "read"),
    ("かう",      "buy"),
    ("おおきい",  "big"),
    ("ちいさい",  "small"),
    ("あたらしい","new"),
    ("ふるい",    "old"),
    ("たかい",    "tall"),
    ("やすい",    "cheap"),
    ("しろい",    "white"),
    ("くろい",    "black"),
    ("あかい",    "red"),
    ("あおい",    "blue"),
];

// ─── Components & Resources ───────────────────────────────────────────────────

#[derive(Component)]
struct Player {
    lateral: f32,
    target_lateral: f32,
    dash_until_z: f32, // f32::INFINITY = not dashing
}

#[derive(Component)]
struct CameraMarker;

#[derive(Component)]
struct Tile;

/// Rotates around Y each frame to face the camera.
#[derive(Component)]
struct Billboard;

#[derive(Component)]
enum GateSign {
    Left,
    Right,
}

/// Slides the sign outward along X once the player passes gate_z.
#[derive(Component)]
struct SignSlide {
    gate_z: f32,
    dir: f32, // -1.0 = left, +1.0 = right
}

/// Marks the center post — tracks which side is correct.
#[derive(Component)]
struct GatePost {
    gate_z: f32,
    correct_side: f32, // -1.0 = left correct, +1.0 = right correct
    passed: bool,
}

#[derive(Resource, Default)]
struct FlashState {
    timer: f32,
    correct: bool,
}

#[derive(Resource)]
struct TileAssets {
    mesh: Handle<Mesh>,
    mat_a: Handle<StandardMaterial>,
    mat_b: Handle<StandardMaterial>,
}

#[derive(Resource)]
struct TileManager {
    frontier_z: f32,
    spawned: std::collections::VecDeque<(f32, Entity)>,
    count: u32,
}

#[derive(Resource)]
struct GateManager {
    next_z: f32,
    live: std::collections::VecDeque<(f32, Vec<Entity>)>,
}

#[derive(Resource, Default)]
struct DragState {
    active: bool,
    start_x: f32,
    current_x: f32,
    start_y: f32,
    current_y: f32,
    prev_y: f32,
    dashed: bool, // prevents re-triggering in the same gesture
    touch_id: Option<u64>,
}

/// Shared material for all Text3d entities.
#[derive(Resource)]
struct TextMat(Handle<StandardMaterial>);

#[derive(Resource)]
struct Deck {
    rng: SmallRng,
}

impl Deck {
    fn pick(&mut self) -> (&'static str, &'static str, &'static str) {
        let q = self.rng.gen_range(0..N5_WORDS.len());
        let mut d = self.rng.gen_range(0..N5_WORDS.len() - 1);
        if d >= q { d += 1; }
        (N5_WORDS[q].1, N5_WORDS[q].0, N5_WORDS[d].0)
    }
}

// ─── Main ─────────────────────────────────────────────────────────────────────

fn main() {
    let (w, h) = measure_screen();

    App::new()
        .add_plugins(
            DefaultPlugins
                .set(AssetPlugin {
                    meta_check: AssetMetaCheck::Never,
                    ..default()
                })
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        title: "Nihongo Run".into(),
                        resolution: (w, h).into(),
                        resizable: false,
                        ..default()
                    }),
                    ..default()
                }),
        )
        // Insert LoadFonts before the plugin so init_resource keeps our value
        .insert_resource(LoadFonts {
            font_embedded: vec![include_bytes!("../assets/fonts/TakaoPGothic.ttf")],
            ..default()
        })
        .add_plugins(Text3dPlugin::default())
        .add_plugins(EguiPlugin::default())
        .insert_resource(ClearColor(Color::srgb(0.4, 0.65, 0.85)))
        .insert_resource(DragState::default())
        .insert_resource(CameraSettings {
            height: CAM_OFFSET_DEFAULT.y,
            distance: CAM_OFFSET_DEFAULT.z,
            lerp: CAM_LERP_DEFAULT,
        })
        .add_systems(Startup, setup)
        .add_systems(
            Update,
            (
                input_system,
                steer_system,
                move_system,
                camera_follow_system,
                manage_tiles_system,
                manage_gates_system,
                gate_check_system,
                billboard_system,
                slide_signs_system,
                screenshot_system,
                auto_screenshot_system,
            )
                .chain(),
        )
        .add_systems(EguiPrimaryContextPass, camera_settings_ui)
        .run();
}

// ─── Startup ──────────────────────────────────────────────────────────────────

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    // Shared material for all Text3d entities
    let text_mat = materials.add(StandardMaterial {
        base_color_texture: Some(TextAtlas::DEFAULT_IMAGE.clone()),
        alpha_mode: AlphaMode::Mask(0.5),
        unlit: true,
        cull_mode: None,
        ..default()
    });
    commands.insert_resource(TextMat(text_mat));
    commands.insert_resource(Deck { rng: SmallRng::seed_from_u64(42) });
    commands.insert_resource(FlashState::default());

    // Lighting
    commands.insert_resource(GlobalAmbientLight {
        color: Color::WHITE,
        brightness: 400.0,
        ..default()
    });
    commands.spawn((
        DirectionalLight {
            illuminance: 8_000.0,
            shadows_enabled: false,
            ..default()
        },
        Transform::from_rotation(Quat::from_euler(EulerRot::XYZ, -0.8, 0.3, 0.0)),
    ));

    // Tile shared assets
    let tile_mesh = meshes.add(Cuboid::new(TILE_WIDTH, 0.3, TILE_LENGTH));
    let mat_a = materials.add(StandardMaterial {
        base_color: Color::srgb(0.58, 0.55, 0.50),
        perceptual_roughness: 0.9,
        ..default()
    });
    let mat_b = materials.add(StandardMaterial {
        base_color: Color::srgb(0.42, 0.39, 0.36),
        perceptual_roughness: 0.9,
        ..default()
    });
    commands.insert_resource(TileAssets { mesh: tile_mesh, mat_a, mat_b });
    commands.insert_resource(TileManager {
        frontier_z: TILE_LENGTH / 2.0,
        spawned: std::collections::VecDeque::new(),
        count: 0,
    });

    // Player
    commands.spawn((
        Mesh3d(meshes.add(Capsule3d::new(0.28, 0.7))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb(0.2, 0.3, 0.7),
            ..default()
        })),
        Transform::from_xyz(0.0, 0.9, 0.0),
        Player { lateral: 0.0, target_lateral: 0.0, dash_until_z: f32::INFINITY },
    ));

    // Main 3D camera with SMAA
    commands.spawn((
        Camera3d::default(),
        Msaa::Off,
        Smaa { preset: SmaaPreset::Medium },
        Transform::from_translation(CAM_OFFSET_DEFAULT).looking_at(Vec3::ZERO, Vec3::Y),
        CameraMarker,
    ));

    commands.insert_resource(GateManager {
        next_z: -50.0,
        live: std::collections::VecDeque::new(),
    });
}

// ─── Gate spawning ────────────────────────────────────────────────────────────

fn spawn_gate(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    text_mat: Handle<StandardMaterial>,
    gate_z: f32,
    question: &str,
    answer_l: &str,
    answer_r: &str,
    correct_is_left: bool,
) -> Vec<Entity> {
    let mut entities = Vec::new();
    let correct_side = if correct_is_left { -1.0 } else { 1.0 };

    // ── Center post ──
    entities.push(commands.spawn((
        Mesh3d(meshes.add(Cylinder::new(0.18, 4.8))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb(0.85, 0.15, 0.05),
            perceptual_roughness: 0.5,
            ..default()
        })),
        Transform::from_xyz(0.0, 2.4, gate_z),
        GatePost { gate_z, correct_side, passed: false },
    )).id());

    // ── Question crossbeam (mesh + text) ──
    entities.push(commands.spawn((
        Mesh3d(meshes.add(Cuboid::new(5.0, 1.5, 0.12))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb(0.95, 0.90, 0.75),
            perceptual_roughness: 0.6,
            ..default()
        })),
        Transform::from_xyz(0.0, 5.8, gate_z),
        Billboard,
    )).id());
    entities.push(commands.spawn((
        Text3d::new(question.to_uppercase()),
        Text3dStyling {
            size: 72.0,
            color: Srgba::new(0.12, 0.06, 0.01, 1.0),
            stroke: NonZero::new(4),
            stroke_color: Srgba::new(1.0, 0.95, 0.8, 1.0),
            world_scale: Some(Vec2::splat(0.6)),
            ..default()
        },
        Mesh3d::default(),
        MeshMaterial3d(text_mat.clone()),
        Transform::from_xyz(0.0, 5.8, gate_z + 0.1),
        Billboard,
    )).id());

    let sign_mesh = meshes.add(Cuboid::new(3.0, 2.5, 0.12));

    // ── Left sign — gold background + hiragana ──
    entities.push(commands.spawn((
        Mesh3d(sign_mesh.clone()),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb(0.9, 0.72, 0.08),
            perceptual_roughness: 0.4,
            ..default()
        })),
        Transform::from_xyz(-1.8, 3.2, gate_z),
        Billboard,
        GateSign::Left,
        SignSlide { gate_z, dir: -1.0 },
    )).id());
    entities.push(commands.spawn((
        Text3d::new(answer_l),
        Text3dStyling {
            size: 96.0,
            color: Srgba::new(0.08, 0.04, 0.0, 1.0),
            stroke: NonZero::new(5),
            stroke_color: Srgba::new(1.0, 0.88, 0.4, 1.0),
            world_scale: Some(Vec2::splat(1.0)),
            ..default()
        },
        Mesh3d::default(),
        MeshMaterial3d(text_mat.clone()),
        Transform::from_xyz(-1.8, 3.2, gate_z + 0.1),
        Billboard,
        SignSlide { gate_z, dir: -1.0 },
    )).id());

    // ── Right sign — blue background + hiragana ──
    entities.push(commands.spawn((
        Mesh3d(sign_mesh),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb(0.1, 0.4, 0.85),
            perceptual_roughness: 0.4,
            ..default()
        })),
        Transform::from_xyz(1.8, 3.2, gate_z),
        Billboard,
        GateSign::Right,
        SignSlide { gate_z, dir: 1.0 },
    )).id());
    entities.push(commands.spawn((
        Text3d::new(answer_r),
        Text3dStyling {
            size: 96.0,
            color: Srgba::new(0.05, 0.05, 0.1, 1.0),
            world_scale: Some(Vec2::splat(1.0)),
            ..default()
        },
        Mesh3d::default(),
        MeshMaterial3d(text_mat.clone()),
        Transform::from_xyz(1.8, 3.2, gate_z + 0.1),
        Billboard,
        SignSlide { gate_z, dir: 1.0 },
    )).id());

    entities
}

// ─── Systems ──────────────────────────────────────────────────────────────────

fn input_system(
    time: Res<Time>,
    touches: Res<Touches>,
    mouse: Res<ButtonInput<MouseButton>>,
    mut drag: ResMut<DragState>,
    mut player_q: Query<(&Transform, &mut Player)>,
    gate_manager: Res<GateManager>,
    window_q: Query<&Window, With<bevy::window::PrimaryWindow>>,
) {
    let Ok(window) = window_q.single() else { return };
    let Ok((player_t, mut player)) = player_q.single_mut() else { return };
    let half_w = window.width() * 0.5;
    let mut touch_handled = false;

    for touch in touches.iter_just_pressed() {
        touch_handled = true;
        drag.active = true;
        drag.start_x = touch.position().x - half_w;
        drag.current_x = drag.start_x;
        drag.start_y = touch.position().y;
        drag.current_y = drag.start_y;
        drag.prev_y = drag.start_y;
        drag.dashed = false;
        drag.touch_id = Some(touch.id());
    }
    for touch in touches.iter() {
        if drag.touch_id == Some(touch.id()) {
            touch_handled = true;
            drag.prev_y = drag.current_y;
            drag.current_x = touch.position().x - half_w;
            drag.current_y = touch.position().y;
        }
    }
    for touch in touches.iter_just_released() {
        if drag.touch_id == Some(touch.id()) {
            touch_handled = true;
            drag.active = false;
            drag.dashed = false;
            drag.touch_id = None;
        }
    }

    if !touch_handled {
        if mouse.just_pressed(MouseButton::Left)
            && let Some(pos) = window.cursor_position()
        {
            drag.active = true;
            drag.start_x = pos.x - half_w;
            drag.current_x = drag.start_x;
            drag.start_y = pos.y;
            drag.current_y = drag.start_y;
            drag.prev_y = drag.start_y;
            drag.dashed = false;
        }
        if mouse.pressed(MouseButton::Left)
            && let Some(pos) = window.cursor_position()
        {
            drag.prev_y = drag.current_y;
            drag.current_x = pos.x - half_w;
            drag.current_y = pos.y;
        }
        if mouse.just_released(MouseButton::Left) {
            drag.active = false;
            drag.dashed = false;
        }
    }

    // Allow re-swiping once the previous dash has finished
    if player.dash_until_z == f32::INFINITY && drag.dashed {
        drag.dashed = false;
        drag.start_y = drag.current_y; // reset origin so next swipe is measured fresh
        drag.start_x = drag.current_x;
    }

    // Swipe-up detection: upward velocity exceeds threshold
    if drag.active && !drag.dashed {
        let dt = time.delta_secs().max(0.001);
        let vel_y = (drag.prev_y - drag.current_y) / dt; // positive = upward
        if vel_y > SWIPE_UP_VELOCITY {
            drag.dashed = true;
            // Dash to the nearest gate ahead of the player
            let pz = player_t.translation.z;
            let target_z = gate_manager.live.iter()
                .map(|(z, _)| *z)
                .filter(|&z| z < pz)
                .fold(f32::NEG_INFINITY, f32::max);
            player.dash_until_z = if target_z == f32::NEG_INFINITY {
                gate_manager.next_z // fallback: sprint to next unspawned gate
            } else {
                target_z
            };
        }
    }
}

fn steer_system(
    drag: Res<DragState>,
    window_q: Query<&Window, With<bevy::window::PrimaryWindow>>,
    mut player_q: Query<&mut Player>,
) {
    let Ok(window) = window_q.single() else { return };
    let Ok(mut player) = player_q.single_mut() else { return };

    if drag.active {
        let delta = drag.current_x - drag.start_x;
        let normalized = (delta / (window.width() * 0.5)).clamp(-1.0, 1.0);
        player.target_lateral = normalized * MAX_LATERAL;
    } else {
        player.target_lateral = 0.0;
    }
}

fn move_system(time: Res<Time>, mut player_q: Query<(&mut Player, &mut Transform)>) {
    let Ok((mut player, mut transform)) = player_q.single_mut() else { return };
    let dt = time.delta_secs();
    player.lateral = player.lateral.lerp(player.target_lateral, STEER_SMOOTHING * dt);
    let dashing = transform.translation.z > player.dash_until_z;
    if !dashing {
        player.dash_until_z = f32::INFINITY;
    }
    let speed = if dashing { PLAYER_SPEED * DASH_SPEED_MULT } else { PLAYER_SPEED };
    transform.translation.z -= speed * dt;
    transform.translation.x = player.lateral;
}

fn camera_follow_system(
    time: Res<Time>,
    settings: Res<CameraSettings>,
    player_q: Query<&Transform, (With<Player>, Without<CameraMarker>)>,
    mut cam_q: Query<&mut Transform, (With<CameraMarker>, Without<Player>)>,
) {
    let Ok(player_t) = player_q.single() else { return };
    let Ok(mut cam_t) = cam_q.single_mut() else { return };
    let offset = Vec3::new(0.0, settings.height, settings.distance);
    let ideal = player_t.translation + offset;
    cam_t.translation = cam_t.translation.lerp(ideal, settings.lerp * time.delta_secs());
    cam_t.look_at(player_t.translation + Vec3::Y * 0.5, Vec3::Y);
}

fn camera_settings_ui(
    mut contexts: EguiContexts,
    mut settings: ResMut<CameraSettings>,
    flash: Res<FlashState>,
) -> Result {
    egui::Window::new("Camera")
        .default_open(false)
        .show(contexts.ctx_mut()?, |ui| {
            ui.add(egui::Slider::new(&mut settings.height, 0.5..=20.0).text("height"));
            ui.add(egui::Slider::new(&mut settings.distance, 1.0..=30.0).text("distance"));
            ui.add(egui::Slider::new(&mut settings.lerp, 0.5..=20.0).text("lerp speed"));
        });

    if flash.timer > 0.0 && !flash.correct {
        let alpha = (flash.timer * 2.5).min(1.0) * 0.35;
        let color = egui::Color32::from_rgba_unmultiplied(220, 40, 40, (alpha * 255.0) as u8);
        let ctx = contexts.ctx_mut()?;
        let screen = ctx.viewport_rect();
        egui::Area::new(egui::Id::new("flash"))
            .fixed_pos(screen.min)
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                ui.painter().rect_filled(screen, 0.0, color);
            });
    }

    Ok(())
}

fn manage_tiles_system(
    mut commands: Commands,
    mut manager: ResMut<TileManager>,
    assets: Res<TileAssets>,
    player_q: Query<&Transform, With<Player>>,
) {
    let Ok(player_t) = player_q.single() else { return };
    let pz = player_t.translation.z;

    while manager.frontier_z > pz - TILES_AHEAD as f32 * TILE_LENGTH {
        let center_z = manager.frontier_z;
        let mat = if manager.count.is_multiple_of(2) {
            assets.mat_a.clone()
        } else {
            assets.mat_b.clone()
        };
        let entity = commands
            .spawn((
                Tile,
                Mesh3d(assets.mesh.clone()),
                MeshMaterial3d(mat),
                Transform::from_xyz(0.0, -0.15, center_z),
            ))
            .id();
        manager.spawned.push_back((center_z, entity));
        manager.frontier_z -= TILE_LENGTH;
        manager.count += 1;
    }

    while let Some(&(center_z, entity)) = manager.spawned.front() {
        if center_z > pz + TILES_BEHIND as f32 * TILE_LENGTH {
            commands.entity(entity).despawn();
            manager.spawned.pop_front();
        } else {
            break;
        }
    }
}

fn manage_gates_system(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut manager: ResMut<GateManager>,
    mut deck: ResMut<Deck>,
    text_mat: Res<TextMat>,
    player_q: Query<&Transform, With<Player>>,
) {
    let Ok(player_t) = player_q.single() else { return };
    let pz = player_t.translation.z;

    while manager.next_z > pz - GATES_AHEAD as f32 * GATE_SPACING {
        let z = manager.next_z;
        let (question, correct, distractor) = deck.pick();
        let correct_is_left = deck.rng.gen_bool(0.5);
        let (answer_l, answer_r) = if correct_is_left {
            (correct, distractor)
        } else {
            (distractor, correct)
        };
        let entities = spawn_gate(
            &mut commands, &mut meshes, &mut materials,
            text_mat.0.clone(), z, question, answer_l, answer_r, correct_is_left,
        );
        manager.live.push_back((z, entities));
        manager.next_z -= GATE_SPACING;
    }

    while let Some((gate_z, entities)) = manager.live.front() {
        if *gate_z > pz + GATE_SPACING {
            for &e in entities {
                commands.entity(e).despawn();
            }
            manager.live.pop_front();
        } else {
            break;
        }
    }
}

fn billboard_system(
    cam_q: Query<&Transform, With<CameraMarker>>,
    mut sign_q: Query<&mut Transform, (With<Billboard>, Without<CameraMarker>)>,
) {
    let Ok(cam_t) = cam_q.single() else { return };
    for mut transform in sign_q.iter_mut() {
        let to_cam = cam_t.translation - transform.translation;
        let yaw = f32::atan2(to_cam.x, to_cam.z);
        transform.rotation = Quat::from_rotation_y(yaw);
    }
}

fn screenshot_system(mut commands: Commands, keys: Res<ButtonInput<KeyCode>>) {
    if keys.just_pressed(KeyCode::F12) {
        let path = "/tmp/nihongo_screenshot.png";
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(path));
        info!("Screenshot saved to {path}");
    }
}

fn gate_check_system(
    time: Res<Time>,
    player_q: Query<(&Transform, &Player)>,
    mut post_q: Query<&mut GatePost>,
    mut flash: ResMut<FlashState>,
) {
    let Ok((player_t, player)) = player_q.single() else { return };
    let pz = player_t.translation.z;

    for mut post in post_q.iter_mut() {
        if !post.passed && pz < post.gate_z - 0.5 {
            post.passed = true;
            let player_side = if player.lateral <= 0.0 { -1.0 } else { 1.0 };
            flash.correct = player_side == post.correct_side;
            flash.timer = 0.6;
        }
    }

    if flash.timer > 0.0 {
        flash.timer -= time.delta_secs();
    }
}

fn slide_signs_system(
    time: Res<Time>,
    player_q: Query<&Transform, With<Player>>,
    mut sign_q: Query<(&SignSlide, &mut Transform), Without<Player>>,
) {
    let Ok(player_t) = player_q.single() else { return };
    let pz = player_t.translation.z;
    for (slide, mut transform) in sign_q.iter_mut() {
        if pz < slide.gate_z {
            let target_x = slide.dir * 10.0;
            transform.translation.x =
                transform.translation.x.lerp(target_x, 3.5 * time.delta_secs());
        }
    }
}

fn auto_screenshot_system(mut commands: Commands, mut frame: Local<u32>) {
    *frame += 1;
    if *frame == 30 {
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk("/tmp/nihongo_screenshot.png"));
        info!("Auto-screenshot taken (frame 30)");
    }
}
