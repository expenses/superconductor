#[cfg(feature = "wasm")]
use wasm_bindgen::prelude::wasm_bindgen;

use bevy_app::Update;
use bevy_ecs::system::Resource;
use superconductor::{
    bevy_app, bevy_ecs, components, renderer_core,
    resources::{
        Camera, EventQueue, HdrTexture, LdrTexture, LightvolTextures, NewIblCubemap,
        NewLightvolTextures, ProbesArrayInfo, WindowChanges,
    },
    url, winit,
    winit::event::{ElementState, VirtualKeyCode},
    Mode, Vec3,
};

#[cfg(feature = "wasm")]
#[wasm_bindgen(start)]
pub fn wasm_main() {
    console_error_panic_hook::set_once();
    console_log::init_with_level(log::Level::Info).unwrap();
    wasm_bindgen_futures::spawn_local(run());
}

pub async fn run() {
    #[cfg(feature = "wasm")]
    basis_universal_wasm::wasm_init().await;

    #[cfg(feature = "webgl")]
    let mode = select_mode_via_buttons().await;

    #[cfg(not(feature = "webgl"))]
    let mode = Mode::Desktop;

    let initialised_state = superconductor::initialise(mode).await;

    let mut app = bevy_app::App::new();

    app.add_plugins(SuperconductorPlugin::new(mode));

    superconductor::run_rendering_loop(app, initialised_state);
}

use bevy_app::{App, Plugin};
use bevy_ecs::prelude::{Component, Local, Query, Res, ResMut, With};

pub struct SuperconductorPlugin {
    mode: Mode,
}

impl SuperconductorPlugin {
    fn new(mode: Mode) -> Self {
        Self { mode }
    }
}

impl Plugin for SuperconductorPlugin {
    fn build(&self, app: &mut App) {
        #[cfg(feature = "wasm")]
        let href = web_sys::window().unwrap().location().href().unwrap();
        #[cfg(not(feature = "wasm"))]
        let href = "http://localhost:8000";
        #[allow(clippy::needless_borrow)]
        let href = url::Url::parse(&href).unwrap();

        let mut model_url = std::borrow::Cow::Borrowed("assets/models/sponza_packed.glb");

        for (key, value) in href.query_pairs() {
            if key == "model" {
                model_url = value;
            }
        }

        let model = app
            .world
            .spawn_empty()
            .insert(components::ModelUrl(
                url::Url::options()
                    .base_url(Some(&href))
                    .parse(&model_url)
                    .unwrap(),
            ))
            .insert(components::Instances::default())
            .insert(components::InstanceRanges::default())
            .id();

        app.world
            .spawn_empty()
            .insert(components::InstanceOf(model))
            .insert(components::Instance(renderer_core::Instance::new(
                Vec3::new(0.0, 0.0, 0.0),
                1.0,
                Default::default(),
            )));

        let camera_rig: dolly::rig::CameraRig = dolly::rig::CameraRig::builder()
            .with(dolly::drivers::Position::new(Vec3::new(0.0, 1.75, 0.0)))
            .with(dolly::drivers::YawPitch::new().pitch_degrees(0.0))
            .with(dolly::drivers::Smooth::new_position_rotation(0.5, 0.5))
            .build();

        app.insert_resource(KeyboardState::default());
        app.insert_resource(CameraRig(camera_rig));

        app.add_systems(Update, rotate_entities);
        app.add_systems(Update, handle_keyboard_input);
        app.add_systems(Update, update_camera);
        app.add_systems(Update, animate_vrms);

        let plugin: superconductor::XrPlugin = superconductor::XrPlugin::new(self.mode);

        plugin.build(app);

        app.insert_resource(NewIblCubemap(Some(
            url::Url::options()
                .base_url(Some(&href))
                .parse("assets/cubemaps/noon.ktx2")
                .unwrap(),
        )));
        app.insert_resource(ProbesArrayInfo::new(
            Vec3::new(0.0, 6.0, 0.0),
            Vec3::new(24.0, 12.0, 12.0),
        ));

        let astc_base_url = url::Url::options()
            .base_url(Some(&href))
            .parse("assets/lighting/astc/")
            .unwrap();

        let bcn_base_url = url::Url::options()
            .base_url(Some(&href))
            .parse("assets/lighting/bcn/")
            .unwrap();

        app.insert_resource(NewLightvolTextures(Some(LightvolTextures {
            sh0: url::Url::options()
                .base_url(Some(&href))
                .parse("assets/lighting/bcn/lightvol.ktx2")
                .unwrap(),
            sh1_x: url::Url::options()
                .base_url(Some(&href))
                .parse("assets/lighting/bcn/lightvol_x.ktx2")
                .unwrap(),
            sh1_y: url::Url::options()
                .base_url(Some(&href))
                .parse("assets/lighting/bcn/lightvol_y.ktx2")
                .unwrap(),
            sh1_z: url::Url::options()
                .base_url(Some(&href))
                .parse("assets/lighting/bcn/lightvol_z.ktx2")
                .unwrap(),
            lightmap_sh0: HdrTexture {
                bc6h: url::Url::options()
                    .base_url(Some(&bcn_base_url))
                    .parse("lightmap.ktx2")
                    .unwrap(),
                astc_hdr: url::Url::options()
                    .base_url(Some(&astc_base_url))
                    .parse("lightmap.ktx2")
                    .unwrap(),
            },
            lightmap_sh1_x: LdrTexture {
                astc: url::Url::options()
                    .base_url(Some(&astc_base_url))
                    .parse("lightmap_x.ktx2")
                    .unwrap(),
                bc7: url::Url::options()
                    .base_url(Some(&bcn_base_url))
                    .parse("lightmap_x.ktx2")
                    .unwrap(),
            },
            lightmap_sh1_y: LdrTexture {
                astc: url::Url::options()
                    .base_url(Some(&astc_base_url))
                    .parse("lightmap_y.ktx2")
                    .unwrap(),
                bc7: url::Url::options()
                    .base_url(Some(&bcn_base_url))
                    .parse("lightmap_y.ktx2")
                    .unwrap(),
            },
            lightmap_sh1_z: LdrTexture {
                astc: url::Url::options()
                    .base_url(Some(&astc_base_url))
                    .parse("lightmap_z.ktx2")
                    .unwrap(),
                bc7: url::Url::options()
                    .base_url(Some(&bcn_base_url))
                    .parse("lightmap_z.ktx2")
                    .unwrap(),
            },
        })));
    }
}

