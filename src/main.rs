mod sr;
mod supabase;

use bevy::anti_alias::smaa::{Smaa, SmaaPreset};
use bevy::gltf::GltfAssetLabel;
use bevy::render::alpha::AlphaMode;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};
use bevy::tasks::{IoTaskPool, Task};
use bevy::{asset::AssetMetaCheck, prelude::*};
use bevy_egui::{EguiContexts, EguiPlugin, EguiPrimaryContextPass, egui};
use bevy_rich_text3d::{LoadFonts, Text3d, Text3dPlugin, Text3dStyling, TextAtlas};
use rand::Rng;
use sr::Scheduler;
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

const STEER_SMOOTHING: f32 = 5.0;
const DASH_STEER_SMOOTHING: f32 = 2.0;
const MAX_LATERAL: f32 = 2.0;
const DASH_SPEED_MULT: f32 = 6.8;
const SWIPE_UP_VELOCITY: f32 = 800.0; // pixels per second upward
const CAM_OFFSET_DEFAULT: Vec3 = Vec3::new(0.0, 2.5, 30.0);

const HIRAGANA_DECK: &[(&str, &str)] = &[
    ("あ", "a"), ("い", "i"), ("う", "u"), ("え", "e"), ("お", "o"),
    ("か", "ka"), ("き", "ki"), ("く", "ku"), ("け", "ke"), ("こ", "ko"),
    ("さ", "sa"), ("し", "shi"), ("す", "su"), ("せ", "se"), ("そ", "so"),
    ("た", "ta"), ("ち", "chi"), ("つ", "tsu"), ("て", "te"), ("と", "to"),
    ("な", "na"), ("に", "ni"), ("ぬ", "nu"), ("ね", "ne"), ("の", "no"),
    ("は", "ha"), ("ひ", "hi"), ("ふ", "fu"), ("へ", "he"), ("ほ", "ho"),
    ("ま", "ma"), ("み", "mi"), ("む", "mu"), ("め", "me"), ("も", "mo"),
    ("や", "ya"), ("ゆ", "yu"), ("よ", "yo"),
    ("ら", "ra"), ("り", "ri"), ("る", "ru"), ("れ", "re"), ("ろ", "ro"),
    ("わ", "wa"), ("を", "wo"), ("ん", "n"),
];

const KATAKANA_DECK: &[(&str, &str)] = &[
    ("ア", "a"), ("イ", "i"), ("ウ", "u"), ("エ", "e"), ("オ", "o"),
    ("カ", "ka"), ("キ", "ki"), ("ク", "ku"), ("ケ", "ke"), ("コ", "ko"),
    ("サ", "sa"), ("シ", "shi"), ("ス", "su"), ("セ", "se"), ("ソ", "so"),
    ("タ", "ta"), ("チ", "chi"), ("ツ", "tsu"), ("テ", "te"), ("ト", "to"),
    ("ナ", "na"), ("ニ", "ni"), ("ヌ", "nu"), ("ネ", "ne"), ("ノ", "no"),
    ("ハ", "ha"), ("ヒ", "hi"), ("フ", "fu"), ("ヘ", "he"), ("ホ", "ho"),
    ("マ", "ma"), ("ミ", "mi"), ("ム", "mu"), ("メ", "me"), ("モ", "mo"),
    ("ヤ", "ya"), ("ユ", "yu"), ("ヨ", "yo"),
    ("ラ", "ra"), ("リ", "ri"), ("ル", "ru"), ("レ", "re"), ("ロ", "ro"),
    ("ワ", "wa"), ("ヲ", "wo"), ("ン", "n"),
];

#[derive(Resource)]
struct CameraSettings {
    height: f32,
    distance: f32,
    lerp: f32,
    fov_degrees: f32,
    gate_scale: f32,
    player_speed: f32,
    gate_spacing: f32,
    player_scale: f32,
    anim_speed: f32,
    cam_side: f32,
    player_y: f32,
}

