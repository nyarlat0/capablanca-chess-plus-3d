use bevy::{
    anti_alias::fxaa::Fxaa,
    asset::RenderAssetUsages,
    camera::{CameraUpdateSystems, RenderTarget, visibility::RenderLayers},
    image::{ImageSampler, ImageSamplerDescriptor},
    math::{reflection_matrix, uvec2},
    pbr::{ExtendedMaterial, MaterialExtension},
    prelude::*,
    render::render_resource::{
        AsBindGroup, Extent3d, TextureDimension, TextureFormat, TextureUsages,
    },
    shader::ShaderRef,
    transform::TransformSystems,
    window::{PrimaryWindow, WindowResized},
};
use bevy_panorbit_camera::{PanOrbitCamera, PanOrbitCameraSystemSet};

use crate::pieces::PieceRoot;

const REFLECTION_LAYER: usize = 1;
const MAX_REFLECTION_TEXTURE_DIMENSION: u32 = 2_048;
const PIPELINE_WARMUP_FRAMES: u8 = 30;
const RENDER_TARGET_REFRESH_FRAMES: u8 = 3;
const PLANAR_REFLECTION_SHADER: &str = "shaders/planar_reflection.wgsl";

pub(crate) type PlanarBoardMaterial = ExtendedMaterial<StandardMaterial, PlanarReflectionExtension>;

pub(crate) struct PlanarReflectionPlugin;

impl Plugin for PlanarReflectionPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(MaterialPlugin::<PlanarBoardMaterial>::default())
            .init_resource::<ReflectionDirty>()
            .add_systems(
                Startup,
                setup_planar_reflection.in_set(PlanarReflectionStartup),
            )
            .add_systems(Update, resize_reflection_texture)
            .add_systems(
                PostUpdate,
                update_reflection_camera
                    .after(PanOrbitCameraSystemSet)
                    .before(TransformSystems::Propagate)
                    .before(CameraUpdateSystems),
            );
    }
}

#[derive(Clone, AsBindGroup, Asset, Reflect)]
pub(crate) struct PlanarReflectionExtension {
    // Vec4 keeps the uniform layout WebGL2-compatible. X stores reflection
    // strength and Y the maximum roughness blur radius in target pixels.
    #[uniform(100)]
    reflection_strength: Vec4,
}

impl PlanarReflectionExtension {
    pub(crate) fn new(strength: f32, max_blur_pixels: f32) -> Self {
        Self {
            reflection_strength: Vec4::new(strength, max_blur_pixels, 0.0, 0.0),
        }
    }
}

impl MaterialExtension for PlanarReflectionExtension {
    fn fragment_shader() -> ShaderRef {
        PLANAR_REFLECTION_SHADER.into()
    }
}

#[derive(Resource)]
pub(crate) struct PlanarReflectionImage(pub(crate) Handle<Image>);

#[derive(SystemSet, Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct PlanarReflectionStartup;

#[derive(Component)]
struct PlanarReflectionCamera;

#[derive(Component)]
pub(crate) struct ReflectedPieceMesh;

type MainCameraData<'a> = (Ref<'a, Transform>, Ref<'a, Projection>);
type MainCameraFilter = (With<PanOrbitCamera>, Without<PlanarReflectionCamera>);
type ReflectionCameraData<'a> = (&'a mut Camera, &'a mut Transform, &'a mut Projection);
type ReflectionCameraFilter = (With<PlanarReflectionCamera>, Without<PanOrbitCamera>);

#[derive(Resource)]
struct ReflectionDirty {
    frames_remaining: u8,
}

impl Default for ReflectionDirty {
    fn default() -> Self {
        Self {
            frames_remaining: RENDER_TARGET_REFRESH_FRAMES,
        }
    }
}

impl ReflectionDirty {
    fn request_frames(&mut self, frames: u8) {
        self.frames_remaining = self.frames_remaining.max(frames);
    }

    fn consume_frame(&mut self) -> bool {
        let needs_render = self.frames_remaining > 0;
        self.frames_remaining = self.frames_remaining.saturating_sub(1);
        needs_render
    }
}

fn setup_planar_reflection(
    mut commands: Commands,
    windows: Query<&Window, With<PrimaryWindow>>,
    mut images: ResMut<Assets<Image>>,
) {
    let window = windows.single().expect("the primary window exists");
    let texture_size = reflection_texture_size(window);
    let image = images.add(create_reflection_image(texture_size));

    commands.insert_resource(PlanarReflectionImage(image.clone()));
    commands.spawn((
        Camera3d::default(),
        Camera {
            order: -1,
            clear_color: Color::NONE.into(),
            invert_culling: true,
            ..default()
        },
        RenderTarget::Image(image.into()),
        Transform::IDENTITY,
        Projection::Perspective(PerspectiveProjection::default()),
        Msaa::Off,
        Fxaa::default(),
        RenderLayers::layer(REFLECTION_LAYER),
        PlanarReflectionCamera,
        Name::new("Planar reflection camera"),
    ));
}

