use bevy::anti_alias::smaa::{Smaa, SmaaPreset};
use bevy::render::alpha::AlphaMode;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};
use bevy::{asset::AssetMetaCheck, prelude::*};
use bevy_egui::{EguiContexts, EguiPlugin, EguiPrimaryContextPass, egui};
use bevy_rich_text3d::{LoadFonts, Text3d, Text3dPlugin, Text3dStyling, TextAtlas};
use rand::{Rng, SeedableRng, rngs::SmallRng};
use std::collections::VecDeque;
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

const STEER_SMOOTHING: f32 = 8.0;
const MAX_LATERAL: f32 = 2.0;
const DASH_SPEED_MULT: f32 = 6.8;
const SWIPE_UP_VELOCITY: f32 = 800.0; // pixels per second upward
const CAM_OFFSET_DEFAULT: Vec3 = Vec3::new(0.0, 2.5, 30.0);

#[derive(Resource)]
struct CameraSettings {
    height: f32,
    distance: f32,
    lerp: f32,
    fov_degrees: f32,
    gate_scale: f32,
    player_speed: f32,
    gate_spacing: f32,
}

const TILE_LENGTH: f32 = 20.0;
const TILE_WIDTH: f32 = 5.0;
const TILES_AHEAD: i32 = 6;
const TILES_BEHIND: i32 = 3;

const GATES_AHEAD: u32 = 2;
const SIGN_X_OFFSET: f32 = 1.8;
const SIGN_SLIDE_TARGET: f32 = 10.0;
const SIGN_SLIDE_SPEED: f32 = 3.5;

// ─── N5 vocabulary ────────────────────────────────────────────────────────────
// (hiragana reading, kanji/kana display)
const N5_WORDS: &[(&str, &str)] = &[
    ("みず", "水"),
    ("ひ", "火"),
    ("やま", "山"),
    ("かわ", "川"),
    ("き", "木"),
    ("はな", "花"),
    ("いぬ", "犬"),
    ("ねこ", "猫"),
    ("さかな", "魚"),
    ("とり", "鳥"),
    ("たべる", "食べる"),
    ("のむ", "飲む"),
    ("いく", "行く"),
    ("くる", "来る"),
    ("みる", "見る"),
    ("きく", "聞く"),
    ("はなす", "話す"),
    ("かく", "書く"),
    ("よむ", "読む"),
    ("かう", "買う"),
    ("おおきい", "大きい"),
    ("ちいさい", "小さい"),
    ("あたらしい", "新しい"),
    ("ふるい", "古い"),
    ("たかい", "高い"),
    ("やすい", "安い"),
    ("しろい", "白い"),
    ("くろい", "黒い"),
    ("あかい", "赤い"),
    ("あおい", "青い"),
];

// ─── Components & Resources ───────────────────────────────────────────────────

#[derive(Component)]
struct Player {
    lateral: f32,
    target_lateral: f32,
    dashing: bool,
}

#[derive(Component)]
struct CameraMarker;

#[derive(Component)]
struct Tile;

/// Rotates around Y each frame to face the camera.
#[derive(Component)]
struct Billboard;

/// Slides the sign outward along X once the player passes gate_z.
#[derive(Component)]
struct SignSlide {
    gate_z: f32,
    dir: f32, // -1.0 = left, +1.0 = right
}

/// Drops the post downward after the player passes.
#[derive(Component)]
struct PostDrop {
    gate_z: f32,
}

/// Rises the crossbeam upward after the player passes.
#[derive(Component)]
struct CrossbeamRise {
    gate_z: f32,
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
    spawned: VecDeque<(f32, Entity)>,
    count: u32,
}

#[derive(Resource)]
struct GateManager {
    next_z: f32,
    live: VecDeque<(f32, Vec<Entity>)>,
}

#[derive(Resource, Default)]
struct DragState {
    active: bool,
    start_x: f32,
    current_x: f32,
    current_y: f32,
    prev_y: f32,
    base_lateral: f32,    // lateral offset at the moment the drag origin was set
    dash_triggered: bool, // swipe-up consumed for this gesture; cleared when dash resolves
    flick_dash: bool,     // finger released during dash — snap to center when gate passes
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
        if d >= q {
            d += 1;
        }
        // question = kanji display, answers = hiragana readings
        (N5_WORDS[q].1, N5_WORDS[q].0, N5_WORDS[d].0)
    }
}

// ─── Main ─────────────────────────────────────────────────────────────────────