const TILE_LENGTH: f32 = 20.0;
const TILE_WIDTH: f32 = 5.0;
const TILES_AHEAD: i32 = 6;
const TILES_BEHIND: i32 = 3;

const GATES_AHEAD: usize = 2;
const GATES_BEHIND: usize = 1;
const SIGN_X_OFFSET: f32 = 1.8;
const SIGN_SLIDE_TARGET: f32 = 10.0;
const SIGN_SLIDE_SPEED: f32 = 3.5;

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
    word_index: usize,
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
    current_x: f32,
    current_y: f32,
    prev_y: f32,
    dash_triggered: bool, // swipe-up consumed for this gesture; cleared when dash resolves
    flick_dash: bool,     // finger released during dash — snap to center when gate passes
    touch_id: Option<u64>,
}

/// In-flight async task that fetches words from Supabase.
#[derive(Resource)]
struct WordsTask(Task<Vec<(String, String)>>);

/// Shared material for all Text3d entities.
#[derive(Resource)]
struct TextMat(Handle<StandardMaterial>);

/// Holds the animation clip loaded from the player GLTF.
#[derive(Resource)]
struct PlayerAnimClip(Handle<AnimationClip>);

/// Pre-created gate geometry and materials — reused for every gate spawn.
#[derive(Resource)]
struct GateAssets {
    post_mesh: Handle<Mesh>,
    crossbeam_mesh: Handle<Mesh>,
    sign_mesh: Handle<Mesh>,
    post_mat: Handle<StandardMaterial>,
    crossbeam_mat: Handle<StandardMaterial>,
    sign_mat_yellow: Handle<StandardMaterial>,
    sign_mat_blue: Handle<StandardMaterial>,
}

// ─── Deck selection & menu markers ───────────────────────────────────────────

#[derive(Resource, Default, Debug, Clone, PartialEq, Eq)]
enum DeckChoice {
    #[default]
    Hiragana,
    Katakana,
    N5Vocab,
}

#[derive(Component)]
struct MenuRoot;

#[derive(Component)]
struct DeckButton(DeckChoice);

#[derive(Component)]
struct StartButton;

// ─── Game state ───────────────────────────────────────────────────────────────

#[derive(States, Default, Debug, Clone, PartialEq, Eq, Hash)]
enum GameState {
    #[default]
    Menu,
    Loading,
    Playing,
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
    .init_state::<GameState>()
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
        player_scale: 1.0,
        anim_speed: 1.0,
        cam_side: 0.0,
        player_y: 0.0,
    })
    .add_systems(Startup, setup)
    // Always-running systems (all states)
    .add_systems(
        Update,
        (
            check_loading_system.run_if(in_state(GameState::Loading)),
            setup_player_animation,
            update_anim_speed,
            apply_player_scale,
            fix_player_material,
            #[cfg(not(target_arch = "wasm32"))]
            screenshot_system,
        ),
    )
    // Gameplay systems — only run once words are loaded
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
        )
            .chain()
            .run_if(in_state(GameState::Playing)),
    )
    .add_systems(
        EguiPrimaryContextPass,
        (
            setup_egui_fonts,
            loading_ui.run_if(in_state(GameState::Loading)),
            camera_settings_ui.run_if(in_state(GameState::Playing)),
            flash_overlay_ui.run_if(in_state(GameState::Playing)),
            sr_stats_ui.run_if(in_state(GameState::Playing)),
            menu_button_ui.run_if(in_state(GameState::Playing)),
        ),
    );

    app.init_resource::<DeckChoice>()
        .add_systems(OnEnter(GameState::Menu), spawn_menu_system)
        .add_systems(OnExit(GameState::Menu), cleanup_menu_system)
        .add_systems(OnExit(GameState::Playing), exit_playing_system)
        .add_systems(OnEnter(GameState::Loading), enter_loading_system)
        .add_systems(Update, poll_words_system.run_if(in_state(GameState::Loading)))
        .add_systems(
            Update,
            (menu_interaction_system, update_deck_button_visuals_system)
                .run_if(in_state(GameState::Menu)),
        );

    #[cfg(not(target_arch = "wasm32"))]
    if std::env::args().any(|a| a == "--screenshot") {
        app.add_systems(Update, auto_screenshot_system);
    }

    app.run();
}