fn animate_vrms(
    mut instance_query: Query<
        (&components::InstanceOf, &mut components::AnimationJoints),
        With<VrmInstance>,
    >,
    model_query: Query<&components::AnimatedModel>,
) {
    instance_query.for_each_mut(|(instance_of, mut animation_joints)| {
        match model_query.get(instance_of.0) {
            Ok(animated_model) => {
                for i in 0..5 {
                    animation_joints
                        .0
                        .get_joint_mut(
                            i,
                            &animated_model
                                .0
                                .animation_data
                                .joint_indices_to_node_indices,
                        )
                        .rotation *=
                        renderer_core::glam::Quat::from_rotation_y(5.0_f32.to_radians());
                }
                animation_joints
                    .0
                    .update(&animated_model.0.animation_data.depth_first_nodes);
            }
            Err(error) => {
                log::warn!("Got an error when proc animations: {}", error);
            }
        }
    })
}

#[cfg(feature = "webgl")]
pub async fn select_mode_via_buttons() -> superconductor::Mode {
    use futures::FutureExt;

    let vr_button = create_button("Start VR");
    let ar_button = create_button("Start AR");
    let desktop_button = create_button("Start Desktop");

    let start_vr_future = button_click_future(&vr_button);
    let start_ar_future = button_click_future(&ar_button);
    let start_desktop_future = button_click_future(&desktop_button);

    futures::select! {
        _ = Box::pin(start_vr_future.fuse()) => superconductor::Mode::Vr,
        _ = Box::pin(start_ar_future.fuse()) => superconductor::Mode::Ar,
        _ = Box::pin(start_desktop_future.fuse()) => superconductor::Mode::Desktop,
    }
}

#[cfg(feature = "webgl")]
async fn button_click_future(button: &web_sys::HtmlButtonElement) {
    wasm_bindgen_futures::JsFuture::from(js_sys::Promise::new(&mut |resolve, _reject| {
        button.set_onclick(Some(&resolve))
    }))
    .await
    .unwrap();
}

