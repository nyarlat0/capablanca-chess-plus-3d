use bevy::{
    asset::RenderAssetUsages,
    image::{ImageLoaderSettings, ImageSampler},
    light::Skybox,
    prelude::*,
    render::render_resource::{TextureDimension, TextureViewDescriptor, TextureViewDimension},
};
use bevy_panorbit_camera::PanOrbitCamera;

// Brightness is expressed in cd/m² and is applied after sampling the cubemap.
// This is the single value to tune if the background should be lighter or darker.
const SKYBOX_BRIGHTNESS: f32 = 500.0;
// Cubemap rotation in degrees around Bevy's world axes. Y is the height axis.
// Rotations are applied in XYZ Euler order.
const SKYBOX_ROTATION_X_DEGREES: f32 = 30.0;
const SKYBOX_ROTATION_Y_DEGREES: f32 = 220.0;
const SKYBOX_ROTATION_Z_DEGREES: f32 = 0.0;

// Cubemap array layers follow the WebGPU order: +X, -X, +Y, -Y, +Z, -Z.
const SKYBOX_FACE_PATHS: [&str; 6] = [
    "textures/Right_2K_TEX.png",
    "textures/Left_2K_TEX.png",
    "textures/Up_2K_TEX.png",
    "textures/Down_2K_TEX.png",
    "textures/Front_2K_TEX.png",
    "textures/Back_2K_TEX.png",
];

pub(crate) struct SkyboxPlugin;

impl Plugin for SkyboxPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, load_skybox_faces)
            .add_systems(Update, finish_skybox_loading);
    }
}

#[derive(Resource)]
struct SkyboxFaces([Handle<Image>; 6]);

fn load_skybox_faces(mut commands: Commands, asset_server: Res<AssetServer>) {
    let faces = SKYBOX_FACE_PATHS.map(|path| {
        asset_server
            .load_builder()
            .with_settings(|settings: &mut ImageLoaderSettings| {
                // The six source faces only need to exist in the main world
                // until they have been combined into one GPU cubemap.
                settings.asset_usage = RenderAssetUsages::MAIN_WORLD;
            })
            .load(path)
    });
    commands.insert_resource(SkyboxFaces(faces));
}

fn finish_skybox_loading(
    mut commands: Commands,
    faces: Option<Res<SkyboxFaces>>,
    mut images: ResMut<Assets<Image>>,
    camera: Single<Entity, (With<Camera3d>, With<PanOrbitCamera>)>,
) {
    let Some(faces) = faces else {
        return;
    };
    if faces.0.iter().any(|handle| images.get(handle).is_none()) {
        return;
    }

    match assemble_cubemap(&faces.0, &mut images) {
        Ok(cubemap) => {
            commands.entity(*camera).insert(Skybox {
                image: Some(cubemap),
                brightness: SKYBOX_BRIGHTNESS,
                rotation: skybox_rotation(),
            });
        }
        Err(error) => error!("Could not assemble the skybox cubemap: {error}"),
    }
    commands.remove_resource::<SkyboxFaces>();
}

fn skybox_rotation() -> Quat {
    Quat::from_euler(
        EulerRot::XYZ,
        SKYBOX_ROTATION_X_DEGREES.to_radians(),
        SKYBOX_ROTATION_Y_DEGREES.to_radians(),
        SKYBOX_ROTATION_Z_DEGREES.to_radians(),
    )
}

fn assemble_cubemap(
    face_handles: &[Handle<Image>; 6],
    images: &mut Assets<Image>,
) -> Result<Handle<Image>, String> {
    let first = images
        .get(&face_handles[0])
        .ok_or_else(|| "the first face is not loaded".to_owned())?;
    let face_size = first.texture_descriptor.size;
    let face_dimension = first.texture_descriptor.dimension;
    let face_format = first.texture_descriptor.format;
    let face_data_order = first.data_order;
    let bytes_per_face = first
        .data
        .as_ref()
        .ok_or_else(|| "the first face has no CPU pixel data".to_owned())?
        .len();

    if face_dimension != TextureDimension::D2
        || face_size.depth_or_array_layers != 1
        || face_size.width != face_size.height
    {
        return Err(format!(
            "every face must be one square 2D image, got {}x{}x{}",
            face_size.width, face_size.height, face_size.depth_or_array_layers
        ));
    }

    for (path, handle) in SKYBOX_FACE_PATHS.iter().zip(face_handles) {
        let face = images
            .get(handle)
            .ok_or_else(|| format!("{path} is not loaded"))?;
        let data_len = face.data.as_ref().map(Vec::len);
        if face.texture_descriptor.size != face_size
            || face.texture_descriptor.dimension != face_dimension
            || face.texture_descriptor.format != face_format
            || face.data_order != face_data_order
            || data_len != Some(bytes_per_face)
        {
            return Err(format!("{path} does not match the other cubemap faces"));
        }
    }

    let mut cubemap = images
        .remove(face_handles[0].id())
        .ok_or_else(|| "the first face disappeared while assembling".to_owned())?;
    let mut data = cubemap
        .data
        .take()
        .ok_or_else(|| "the first face has no CPU pixel data".to_owned())?;
    flip_face_horizontally(&mut data, face_size.width, face_size.height)?;
    data.reserve(bytes_per_face * (face_handles.len() - 1));

    for handle in &face_handles[1..] {
        let mut face = images
            .remove(handle.id())
            .ok_or_else(|| "a face disappeared while assembling".to_owned())?;
        let mut face_data = face
            .data
            .take()
            .ok_or_else(|| "a face has no CPU pixel data".to_owned())?;
        flip_face_horizontally(&mut face_data, face_size.width, face_size.height)?;
        data.append(&mut face_data);
    }

    cubemap.data = Some(data);
    cubemap.texture_descriptor.label = Some("space skybox cubemap");
    cubemap.texture_descriptor.size.depth_or_array_layers = face_handles.len() as u32;
    cubemap.texture_view_descriptor = Some(TextureViewDescriptor {
        dimension: Some(TextureViewDimension::Cube),
        ..default()
    });
    cubemap.sampler = ImageSampler::linear();
    cubemap.asset_usage = RenderAssetUsages::RENDER_WORLD;

    Ok(images.add(cubemap))
}

fn flip_face_horizontally(data: &mut [u8], width: u32, height: u32) -> Result<(), String> {
    let pixel_count = width as usize * height as usize;
    let bytes_per_pixel = data
        .len()
        .checked_div(pixel_count)
        .filter(|bytes| *bytes > 0 && *bytes * pixel_count == data.len())
        .ok_or_else(|| "a cubemap face has an invalid pixel buffer length".to_owned())?;
    let row_bytes = width as usize * bytes_per_pixel;

    for row in data.chunks_exact_mut(row_bytes) {
        for x in 0..width as usize / 2 {
            let opposite = width as usize - 1 - x;
            for channel in 0..bytes_per_pixel {
                row.swap(
                    x * bytes_per_pixel + channel,
                    opposite * bytes_per_pixel + channel,
                );
            }
        }
    }
    Ok(())
}