// ─── Egui font setup ──────────────────────────────────────────────────────────

fn setup_egui_fonts(mut contexts: EguiContexts, mut done: Local<bool>) -> Result {
    if *done {
        return Ok(());
    }
    *done = true;
    let ctx = contexts.ctx_mut()?;
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        "takao".to_owned(),
        egui::FontData::from_static(include_bytes!("../assets/fonts/TakaoPGothic.ttf")).into(),
    );
    fonts
        .families
        .entry(egui::FontFamily::Proportional)
        .or_default()
        .insert(0, "takao".to_owned());
    ctx.set_fonts(fonts);
    Ok(())
}

// ─── Words loading ────────────────────────────────────────────────────────────

fn poll_words_system(
    mut commands: Commands,
    task: Option<ResMut<WordsTask>>,
    mut scheduler: ResMut<Scheduler>,
) {
    let Some(mut task) = task else { return };
    if let Some(words) =
        bevy::tasks::block_on(bevy::tasks::futures_lite::future::poll_once(&mut task.0))
    {
        info!("loaded {} words from Supabase", words.len());
        scheduler.load_words(words);
        commands.remove_resource::<WordsTask>();
    }
}

fn check_loading_system(
    scheduler: Res<Scheduler>,
    mut next_state: ResMut<NextState<GameState>>,
) {
    if scheduler.is_ready() {
        next_state.set(GameState::Playing);
    }
}

fn loading_ui(mut contexts: EguiContexts) -> Result {
    let ctx = contexts.ctx_mut()?;
    egui::CentralPanel::default()
        .frame(egui::Frame::NONE.fill(egui::Color32::from_black_alpha(180)))
        .show(ctx, |ui| {
            ui.centered_and_justified(|ui| {
                ui.heading("Loading words...");
            });
        });
    Ok(())
}

fn menu_button_ui(
    mut contexts: EguiContexts,
    mut next_state: ResMut<NextState<GameState>>,
) -> Result {
    let ctx = contexts.ctx_mut()?;
    egui::Area::new(egui::Id::new("menu_btn"))
        .anchor(egui::Align2::RIGHT_TOP, [-8.0, 8.0])
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            if ui.button("≡ Menu").clicked() {
                next_state.set(GameState::Menu);
            }
        });
    Ok(())
}

fn exit_playing_system(
    mut commands: Commands,
    tile_q: Query<Entity, With<Tile>>,
    mut tile_manager: ResMut<TileManager>,
    mut gate_manager: ResMut<GateManager>,
    mut scheduler: ResMut<Scheduler>,
    mut player_q: Query<(&mut Player, &mut Transform), Without<CameraMarker>>,
    mut cam_q: Query<&mut Transform, With<CameraMarker>>,
    mut drag: ResMut<DragState>,
    mut flash: ResMut<FlashState>,
) {
    for e in tile_q.iter() {
        commands.entity(e).despawn();
    }
    for (_, entities) in gate_manager.live.drain(..) {
        for e in entities {
            commands.entity(e).despawn();
        }
    }
    *tile_manager = TileManager {
        frontier_z: TILE_LENGTH / 2.0,
        spawned: std::collections::VecDeque::new(),
        count: 0,
    };
    *gate_manager = GateManager {
        next_z: -50.0,
        live: std::collections::VecDeque::new(),
    };
    *scheduler = Scheduler::new();
    if let Ok((mut player, mut transform)) = player_q.single_mut() {
        player.lateral = 0.0;
        player.target_lateral = 0.0;
        player.dashing = false;
        transform.translation = Vec3::new(0.0, 0.9, 0.0);
        transform.rotation = Quat::from_rotation_y(std::f32::consts::PI);
    }
    if let Ok(mut cam_t) = cam_q.single_mut() {
        cam_t.translation = CAM_OFFSET_DEFAULT;
        cam_t.look_at(Vec3::ZERO, Vec3::Y);
    }
    *drag = DragState::default();
    *flash = FlashState::default();
}