#[cfg(feature = "webgl")]
fn create_button(text: &str) -> web_sys::HtmlButtonElement {
    use wasm_bindgen::JsCast;

    let button: web_sys::HtmlButtonElement = web_sys::window()
        .unwrap()
        .document()
        .unwrap()
        .create_element("button")
        .unwrap()
        .unchecked_into();

    button.set_inner_text(text);

    let body = web_sys::window()
        .unwrap()
        .document()
        .unwrap()
        .body()
        .unwrap();

    body.append_child(&web_sys::Element::from(button.clone()))
        .unwrap();

    button
}

#[derive(Component)]
struct Spinning;

#[derive(Component)]
struct VrmInstance;

#[derive(Default, Resource)]
struct KeyboardState {
    forwards: bool,
    right: bool,
    left: bool,
    backwards: bool,
    cursor_grab: bool,
    control: bool,
}

fn rotate_entities(mut query: Query<&mut components::Instance, With<Spinning>>) {
    query.for_each_mut(|mut instance| {
        instance.0.rotation *= renderer_core::glam::Quat::from_rotation_y(0.01)
    });
}

#[allow(clippy::collapsible_match, clippy::single_match)]
fn handle_keyboard_input(
    mut events: ResMut<EventQueue>,
    mut keyboard_state: ResMut<KeyboardState>,
    mut camera_rig: ResMut<CameraRig>,
    mut window_changes: ResMut<WindowChanges>,
    mut fullscreen: Local<bool>,
) {
    for event in events.0.drain(..) {
        match event {
            winit::event::Event::WindowEvent { event, .. } => match event {
                winit::event::WindowEvent::KeyboardInput { input, .. } => {
                    let pressed = input.state == ElementState::Pressed;

                    match input.virtual_keycode {
                        Some(VirtualKeyCode::W | VirtualKeyCode::Up) => {
                            keyboard_state.forwards = pressed;
                        }
                        Some(VirtualKeyCode::A | VirtualKeyCode::Left) => {
                            keyboard_state.left = pressed;
                        }
                        Some(VirtualKeyCode::S | VirtualKeyCode::Down) => {
                            keyboard_state.backwards = pressed;
                        }
                        Some(VirtualKeyCode::D | VirtualKeyCode::Right) => {
                            keyboard_state.right = pressed;
                        }
                        Some(VirtualKeyCode::G) => {
                            if pressed {
                                keyboard_state.cursor_grab = !keyboard_state.cursor_grab;
                                window_changes.cursor_grab = Some(keyboard_state.cursor_grab);
                                window_changes.cursor_visible = Some(!keyboard_state.cursor_grab);
                            }
                        }
                        Some(VirtualKeyCode::LControl | VirtualKeyCode::RControl) => {
                            keyboard_state.control = pressed
                        }
                        Some(VirtualKeyCode::F) => {
                            if pressed && keyboard_state.control {
                                *fullscreen = !*fullscreen;

                                window_changes.fullscreen = Some(*fullscreen);
                            }
                        }
                        _ => {}
                    }
                }
                _ => {}
            },
            winit::event::Event::DeviceEvent { event, .. } => match event {
                winit::event::DeviceEvent::MouseMotion {
                    delta: (delta_x, delta_y),
                } => {
                    if keyboard_state.cursor_grab {
                        camera_rig
                            .0
                            .driver_mut::<dolly::drivers::YawPitch>()
                            .rotate_yaw_pitch(-0.1 * delta_x as f32, -0.1 * delta_y as f32);
                    }
                }
                _ => {}
            },
            _ => {}
        }
    }
}

fn update_camera(
    keyboard_state: Res<KeyboardState>,
    mut camera: ResMut<Camera>,
    mut camera_rig: ResMut<CameraRig>,
) {
    let forwards = keyboard_state.forwards as i32 - keyboard_state.backwards as i32;
    let right = keyboard_state.right as i32 - keyboard_state.left as i32;

    let move_vec = camera_rig.0.final_transform.rotation
        * Vec3::new(right as f32, 0.0, -forwards as f32).clamp_length_max(1.0);

    let delta_time = 1.0 / 60.0;
    let speed = 3.0;

    camera_rig
        .0
        .driver_mut::<dolly::drivers::Position>()
        .translate(move_vec * delta_time * speed);

    camera_rig.0.update(delta_time);

    camera.position = camera_rig.0.final_transform.position;
    camera.rotation = camera_rig.0.final_transform.rotation;
}

#[derive(Resource)]
pub struct CameraRig(dolly::rig::CameraRig);