fn main() {
    let (w, h) = measure_screen();

    let mut app = App::new();
    app.add_plugins(
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
        height: 2.5,
        distance: 30.0,
        lerp: 6.0,
        fov_degrees: 15.0,
        gate_scale: 0.65,
        player_speed: 25.0,
        gate_spacing: 125.0,
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
            apply_fov_system,
            billboard_system,
            slide_signs_system,
            drop_posts_system,
            rise_crossbeam_system,
            #[cfg(not(target_arch = "wasm32"))]
            screenshot_system,
        )
            .chain(),
    )
    .add_systems(
        EguiPrimaryContextPass,
        (camera_settings_ui, flash_overlay_ui),
    );

    #[cfg(not(target_arch = "wasm32"))]
    if std::env::args().any(|a| a == "--screenshot") {
        app.add_systems(Update, auto_screenshot_system);
    }

    app.run();
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
    commands.insert_resource(Deck {
        rng: SmallRng::seed_from_u64(42),
    });
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
    commands.insert_resource(TileAssets {
        mesh: tile_mesh,
        mat_a,
        mat_b,
    });
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
        Player {
            lateral: 0.0,
            target_lateral: 0.0,
            dashing: false,
        },
    ));

    // Main 3D camera with SMAA
    commands.spawn((
        Camera3d::default(),
        Msaa::Off,
        Smaa {
            preset: SmaaPreset::Medium,
        },
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
    s: f32,
) -> Vec<Entity> {
    let mut entities = Vec::new();
    let correct_side = if correct_is_left { -1.0 } else { 1.0 };

    // ── Center post ──
    entities.push(
        commands
            .spawn((
                Mesh3d(meshes.add(Cylinder::new(0.18 * s, 4.8 * s))),
                MeshMaterial3d(materials.add(StandardMaterial {
                    base_color: Color::srgb(0.85, 0.15, 0.05),
                    perceptual_roughness: 0.5,
                    ..default()
                })),
                Transform::from_xyz(0.0, 2.4 * s, gate_z),
                GatePost {
                    gate_z,
                    correct_side,
                    passed: false,
                },
                PostDrop { gate_z },
            ))
            .id(),
    );

    // ── Question crossbeam (mesh + text) ──
    entities.push(
        commands
            .spawn((
                Mesh3d(meshes.add(Cuboid::new(5.0 * s, 1.5 * s, 0.12))),
                MeshMaterial3d(materials.add(StandardMaterial {
                    base_color: Color::srgb(0.95, 0.90, 0.75),
                    perceptual_roughness: 0.6,
                    ..default()
                })),
                Transform::from_xyz(0.0, 5.8 * s, gate_z),
                Billboard,
                CrossbeamRise { gate_z },
            ))
            .id(),
    );
    entities.push(
        commands
            .spawn((
                Text3d::new(question),
                Text3dStyling {
                    size: 72.0,
                    color: Srgba::new(0.12, 0.06, 0.01, 1.0),
                    stroke: NonZero::new(4),
                    stroke_color: Srgba::new(1.0, 0.95, 0.8, 1.0),
                    world_scale: Some(Vec2::splat(0.6 * s)),
                    ..default()
                },
                Mesh3d::default(),
                MeshMaterial3d(text_mat.clone()),
                Transform::from_xyz(0.0, 5.8 * s, gate_z + 0.1),
                Billboard,
                CrossbeamRise { gate_z },
            ))
            .id(),
    );

    let sign_mesh = meshes.add(Cuboid::new(3.0 * s, 2.5 * s, 0.12));
    let [l0, l1] = spawn_sign(
        commands,
        &sign_mesh,
        materials,
        text_mat.clone(),
        gate_z,
        -SIGN_X_OFFSET * s,
        Color::srgb(0.9, 0.72, 0.08),
        answer_l,
        s,
    );
    let [r0, r1] = spawn_sign(
        commands,
        &sign_mesh,
        materials,
        text_mat,
        gate_z,
        SIGN_X_OFFSET * s,
        Color::srgb(0.1, 0.40, 0.85),
        answer_r,
        s,
    );
    entities.extend([l0, l1, r0, r1]);

    entities
}

fn spawn_sign(
    commands: &mut Commands,
    sign_mesh: &Handle<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    text_mat: Handle<StandardMaterial>,
    gate_z: f32,
    x: f32,
    bg_color: Color,
    text: &str,
    s: f32,
) -> [Entity; 2] {
    let dir = x.signum();
    let bg = commands
        .spawn((
            Mesh3d(sign_mesh.clone()),
            MeshMaterial3d(materials.add(StandardMaterial {
                base_color: bg_color,
                perceptual_roughness: 0.4,
                ..default()
            })),
            Transform::from_xyz(x, 3.2 * s, gate_z),
            Billboard,
            SignSlide { gate_z, dir },
        ))
        .id();
    let label = commands
        .spawn((
            Text3d::new(text),
            Text3dStyling {
                size: 96.0,
                color: Srgba::new(0.05, 0.05, 0.1, 1.0),
                world_scale: Some(Vec2::splat(1.0 * s)),
                ..default()
            },
            Mesh3d::default(),
            MeshMaterial3d(text_mat),
            Transform::from_xyz(x, 3.2 * s, gate_z + 0.1),
            Billboard,
            SignSlide { gate_z, dir },
        ))
        .id();
    [bg, label]
}

// ─── Systems ──────────────────────────────────────────────────────────────────

fn input_system(
    time: Res<Time>,
    touches: Res<Touches>,
    mouse: Res<ButtonInput<MouseButton>>,
    mut drag: ResMut<DragState>,
    mut player_q: Query<&mut Player>,
    window_q: Query<&Window, With<bevy::window::PrimaryWindow>>,
) {
    let Ok(window) = window_q.single() else {
        return;
    };
    let Ok(mut player) = player_q.single_mut() else {
        return;
    };
    let half_w = window.width() * 0.5;
    let mut touch_handled = false;

    for touch in touches.iter_just_pressed() {
        touch_handled = true;
        drag.active = true;
        drag.start_x = touch.position().x - half_w;
        drag.current_x = drag.start_x;
        drag.current_y = touch.position().y;
        drag.prev_y = drag.current_y;
        drag.base_lateral = player.lateral;
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
            if drag.dash_triggered {
                drag.flick_dash = true;
            }
            drag.dash_triggered = false;
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
            drag.current_y = pos.y;
            drag.prev_y = drag.current_y;
            drag.base_lateral = player.lateral;
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
            if drag.dash_triggered {
                drag.flick_dash = true;
            }
            drag.dash_triggered = false;
        }
    }

    // Hold dash ended: hold lane and allow re-swiping
    if !player.dashing && drag.dash_triggered {
        drag.dash_triggered = false;
        drag.start_x = drag.current_x;
        drag.base_lateral = player.lateral;
    // Flick dash ended: snap to center
    } else if !player.dashing && drag.flick_dash {
        drag.flick_dash = false;
        player.target_lateral = 0.0;
    }

    // Swipe-up detection: upward velocity exceeds threshold
    if drag.active && !drag.dash_triggered {
        let dt = time.delta_secs().max(0.001);
        let vel_y = (drag.prev_y - drag.current_y) / dt; // positive = upward
        if vel_y > SWIPE_UP_VELOCITY {
            drag.dash_triggered = true;
            player.dashing = true;
        }
    }
}