// ─── Loading entry ────────────────────────────────────────────────────────────

fn enter_loading_system(
    mut commands: Commands,
    deck: Res<DeckChoice>,
    mut scheduler: ResMut<Scheduler>,
) {
    match *deck {
        DeckChoice::Hiragana => {
            let words = HIRAGANA_DECK
                .iter()
                .map(|&(a, q)| (a.to_string(), q.to_string()))
                .collect();
            scheduler.load_words(words);
        }
        DeckChoice::Katakana => {
            let words = KATAKANA_DECK
                .iter()
                .map(|&(a, q)| (a.to_string(), q.to_string()))
                .collect();
            scheduler.load_words(words);
        }
        DeckChoice::N5Vocab => {
            let task = IoTaskPool::get().spawn(supabase::fetch_words());
            commands.insert_resource(WordsTask(task));
        }
    }
}

// ─── Menu ─────────────────────────────────────────────────────────────────────

const COLOR_DECK_SELECTED: Color = Color::srgb(0.9, 0.72, 0.08);
const COLOR_DECK_UNSELECTED: Color = Color::srgba(1.0, 1.0, 1.0, 0.15);

fn spawn_menu_system(mut commands: Commands) {
    commands
        .spawn((
            Node {
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                row_gap: Val::Px(24.0),
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.3)),
            MenuRoot,
        ))
        .with_children(|root| {
            // ── Title ──
            root.spawn((
                Text::new("NIHONGO RUN"),
                TextFont { font_size: 48.0, ..default() },
                TextColor(Color::WHITE),
            ));

            // ── Mode card ──
            root.spawn((
                Node {
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Center,
                    padding: UiRect::all(Val::Px(16.0)),
                    row_gap: Val::Px(10.0),
                    border_radius: BorderRadius::all(Val::Px(12.0)),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.55)),
            ))
            .with_children(|card| {
                card.spawn((
                    Text::new("MODE"),
                    TextFont { font_size: 14.0, ..default() },
                    TextColor(Color::srgba(1.0, 1.0, 1.0, 0.6)),
                ));
                card.spawn((
                    Node {
                        padding: UiRect {
                            left: Val::Px(20.0),
                            right: Val::Px(20.0),
                            top: Val::Px(8.0),
                            bottom: Val::Px(8.0),
                        },
                        align_items: AlignItems::Center,
                        justify_content: JustifyContent::Center,
                        border_radius: BorderRadius::all(Val::Px(8.0)),
                        ..default()
                    },
                    BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.15)),
                ))
                .with_children(|badge| {
                    badge.spawn((
                        Text::new("Endless Run"),
                        TextFont { font_size: 18.0, ..default() },
                        TextColor(Color::WHITE),
                    ));
                });
            });

            // ── Deck card ──
            root.spawn((
                Node {
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Center,
                    padding: UiRect::all(Val::Px(16.0)),
                    row_gap: Val::Px(10.0),
                    border_radius: BorderRadius::all(Val::Px(12.0)),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.55)),
            ))
            .with_children(|card| {
                card.spawn((
                    Text::new("DECK"),
                    TextFont { font_size: 14.0, ..default() },
                    TextColor(Color::srgba(1.0, 1.0, 1.0, 0.6)),
                ));
                // Row of deck buttons
                card.spawn(Node {
                    flex_direction: FlexDirection::Row,
                    column_gap: Val::Px(12.0),
                    ..default()
                })
                .with_children(|row| {
                    for (choice, label, selected) in [
                        (DeckChoice::Hiragana, "Hiragana", true),
                        (DeckChoice::Katakana, "Katakana", false),
                        (DeckChoice::N5Vocab, "N5 Vocab", false),
                    ] {
                        row.spawn((
                            Button,
                            DeckButton(choice),
                            Node {
                                padding: UiRect {
                                    left: Val::Px(20.0),
                                    right: Val::Px(20.0),
                                    top: Val::Px(10.0),
                                    bottom: Val::Px(10.0),
                                },
                                align_items: AlignItems::Center,
                                justify_content: JustifyContent::Center,
                                border_radius: BorderRadius::all(Val::Px(8.0)),
                                ..default()
                            },
                            BackgroundColor(if selected {
                                COLOR_DECK_SELECTED
                            } else {
                                COLOR_DECK_UNSELECTED
                            }),
                        ))
                        .with_children(|btn| {
                            btn.spawn((
                                Text::new(label),
                                TextFont { font_size: 18.0, ..default() },
                                TextColor(Color::WHITE),
                            ));
                        });
                    }
                });
            });

            // ── START button ──
            root.spawn((
                Button,
                StartButton,
                Node {
                    width: Val::Px(200.0),
                    height: Val::Px(56.0),
                    align_items: AlignItems::Center,
                    justify_content: JustifyContent::Center,
                    border_radius: BorderRadius::all(Val::Px(10.0)),
                    ..default()
                },
                BackgroundColor(Color::srgb(0.85, 0.15, 0.05)),
            ))
            .with_children(|btn| {
                btn.spawn((
                    Text::new("START"),
                    TextFont { font_size: 24.0, ..default() },
                    TextColor(Color::WHITE),
                ));
            });
        });
}

