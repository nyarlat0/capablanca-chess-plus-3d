// Samples the planar reflection render target in screen space. The reflected
// camera uses the mirrored main-camera projection, so a point on the y=0 board
// plane lands at the same normalized screen coordinate in both passes.

#import bevy_pbr::{
    forward_io::{VertexOutput, FragmentOutput},
    mesh_view_bindings::view,
    pbr_bindings::{emissive_texture, emissive_sampler},
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{apply_pbr_lighting, main_pass_post_lighting_processing}
}

struct PlanarReflectionMaterial {
    // X is reflection strength. Highlights render as separate unlit geometry.
    reflection_strength: vec4<f32>,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(100) var<uniform> material: PlanarReflectionMaterial;

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    var pbr_input = pbr_input_from_standard_material(in, is_front);

    // The reflection image occupies the whole render target but is capped on
    // high-DPI displays, hence normalized sampling rather than textureLoad.
    let reflection_uv = clamp(
        (in.position.xy - view.viewport.xy) / view.viewport.zw,
        vec2<f32>(0.0),
        vec2<f32>(1.0)
    );
    let reflected = textureSample(emissive_texture, emissive_sampler, reflection_uv);

    // pbr_input sampled the reflection image using the mesh UVs because it is
    // bound as the StandardMaterial emissive texture. Remove that accidental
    // emissive contribution before evaluating the normal PBR lighting.
    pbr_input.material.emissive = vec4<f32>(0.0, 0.0, 0.0, 1.0);

    var out: FragmentOutput;
    out.color = apply_pbr_lighting(pbr_input);

    // Rough regions keep their diffuse appearance while polished regions show
    // a clear planar image. Alpha masks out the transparent target background,
    // so pixels without reflected geometry do not darken the marble.
    let smoothness = 1.0 - clamp(pbr_input.material.perceptual_roughness, 0.0, 1.0);
    let ndotv = clamp(dot(pbr_input.N, pbr_input.V), 0.0, 1.0);
    let fresnel = 0.04 + 0.96 * pow(1.0 - ndotv, 5.0);
    let strength = material.reflection_strength.x;
    let reflection_weight = clamp(
        reflected.a * strength * smoothness * mix(0.75, 1.0, fresnel),
        0.0,
        0.65
    );
    out.color = vec4<f32>(
        mix(out.color.rgb, reflected.rgb, reflection_weight),
        out.color.a
    );

    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
    return out;
}
