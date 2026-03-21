use bevy::{asset::AssetMetaCheck, prelude::*};

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
            let w = win.inner_width().ok().and_then(|v| v.as_f64()).unwrap_or(DEFAULT_WIDTH as f64) as u32;
            let h = win.inner_height().ok().and_then(|v| v.as_f64()).unwrap_or(DEFAULT_HEIGHT as f64) as u32;
            return (w.min(MAX_WEB_WIDTH), h.min(MAX_WEB_HEIGHT));
        }
    }
    (DEFAULT_WIDTH, DEFAULT_HEIGHT)
}

// ─── Game constants ───────────────────────────────────────────────────────────

const PLAYER_SPEED: f32 = 10.0;
const STEER_SMOOTHING: f32 = 8.0;
const MAX_LATERAL: f32 = 2.0;
const CAM_OFFSET: Vec3 = Vec3::new(0.0, 4.5, 8.0);
const CAM_LERP: f32 = 6.0;

const TILE_LENGTH: f32 = 20.0;
const TILE_WIDTH: f32 = 5.0;
const TILES_AHEAD: i32 = 6;   // how many tiles to keep in front of the player
const TILES_BEHIND: i32 = 3;  // how many tiles to keep behind the player

// ─── Components & Resources ───────────────────────────────────────────────────

#[derive(Component)]
struct Player {
    lateral: f32,         // current left/right position
    target_lateral: f32,  // where we're steering toward
}

#[derive(Component)]
struct CameraMarker;

#[derive(Component)]
struct Tile;

#[derive(Resource)]
struct TileAssets {
    mesh: Handle<Mesh>,
    mat_a: Handle<StandardMaterial>,
    mat_b: Handle<StandardMaterial>,
}

#[derive(Resource)]
struct TileManager {
    /// Z of the center of the next tile to spawn (decrements each spawn)
    frontier_z: f32,
    /// (center_z, entity) oldest → newest
    spawned: std::collections::VecDeque<(f32, Entity)>,
    /// Increments each spawn so we can alternate materials
    count: u32,
}

#[derive(Resource, Default)]
struct DragState {
    active: bool,
    start_x: f32,
    current_x: f32,
    touch_id: Option<u64>,
}

// ─── Main ─────────────────────────────────────────────────────────────────────

fn main() {
    let (w, h) = measure_screen();

    App::new()
        .add_plugins(
            DefaultPlugins
                .set(AssetPlugin { meta_check: AssetMetaCheck::Never, ..default() })
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        title: "Nihongo Run".into(),
                        resolution: (w, h).into(),
                        resizable: false,
                        canvas: Some("#bevy".to_string()),
                        ..default()
                    }),
                    ..default()
                }),
        )
        .insert_resource(ClearColor(Color::srgb(0.4, 0.65, 0.85)))
        .insert_resource(DragState::default())
        .add_systems(Startup, setup)
        .add_systems(Update, (input_system, steer_system, move_system, camera_follow_system, manage_tiles_system).chain())
        .run();
}

// ─── Startup ──────────────────────────────────────────────────────────────────

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    // Lighting
    commands.insert_resource(GlobalAmbientLight {
        color: Color::WHITE,
        brightness: 400.0,
        ..default()
    });
    commands.spawn((
        DirectionalLight { illuminance: 8_000.0, shadows_enabled: false, ..default() },
        Transform::from_rotation(Quat::from_euler(EulerRot::XYZ, -0.8, 0.3, 0.0)),
    ));

    // Tile shared assets — two alternating stone colors so motion is visible
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

    // Camera
    commands.spawn((
        Camera3d::default(),
        Msaa::Off,
        Transform::from_translation(CAM_OFFSET).looking_at(Vec3::ZERO, Vec3::Y),
        CameraMarker,
    ));
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
        if mouse.just_pressed(MouseButton::Left) {
            if let Some(pos) = window.cursor_position() {
                drag.active = true;
                drag.start_x = pos.x - half_w;
                drag.current_x = drag.start_x;
            }
        }
        if mouse.pressed(MouseButton::Left) {
            if let Some(pos) = window.cursor_position() {
                drag.current_x = pos.x - half_w;
            }
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
    player_q: Query<&Transform, (With<Player>, Without<CameraMarker>)>,
    mut cam_q: Query<&mut Transform, (With<CameraMarker>, Without<Player>)>,
) {
    let Ok(player_t) = player_q.single() else { return };
    let Ok(mut cam_t) = cam_q.single_mut() else { return };

    let ideal = player_t.translation + CAM_OFFSET;
    cam_t.translation = cam_t.translation.lerp(ideal, CAM_LERP * time.delta_secs());
    cam_t.look_at(player_t.translation + Vec3::Y * 0.5, Vec3::Y);
}

fn manage_tiles_system(
    mut commands: Commands,
    mut manager: ResMut<TileManager>,
    assets: Res<TileAssets>,
    player_q: Query<&Transform, With<Player>>,
) {
    let Ok(player_t) = player_q.single() else { return };
    let pz = player_t.translation.z;

    // Spawn tiles ahead until we have TILES_AHEAD tiles in front of the player.
    // Player moves toward -Z, so "ahead" = lower (more negative) Z.
    while manager.frontier_z > pz - TILES_AHEAD as f32 * TILE_LENGTH {
        let center_z = manager.frontier_z;
        let mat = if manager.count % 2 == 0 {
            assets.mat_a.clone()
        } else {
            assets.mat_b.clone()
        };
        let entity = commands.spawn((
            Tile,
            Mesh3d(assets.mesh.clone()),
            MeshMaterial3d(mat),
            Transform::from_xyz(0.0, -0.15, center_z),
        )).id();
        manager.spawned.push_back((center_z, entity));
        manager.frontier_z -= TILE_LENGTH;
        manager.count += 1;
    }

    // Despawn tiles that are TILES_BEHIND tiles behind the player.
    while let Some(&(center_z, entity)) = manager.spawned.front() {
        if center_z > pz + TILES_BEHIND as f32 * TILE_LENGTH {
            commands.entity(entity).despawn();
            manager.spawned.pop_front();
        } else {
            break;
        }
    }
}
