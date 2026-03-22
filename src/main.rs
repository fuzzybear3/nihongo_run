use bevy::{asset::AssetMetaCheck, prelude::*};
use bevy::camera::{ImageRenderTarget, RenderTarget};
use bevy::asset::RenderAssetUsages;
use bevy::render::render_resource::{
    Extent3d, TextureDimension, TextureFormat, TextureUsages,
};
use bevy_egui::{EguiContexts, EguiPlugin, EguiPrimaryContextPass, egui};
use rand::{rngs::SmallRng, Rng, SeedableRng};

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

const GATE_SPACING: f32 = 80.0;
const GATES_AHEAD: u32 = 2;

// Sign texture resolution
const SIGN_TEX_SIZE: u32 = 512;

// ─── N5 vocabulary ────────────────────────────────────────────────────────────
// (hiragana, english)
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
}

#[derive(Component)]
struct CameraMarker;

#[derive(Component)]
struct Tile;

/// Rotates around Y each frame to face the camera.
#[derive(Component)]
struct Billboard;

/// Marks the left or right answer sign on a decision gate.
#[derive(Component)]
enum GateSign {
    Left,
    Right,
}

/// Holds the offscreen UI camera entity so it can be despawned with the gate.
#[derive(Component)]
struct GateUiCamera(Entity);

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
    touch_id: Option<u64>,
}

#[derive(Resource)]
struct Deck {
    rng: SmallRng,
}

impl Deck {
    /// Returns (english_question, correct_japanese, distractor_japanese).
    fn pick(&mut self) -> (&'static str, &'static str, &'static str) {
        let q = self.rng.gen_range(0..N5_WORDS.len());
        let mut d = self.rng.gen_range(0..N5_WORDS.len() - 1);
        if d >= q {
            d += 1;
        }
        (N5_WORDS[q].1, N5_WORDS[q].0, N5_WORDS[d].0)
    }
}

/// Font handle loaded from assets.
#[derive(Resource)]
struct JpFont(Handle<Font>);

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
                billboard_system,
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
    asset_server: Res<AssetServer>,
) {
    commands.insert_resource(Deck { rng: SmallRng::seed_from_u64(42) });

    let font: Handle<Font> = asset_server.load("fonts/TakaoPGothic.ttf");
    commands.insert_resource(JpFont(font));

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
        Player { lateral: 0.0, target_lateral: 0.0 },
    ));

    // Main 3D camera
    commands.spawn((
        Camera3d::default(),
        Msaa::Off,
        Transform::from_translation(CAM_OFFSET_DEFAULT).looking_at(Vec3::ZERO, Vec3::Y),
        CameraMarker,
    ));

    commands.insert_resource(GateManager {
        next_z: -50.0,
        live: std::collections::VecDeque::new(),
    });
}

// ─── Gate spawning ────────────────────────────────────────────────────────────

/// Creates an offscreen render target image, spawns a Camera2d targeting it,
/// spawns a UI text node attached to that camera, and returns the image handle
/// plus the camera entity (for later cleanup).
fn make_sign_texture(
    commands: &mut Commands,
    images: &mut Assets<Image>,
    font: Handle<Font>,
    text: &str,
    bg_color: Color,
    text_color: Color,
) -> (Handle<Image>, Entity) {
    // 1. Create blank render-target image
    let size = Extent3d {
        width: SIGN_TEX_SIZE,
        height: SIGN_TEX_SIZE,
        depth_or_array_layers: 1,
    };
    let mut image = Image::new_fill(
        size,
        TextureDimension::D2,
        &[0, 0, 0, 255],
        TextureFormat::Bgra8UnormSrgb,
        RenderAssetUsages::default(),
    );
    image.texture_descriptor.usage =
        TextureUsages::TEXTURE_BINDING
        | TextureUsages::COPY_DST
        | TextureUsages::RENDER_ATTACHMENT;
    let image_handle = images.add(image);

    // 2. Spawn offscreen Camera2d targeting the image
    let cam_entity = commands.spawn((
        Camera2d,
        Camera {
            order: -1,
            clear_color: ClearColorConfig::Custom(bg_color),
            ..default()
        },
        RenderTarget::Image(ImageRenderTarget {
            handle: image_handle.clone(),
            scale_factor: 1.0,
        }),
    )).id();

    // 3. Spawn UI text node attached to that camera
    commands.spawn((
        Node {
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            ..default()
        },
        bevy::ui::UiTargetCamera(cam_entity),
    )).with_children(|parent| {
        parent.spawn((
            Text::new(text),
            TextFont {
                font,
                font_size: 120.0,
                ..default()
            },
            TextColor(text_color),
        ));
    });

    (image_handle, cam_entity)
}