fn cleanup_menu_system(mut commands: Commands, q: Query<Entity, With<MenuRoot>>) {
    for e in q.iter() {
        commands.entity(e).despawn();
    }
}

fn menu_interaction_system(
    mut commands: Commands,
    deck_q: Query<(&Interaction, &DeckButton), (Changed<Interaction>, With<Button>)>,
    start_q: Query<&Interaction, (Changed<Interaction>, With<StartButton>, With<Button>)>,
    mut next_state: ResMut<NextState<GameState>>,
) {
    for (interaction, deck_btn) in deck_q.iter() {
        if *interaction == Interaction::Pressed {
            commands.insert_resource(deck_btn.0.clone());
        }
    }
    for interaction in start_q.iter() {
        if *interaction == Interaction::Pressed {
            next_state.set(GameState::Loading);
        }
    }
}

fn update_deck_button_visuals_system(
    deck: Res<DeckChoice>,
    mut btn_q: Query<(&DeckButton, &mut BackgroundColor)>,
) {
    if !deck.is_changed() {
        return;
    }
    for (btn, mut color) in btn_q.iter_mut() {
        *color = BackgroundColor(if btn.0 == *deck {
            COLOR_DECK_SELECTED
        } else {
            COLOR_DECK_UNSELECTED
        });
    }
}