fn steer_system(
    drag: Res<DragState>,
    window_q: Query<&Window, With<bevy::window::PrimaryWindow>>,
    mut player_q: Query<&mut Player>,
) {
    let Ok(window) = window_q.single() else {
        return;
    };
    let Ok(mut player) = player_q.single_mut() else {
        return;
    };

    if drag.active {
        let delta = drag.current_x - drag.start_x;
        let normalized = (delta / (window.width() * 0.5)).clamp(-1.0, 1.0);
        player.target_lateral =
            (drag.base_lateral + normalized * MAX_LATERAL).clamp(-MAX_LATERAL, MAX_LATERAL);
    }
}

fn move_system(
    time: Res<Time>,
    settings: Res<CameraSettings>,
    mut player_q: Query<(&mut Player, &mut Transform)>,
) {
    let Ok((mut player, mut transform)) = player_q.single_mut() else {
        return;
    };
    let dt = time.delta_secs();
    player.lateral = player
        .lateral
        .lerp(player.target_lateral, STEER_SMOOTHING * dt);
    let speed = if player.dashing {
        settings.player_speed * DASH_SPEED_MULT
    } else {
        settings.player_speed
    };
    transform.translation.z -= speed * dt;
    transform.translation.x = player.lateral;
}

fn camera_follow_system(
    time: Res<Time>,
    settings: Res<CameraSettings>,
    player_q: Query<&Transform, (With<Player>, Without<CameraMarker>)>,
    mut cam_q: Query<&mut Transform, (With<CameraMarker>, Without<Player>)>,
) {
    let Ok(player_t) = player_q.single() else {
        return;
    };
    let Ok(mut cam_t) = cam_q.single_mut() else {
        return;
    };
    let offset = Vec3::new(0.0, settings.height, settings.distance);
    let ideal = player_t.translation + offset;
    cam_t.translation = cam_t
        .translation
        .lerp(ideal, settings.lerp * time.delta_secs());
    cam_t.look_at(player_t.translation + Vec3::Y * 0.5, Vec3::Y);
}