fn spawn_gate(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    images: &mut Assets<Image>,
    font: Handle<Font>,
    gate_z: f32,
    question: &str,
    answer_l: &str,
    answer_r: &str,
) -> Vec<Entity> {
    let mut entities = Vec::new();

    // ── Center post (torii vermillion) ──
    entities.push(commands.spawn((
        Mesh3d(meshes.add(Cylinder::new(0.18, 4.8))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb(0.85, 0.15, 0.05),
            perceptual_roughness: 0.5,
            ..default()
        })),
        Transform::from_xyz(0.0, 2.4, gate_z),
    )).id());

    // ── Question crossbeam with text ──
    let (q_img, q_cam) = make_sign_texture(
        commands, images,
        font.clone(),
        &question.to_uppercase(),
        Color::srgb(0.95, 0.90, 0.75),
        Color::srgb(0.15, 0.08, 0.02),
    );
    let crossbeam = commands.spawn((
        Mesh3d(meshes.add(Cuboid::new(5.0, 1.5, 0.15))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color_texture: Some(q_img),
            base_color: Color::WHITE,
            unlit: true,
            ..default()
        })),
        Transform::from_xyz(0.0, 5.8, gate_z),
        Billboard,
        GateUiCamera(q_cam),
    )).id();
    entities.push(crossbeam);

    let sign_mesh = meshes.add(Cuboid::new(2.2, 2.5, 0.12));

    // ── Left sign — warm gold ──
    let (l_img, l_cam) = make_sign_texture(
        commands, images,
        font.clone(),
        answer_l,
        Color::srgb(0.9, 0.72, 0.08),
        Color::srgb(0.08, 0.04, 0.0),
    );
    entities.push(commands.spawn((
        Mesh3d(sign_mesh.clone()),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color_texture: Some(l_img),
            base_color: Color::WHITE,
            unlit: true,
            ..default()
        })),
        Transform::from_xyz(-1.8, 3.2, gate_z),
        Billboard,
        GateSign::Left,
        GateUiCamera(l_cam),
    )).id());

    // ── Right sign — cool blue ──
    let (r_img, r_cam) = make_sign_texture(
        commands, images,
        font.clone(),
        answer_r,
        Color::srgb(0.1, 0.4, 0.85),
        Color::WHITE,
    );
    entities.push(commands.spawn((
        Mesh3d(sign_mesh),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color_texture: Some(r_img),
            base_color: Color::WHITE,
            unlit: true,
            ..default()
        })),
        Transform::from_xyz(1.8, 3.2, gate_z),
        Billboard,
        GateSign::Right,
        GateUiCamera(r_cam),
    )).id());

    entities
}

// ─── Systems ──────────────────────────────────────────────────────────────────

fn input_system(
    touches: Res<Touches>,
    mouse: Res<ButtonInput<MouseButton>>,
    mut drag: ResMut<DragState>,
    window_q: Query<&Window, With<bevy::window::PrimaryWindow>>,
) {
    let Ok(window) = window_q.single() else { return };
    let half_w = window.width() * 0.5;
    let mut touch_handled = false;

    for touch in touches.iter_just_pressed() {
        touch_handled = true;
        drag.active = true;
        drag.start_x = touch.position().x - half_w;
        drag.current_x = drag.start_x;
        drag.touch_id = Some(touch.id());
    }
    for touch in touches.iter() {
        if drag.touch_id == Some(touch.id()) {
            touch_handled = true;
            drag.current_x = touch.position().x - half_w;
        }
    }
    for touch in touches.iter_just_released() {
        if drag.touch_id == Some(touch.id()) {
            touch_handled = true;
            drag.active = false;
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
        }
        if mouse.pressed(MouseButton::Left)
            && let Some(pos) = window.cursor_position()
        {
            drag.current_x = pos.x - half_w;
        }
        if mouse.just_released(MouseButton::Left) {
            drag.active = false;
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
    transform.translation.z -= PLAYER_SPEED * dt;
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
) -> Result {
    egui::Window::new("Camera")
        .default_open(false)
        .show(contexts.ctx_mut()?, |ui| {
            ui.add(egui::Slider::new(&mut settings.height, 0.5..=20.0).text("height"));
            ui.add(egui::Slider::new(&mut settings.distance, 1.0..=30.0).text("distance"));
            ui.add(egui::Slider::new(&mut settings.lerp, 0.5..=20.0).text("lerp speed"));
        });
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
    mut images: ResMut<Assets<Image>>,
    mut manager: ResMut<GateManager>,
    mut deck: ResMut<Deck>,
    font: Res<JpFont>,
    player_q: Query<&Transform, With<Player>>,
    ui_cam_q: Query<&GateUiCamera>,
) {
    let Ok(player_t) = player_q.single() else { return };
    let pz = player_t.translation.z;

    while manager.next_z > pz - GATES_AHEAD as f32 * GATE_SPACING {
        let z = manager.next_z;
        let (question, correct, distractor) = deck.pick();
        let (answer_l, answer_r) = if deck.rng.gen_bool(0.5) {
            (correct, distractor)
        } else {
            (distractor, correct)
        };
        let entities = spawn_gate(
            &mut commands, &mut meshes, &mut materials, &mut images,
            font.0.clone(), z, question, answer_l, answer_r,
        );
        manager.live.push_back((z, entities));
        manager.next_z -= GATE_SPACING;
    }

    while let Some((gate_z, entities)) = manager.live.front() {
        if *gate_z > pz + GATE_SPACING {
            for &e in entities {
                // Also despawn the offscreen UI camera attached to this mesh
                if let Ok(ui_cam) = ui_cam_q.get(e) {
                    commands.entity(ui_cam.0).despawn_related::<Children>();
                    commands.entity(ui_cam.0).despawn();
                }
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