// ─── Startup ──────────────────────────────────────────────────────────────────

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    asset_server: Res<AssetServer>,
) {
    // Shared material for all Text3d entities
    let text_mat = materials.add(StandardMaterial {
        base_color_texture: Some(TextAtlas::DEFAULT_IMAGE.clone()),
        alpha_mode: AlphaMode::Blend,
        unlit: true,
        cull_mode: None,
        ..default()
    });
    commands.insert_resource(TextMat(text_mat));
    commands.insert_resource(Scheduler::new());
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
    commands.insert_resource(GateAssets {
        post_mesh: meshes.add(Cylinder::new(0.18, 4.8)),
        crossbeam_mesh: meshes.add(Cuboid::new(2.5, 2.5, 0.12)),
        sign_mesh: meshes.add(Cuboid::new(3.0, 2.5, 0.12)),
        post_mat: materials.add(StandardMaterial {
            base_color: Color::srgb(0.85, 0.15, 0.05),
            perceptual_roughness: 0.5,
            ..default()
        }),
        crossbeam_mat: materials.add(StandardMaterial {
            base_color: Color::srgb(0.95, 0.90, 0.75),
            perceptual_roughness: 0.6,
            ..default()
        }),
        sign_mat_yellow: materials.add(StandardMaterial {
            base_color: Color::srgb(0.9, 0.72, 0.08),
            perceptual_roughness: 0.4,
            ..default()
        }),
        sign_mat_blue: materials.add(StandardMaterial {
            base_color: Color::srgb(0.1, 0.40, 0.85),
            perceptual_roughness: 0.4,
            ..default()
        }),
    });

    // Player — load from GLTF
    let anim_clip: Handle<AnimationClip> =
        asset_server.load(GltfAssetLabel::Animation(0).from_asset("test.glb"));
    commands.insert_resource(PlayerAnimClip(anim_clip));

    commands.spawn((
        SceneRoot(asset_server.load(GltfAssetLabel::Scene(0).from_asset("test.glb"))),
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
    gate_assets: &GateAssets,
    text_mat: Handle<StandardMaterial>,
    gate_z: f32,
    question: &str,
    answer_l: &str,
    answer_r: &str,
    correct_is_left: bool,
    word_index: usize,
    s: f32,
) -> Vec<Entity> {
    let mut entities = Vec::new();
    let correct_side = if correct_is_left { -1.0 } else { 1.0 };

    // ── Center post ──
    entities.push(
        commands
            .spawn((
                Mesh3d(gate_assets.post_mesh.clone()),
                MeshMaterial3d(gate_assets.post_mat.clone()),
                Transform::from_xyz(0.0, 2.4 * s, gate_z).with_scale(Vec3::splat(s)),
                GatePost { gate_z, correct_side, passed: false, word_index },
                PostDrop { gate_z },
            ))
            .id(),
    );

    // ── Question crossbeam (mesh + text) ──
    entities.push(
        commands
            .spawn((
                Mesh3d(gate_assets.crossbeam_mesh.clone()),
                MeshMaterial3d(gate_assets.crossbeam_mat.clone()),
                Transform::from_xyz(0.0, 5.8 * s, gate_z).with_scale(Vec3::splat(s)),
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
                    world_scale: Some(Vec2::splat(1.2 * s)),
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

    let [l0, l1] = spawn_sign(
        commands, gate_assets, text_mat.clone(),
        gate_z, -SIGN_X_OFFSET * s, gate_assets.sign_mat_yellow.clone(), answer_l, s,
    );
    let [r0, r1] = spawn_sign(
        commands, gate_assets, text_mat,
        gate_z, SIGN_X_OFFSET * s, gate_assets.sign_mat_blue.clone(), answer_r, s,
    );
    entities.extend([l0, l1, r0, r1]);

    entities
}

fn spawn_sign(
    commands: &mut Commands,
    gate_assets: &GateAssets,
    text_mat: Handle<StandardMaterial>,
    gate_z: f32,
    x: f32,
    sign_mat: Handle<StandardMaterial>,
    text: &str,
    s: f32,
) -> [Entity; 2] {
    let dir = x.signum();
    let bg = commands
        .spawn((
            Mesh3d(gate_assets.sign_mesh.clone()),
            MeshMaterial3d(sign_mat),
            Transform::from_xyz(x, 3.2 * s, gate_z).with_scale(Vec3::splat(s)),
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
                world_scale: Some(Vec2::splat(s)),
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
        drag.current_x = touch.position().x - half_w;
        drag.current_y = touch.position().y;
        drag.prev_y = drag.current_y;
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
            drag.current_x = pos.x - half_w;
            drag.current_y = pos.y;
            drag.prev_y = drag.current_y;
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

    // Hold dash ended: allow re-swiping
    if !player.dashing && drag.dash_triggered {
        drag.dash_triggered = false;
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
        let normalized = (drag.current_x / (window.width() * 0.5)).clamp(-1.0, 1.0);
        player.target_lateral = normalized * MAX_LATERAL;
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
    let smoothing = if player.dashing { DASH_STEER_SMOOTHING } else { STEER_SMOOTHING };
    player.lateral = player
        .lateral
        .lerp(player.target_lateral, smoothing * dt);
    let speed = if player.dashing {
        settings.player_speed * DASH_SPEED_MULT
    } else {
        settings.player_speed
    };
    transform.translation.z -= speed * dt;
    transform.translation.x = player.lateral;
    let lateral_vel = (player.target_lateral - player.lateral) * smoothing;
    let yaw = f32::atan2(-lateral_vel, speed);
    transform.rotation = Quat::from_rotation_y(std::f32::consts::PI + yaw);
}


fn setup_player_animation(
    mut commands: Commands,
    mut anim_players: Query<(Entity, &mut AnimationPlayer), Added<AnimationPlayer>>,
    mut graphs: ResMut<Assets<AnimationGraph>>,
    clip: Option<Res<PlayerAnimClip>>,
) {
    let Some(clip) = clip else { return };
    for (entity, mut player) in anim_players.iter_mut() {
        let (graph, node) = AnimationGraph::from_clip(clip.0.clone());
        let graph_handle = graphs.add(graph);
        commands
            .entity(entity)
            .insert(AnimationGraphHandle(graph_handle));
        player.play(node).repeat();
    }
}


fn update_anim_speed(
    settings: Res<CameraSettings>,
    mut anim_players: Query<&mut AnimationPlayer>,
) {
    for mut player in anim_players.iter_mut() {
        for (_, anim) in player.playing_animations_mut() {
            anim.set_speed(settings.anim_speed);
        }
    }
}

fn apply_player_scale(
    settings: Res<CameraSettings>,
    mut player_q: Query<&mut Transform, With<Player>>,
) {
    if let Ok(mut transform) = player_q.single_mut() {
        transform.scale = Vec3::splat(settings.player_scale);
        transform.translation.y = settings.player_y;
    }
}

fn fix_player_material(
    player_q: Query<Entity, With<Player>>,
    children_q: Query<&Children>,
    mesh_mat_q: Query<&MeshMaterial3d<StandardMaterial>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut done: Local<bool>,
) {
    if *done { return; }
    let Ok(player_entity) = player_q.single() else { return };
    let mut stack = vec![player_entity];
    let mut fixed = false;
    while let Some(entity) = stack.pop() {
        if let Ok(handle) = mesh_mat_q.get(entity) {
            if let Some(mat) = materials.get_mut(handle.id()) {
                mat.alpha_mode = AlphaMode::Opaque;
                fixed = true;
            }
        }
        if let Ok(children) = children_q.get(entity) {
            stack.extend(children.iter());
        }
    }
    if fixed { *done = true; }
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
    let offset = Vec3::new(settings.cam_side, settings.height, settings.distance);
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
            ui.add(egui::Slider::new(&mut settings.player_scale, 0.1..=5.0).text("player size"));
            ui.add(egui::Slider::new(&mut settings.anim_speed, 0.1..=3.0).text("anim speed"));
            ui.add(egui::Slider::new(&mut settings.cam_side, -20.0..=20.0).text("cam side"));
            ui.add(egui::Slider::new(&mut settings.player_y, -5.0..=5.0).text("player y"));
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

fn sr_stats_ui(mut contexts: EguiContexts, scheduler: Res<Scheduler>) -> Result {
    let cards = &scheduler.cards;
    let now = scheduler.gate_pass_count;
    let due   = cards.iter().filter(|c| c.due_at <= now && c.reps > 0).count();
    let new   = cards.iter().filter(|c| c.reps == 0).count();
    let learn = cards.iter().filter(|c| c.reps > 0 && c.interval < 21.0).count();
    let rev   = cards.iter().filter(|c| c.interval >= 21.0).count();

    egui::Window::new("SR Stats")
        .default_open(false)
        .default_width(320.0)
        .show(contexts.ctx_mut()?, |ui| {
            ui.horizontal(|ui| {
                ui.label(format!("passes: {now}"));
                ui.separator();
                ui.label(format!("new: {new}"));
                ui.label(format!("due: {due}"));
                ui.label(format!("learn: {learn}"));
                ui.label(format!("review: {rev}"));
            });
            ui.separator();

            egui::ScrollArea::vertical().max_height(300.0).show(ui, |ui| {
                egui::Grid::new("sr_cards")
                    .striped(true)
                    .spacing([8.0, 2.0])
                    .show(ui, |ui| {
                        // Header
                        ui.strong("kanji");
                        ui.strong("kana");
                        ui.strong("status");
                        ui.strong("reps");
                        ui.strong("ivl");
                        ui.strong("ease");
                        ui.strong("due in");
                        ui.end_row();

                        for (i, card) in cards.iter().enumerate() {
                            let (kana, kanji) = &scheduler.words[i];
                            let status = if card.reps == 0 {
                                "new"
                            } else if card.due_at <= now {
                                "due"
                            } else if card.interval < 21.0 {
                                "learn"
                            } else {
                                "review"
                            };
                            let due_in = card.due_at.saturating_sub(now);
                            ui.label(kanji);
                            ui.label(kana);
                            ui.label(status);
                            ui.label(card.reps.to_string());
                            ui.label(format!("{:.0}", card.interval));
                            ui.label(format!("{:.2}", card.ease));
                            ui.label(if card.reps == 0 {
                                "-".to_string()
                            } else {
                                due_in.to_string()
                            });
                            ui.end_row();
                        }
                    });
            });
        });
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
    mut manager: ResMut<GateManager>,
    mut scheduler: ResMut<Scheduler>,
    gate_assets: Res<GateAssets>,
    text_mat: Res<TextMat>,
    settings: Res<CameraSettings>,
    player_q: Query<&Transform, With<Player>>,
) {
    let Ok(player_t) = player_q.single() else {
        return;
    };
    if !scheduler.is_ready() {
        return; // words not yet loaded from Supabase
    }
    let pz = player_t.translation.z;

    // Spawn until exactly GATES_AHEAD gates are in front of the player.
    let ahead = manager.live.iter().filter(|(z, _)| *z < pz).count();
    let to_spawn = GATES_AHEAD.saturating_sub(ahead);
    for _ in 0..to_spawn {
        let z = manager.next_z;
        let (question, correct, distractor, word_index) = scheduler.pick();
        let correct_is_left = scheduler.rng.gen_bool(0.5);
        let (answer_l, answer_r) = if correct_is_left {
            (correct, distractor)
        } else {
            (distractor, correct)
        };
        let entities = spawn_gate(
            &mut commands,
            &gate_assets,
            text_mat.0.clone(),
            z,
            &question,
            &answer_l,
            &answer_r,
            correct_is_left,
            word_index,
            settings.gate_scale,
        );
        manager.live.push_back((z, entities));
        manager.next_z -= settings.gate_spacing;
    }

    // Despawn oldest gates until at most GATES_BEHIND remain behind the player.
    loop {
        let behind = manager.live.iter().filter(|(z, _)| *z > pz).count();
        if behind <= GATES_BEHIND {
            break;
        }
        if let Some((_, entities)) = manager.live.pop_front() {
            for &e in &entities {
                commands.entity(e).despawn();
            }
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
    mut scheduler: ResMut<Scheduler>,
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
            let correct = player_side == post.correct_side;
            flash.correct = correct;
            flash.timer = 0.6;
            scheduler.record(post.word_index, correct);
            scheduler.gate_pass_count += 1;
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