fn camera_settings_ui(mut contexts: EguiContexts, mut settings: ResMut<CameraSettings>) -> Result {
    egui::Window::new("Camera")
        .default_open(false)
        .show(contexts.ctx_mut()?, |ui| {
            ui.add(egui::Slider::new(&mut settings.height, 0.5..=20.0).text("height"));
            ui.add(egui::Slider::new(&mut settings.distance, 1.0..=200.0).text("distance"));
            ui.add(egui::Slider::new(&mut settings.lerp, 0.5..=20.0).text("lerp speed"));
            ui.add(egui::Slider::new(&mut settings.fov_degrees, 10.0..=90.0).text("fov"));
            ui.add(egui::Slider::new(&mut settings.gate_scale, 0.3..=3.0).text("gate size"));
            ui.add(egui::Slider::new(&mut settings.player_speed, 1.0..=50.0).text("speed"));
            ui.add(egui::Slider::new(&mut settings.gate_spacing, 10.0..=200.0).text("gate spacing"));
        });
    Ok(())
}

fn apply_fov_system(
    settings: Res<CameraSettings>,
    mut cam_q: Query<&mut Projection, With<CameraMarker>>,
) {
    if !settings.is_changed() { return; }
    let Ok(mut proj) = cam_q.single_mut() else { return };
    if let Projection::Perspective(ref mut p) = *proj {
        p.fov = settings.fov_degrees.to_radians();
    }
}

fn flash_overlay_ui(mut contexts: EguiContexts, flash: Res<FlashState>) -> Result {
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
    let Ok(player_t) = player_q.single() else {
        return;
    };
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
    settings: Res<CameraSettings>,
    player_q: Query<&Transform, With<Player>>,
) {
    let Ok(player_t) = player_q.single() else {
        return;
    };
    let pz = player_t.translation.z;

    while manager.next_z > pz - GATES_AHEAD as f32 * settings.gate_spacing {
        let z = manager.next_z;
        let (question, correct, distractor) = deck.pick();
        let correct_is_left = deck.rng.gen_bool(0.5);
        let (answer_l, answer_r) = if correct_is_left {
            (correct, distractor)
        } else {
            (distractor, correct)
        };
        let entities = spawn_gate(
            &mut commands,
            &mut meshes,
            &mut materials,
            text_mat.0.clone(),
            z,
            question,
            answer_l,
            answer_r,
            correct_is_left,
            settings.gate_scale,
        );
        manager.live.push_back((z, entities));
        manager.next_z -= settings.gate_spacing;
    }

    while let Some((gate_z, entities)) = manager.live.front() {
        if *gate_z > pz + settings.gate_spacing {
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

#[cfg(not(target_arch = "wasm32"))]
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
    mut player_q: Query<(&Transform, &mut Player)>,
    mut post_q: Query<&mut GatePost>,
    mut flash: ResMut<FlashState>,
) {
    let Ok((player_t, mut player)) = player_q.single_mut() else {
        return;
    };
    let pz = player_t.translation.z;

    for mut post in post_q.iter_mut() {
        if !post.passed && pz < post.gate_z - 0.5 {
            post.passed = true;
            player.dashing = false;
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
    let Ok(player_t) = player_q.single() else {
        return;
    };
    let pz = player_t.translation.z;
    for (slide, mut transform) in sign_q.iter_mut() {
        if pz < slide.gate_z {
            let target_x = slide.dir * SIGN_SLIDE_TARGET;
            transform.translation.x = transform
                .translation
                .x
                .lerp(target_x, SIGN_SLIDE_SPEED * time.delta_secs());
        }
    }
}

fn drop_posts_system(
    time: Res<Time>,
    player_q: Query<&Transform, With<Player>>,
    mut post_q: Query<(&PostDrop, &mut Transform), Without<Player>>,
) {
    let Ok(player_t) = player_q.single() else {
        return;
    };
    let pz = player_t.translation.z;
    for (post, mut transform) in post_q.iter_mut() {
        if pz < post.gate_z {
            transform.translation.y = transform
                .translation
                .y
                .lerp(-5.0, SIGN_SLIDE_SPEED * time.delta_secs());
        }
    }
}

fn rise_crossbeam_system(
    time: Res<Time>,
    player_q: Query<&Transform, With<Player>>,
    mut beam_q: Query<(&CrossbeamRise, &mut Transform), Without<Player>>,
) {
    let Ok(player_t) = player_q.single() else {
        return;
    };
    let pz = player_t.translation.z;
    for (beam, mut transform) in beam_q.iter_mut() {
        if pz < beam.gate_z {
            transform.translation.y = transform
                .translation
                .y
                .lerp(20.0, SIGN_SLIDE_SPEED * time.delta_secs());
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn auto_screenshot_system(mut commands: Commands, mut frame: Local<u32>) {
    *frame += 1;
    if *frame == 30 {
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk("/tmp/nihongo_screenshot.png"));
        info!("Auto-screenshot taken (frame 30)");
    }
}