fn resize_reflection_texture(
    windows: Query<&Window>,
    mut resize_events: MessageReader<WindowResized>,
    reflection_image: Option<ResMut<PlanarReflectionImage>>,
    mut reflection_targets: Query<&mut RenderTarget, With<PlanarReflectionCamera>>,
    mut board_materials: ResMut<Assets<PlanarBoardMaterial>>,
    mut images: ResMut<Assets<Image>>,
    mut dirty: ResMut<ReflectionDirty>,
) {
    let Some(event) = resize_events.read().last() else {
        return;
    };
    let Ok(window) = windows.get(event.window) else {
        return;
    };
    let Some(mut reflection_image) = reflection_image else {
        return;
    };
    let size = reflection_texture_size(window);
    let Some(previous_image) = images.get(&reflection_image.0) else {
        return;
    };
    if previous_image.texture_descriptor.size.width == size.x
        && previous_image.texture_descriptor.size.height == size.y
    {
        return;
    }

    // A render target's GPU texture view changes with its extent. Replacing
    // the handle ensures that the camera and every material bind group switch
    // to the same view in one frame; resizing the asset in place can leave
    // stale bindings after a window resize.
    let image = images.add(create_reflection_image(size));
    let previous_image = std::mem::replace(&mut reflection_image.0, image.clone());
    for mut target in &mut reflection_targets {
        *target = image.clone().into();
    }
    for (_, material) in board_materials.iter_mut() {
        material.base.emissive_texture = Some(image.clone());
    }
    images.remove(previous_image.id());
    dirty.request_frames(RENDER_TARGET_REFRESH_FRAMES);
}

fn update_reflection_camera(
    main_camera: Single<MainCameraData<'_>, MainCameraFilter>,
    reflection_camera: Single<ReflectionCameraData<'_>, ReflectionCameraFilter>,
    piece_roots: Query<Ref<Transform>, (With<PieceRoot>, Without<PlanarReflectionCamera>)>,
    added_reflected_meshes: Query<(), Added<ReflectedPieceMesh>>,
    mut dirty: ResMut<ReflectionDirty>,
) {
    let (main_transform, main_projection) = main_camera.into_inner();
    let camera_changed = main_transform.is_changed() || main_projection.is_changed();
    let pieces_changed = piece_roots.iter().any(|transform| transform.is_changed());
    if !added_reflected_meshes.is_empty() {
        // The mesh exists in the ECS before its specialized GPU pipeline is
        // necessarily ready, especially on WebGPU. Keep submitting the pass
        // for a short warmup instead of caching the first (possibly empty)
        // target forever.
        dirty.request_frames(PIPELINE_WARMUP_FRAMES);
    }
    let needs_render = dirty.consume_frame() || camera_changed || pieces_changed;
    let (mut camera, mut reflected_transform, mut reflected_projection) =
        reflection_camera.into_inner();

    camera.is_active = needs_render;
    if !needs_render {
        return;
    }

    let Projection::Perspective(main_projection) = &*main_projection else {
        camera.is_active = false;
        return;
    };
    let (transform, projection) = calculate_reflection_camera(&main_transform, main_projection);
    *reflected_transform = transform;
    *reflected_projection = Projection::Perspective(projection);
}

fn calculate_reflection_camera(
    main_transform: &Transform,
    main_projection: &PerspectiveProjection,
) -> (Transform, PerspectiveProjection) {
    // The polished square tops lie on the horizontal y=0 plane.
    let plane_normal = Vec3::Y;
    let reflected_transform = Transform::from_matrix(
        Mat4::from_mat3a(reflection_matrix(plane_normal)) * main_transform.to_matrix(),
    );

    // Use an oblique near plane so geometry below the board cannot leak into
    // the reflection texture.
    let distance_from_camera_to_plane = InfinitePlane3d::new(plane_normal)
        .signed_distance(Isometry3d::IDENTITY, -main_transform.translation);
    let view_from_world = main_transform.compute_affine().matrix3.inverse();
    let plane_normal_in_view = (view_from_world * -plane_normal).normalize();
    let projection = PerspectiveProjection {
        near_clip_plane: plane_normal_in_view.extend(distance_from_camera_to_plane),
        ..main_projection.clone()
    };

    (reflected_transform, projection)
}

fn reflection_texture_size(window: &Window) -> UVec2 {
    let full_size = uvec2(
        window.physical_width().max(1),
        window.physical_height().max(1),
    );
    let largest_dimension = full_size.max_element();
    if largest_dimension <= MAX_REFLECTION_TEXTURE_DIMENSION {
        return full_size;
    }

    let scale = MAX_REFLECTION_TEXTURE_DIMENSION as f32 / largest_dimension as f32;
    (full_size.as_vec2() * scale)
        .round()
        .as_uvec2()
        .max(UVec2::ONE)
}

fn create_reflection_image(size: UVec2) -> Image {
    let mut image = Image::new_uninit(
        image_extent(size),
        TextureDimension::D2,
        TextureFormat::Bgra8UnormSrgb,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor::linear());
    image.texture_descriptor.usage |=
        TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST | TextureUsages::RENDER_ATTACHMENT;
    image
}

fn image_extent(size: UVec2) -> Extent3d {
    Extent3d {
        width: size.x,
        height: size.y,
        depth_or_array_layers: 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reflected_camera_is_mirrored_across_board_plane() {
        let main_transform =
            Transform::from_xyz(3.0, 7.5, -11.0).looking_at(Vec3::new(0.5, 0.0, 0.5), Vec3::Y);
        let (reflected, projection) =
            calculate_reflection_camera(&main_transform, &PerspectiveProjection::default());

        assert!(
            reflected
                .translation
                .abs_diff_eq(Vec3::new(3.0, -7.5, -11.0), 1e-5)
        );
        assert!(projection.near_clip_plane.is_finite());
        assert!((projection.near_clip_plane.xyz().length() - 1.0).abs() < 1e-5);
    }

    #[test]
    fn dirty_countdown_extends_but_never_shortens_pending_refresh() {
        let mut dirty = ReflectionDirty {
            frames_remaining: 2,
        };

        dirty.request_frames(1);
        assert!(dirty.consume_frame());
        dirty.request_frames(3);
        assert!(dirty.consume_frame());
        assert!(dirty.consume_frame());
        assert!(dirty.consume_frame());
        assert!(!dirty.consume_frame());
    }
}
