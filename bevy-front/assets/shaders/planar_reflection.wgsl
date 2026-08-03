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
    // X is reflection strength and Y is maximum blur radius in target pixels.
    reflection_strength: vec4<f32>,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(100) var<uniform> material: PlanarReflectionMaterial;

fn reflection_sample(uv: vec2<f32>) -> vec4<f32> {
    return textureSample(
        emissive_texture,
        emissive_sampler,
        clamp(uv, vec2<f32>(0.0), vec2<f32>(1.0))
    );
}

// Deterministic nine-tap tent filter. Samples are accumulated as premultiplied
// alpha so transparent pixels around a reflected silhouette soften its edge
// without producing a dark fringe. Unlike temporal/SSR filtering this cannot
// introduce noise or temporal shimmer.
fn rough_planar_reflection(
    uv: vec2<f32>,
    perceptual_roughness: f32,
) -> vec4<f32> {
    let center = reflection_sample(uv);
    let target_size = max(
        vec2<f32>(textureDimensions(emissive_texture)),
        vec2<f32>(1.0)
    );
    let blur_pixels = material.reflection_strength.y
        * perceptual_roughness * perceptual_roughness;
    let offset = vec2<f32>(blur_pixels) / target_size;
    let horizontal = vec2<f32>(offset.x, 0.0);
    let vertical = vec2<f32>(0.0, offset.y);
    let diagonal_a = offset * 0.70710678;
    let diagonal_b = vec2<f32>(diagonal_a.x, -diagonal_a.y);

    var premultiplied = center.rgb * center.a * 0.25;
    var alpha = center.a * 0.25;

    let horizontal_positive = reflection_sample(uv + horizontal);
    let horizontal_negative = reflection_sample(uv - horizontal);
    let vertical_positive = reflection_sample(uv + vertical);
    let vertical_negative = reflection_sample(uv - vertical);
    premultiplied += horizontal_positive.rgb * horizontal_positive.a * 0.09375;
    premultiplied += horizontal_negative.rgb * horizontal_negative.a * 0.09375;
    premultiplied += vertical_positive.rgb * vertical_positive.a * 0.09375;
    premultiplied += vertical_negative.rgb * vertical_negative.a * 0.09375;
    alpha += (
        horizontal_positive.a + horizontal_negative.a
        + vertical_positive.a + vertical_negative.a
    ) * 0.09375;

    let diagonal_a_positive = reflection_sample(uv + diagonal_a);
    let diagonal_a_negative = reflection_sample(uv - diagonal_a);
    let diagonal_b_positive = reflection_sample(uv + diagonal_b);
    let diagonal_b_negative = reflection_sample(uv - diagonal_b);
    premultiplied += diagonal_a_positive.rgb * diagonal_a_positive.a * 0.09375;
    premultiplied += diagonal_a_negative.rgb * diagonal_a_negative.a * 0.09375;
    premultiplied += diagonal_b_positive.rgb * diagonal_b_positive.a * 0.09375;
    premultiplied += diagonal_b_negative.rgb * diagonal_b_negative.a * 0.09375;
    alpha += (
        diagonal_a_positive.a + diagonal_a_negative.a
        + diagonal_b_positive.a + diagonal_b_negative.a
    ) * 0.09375;

    return vec4<f32>(premultiplied / max(alpha, 0.0001), alpha);
}

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
    let roughness = clamp(pbr_input.material.perceptual_roughness, 0.0, 1.0);
    let reflected = rough_planar_reflection(reflection_uv, roughness);

    // pbr_input sampled the reflection image using the mesh UVs because it is
    // bound as the StandardMaterial emissive texture. Remove that accidental
    // emissive contribution before evaluating the normal PBR lighting.
    pbr_input.material.emissive = vec4<f32>(0.0, 0.0, 0.0, 1.0);

    var out: FragmentOutput;
    out.color = apply_pbr_lighting(pbr_input);

    // Rough regions keep their diffuse appearance while polished regions show
    // a clear planar image. Alpha masks out the transparent target background,
    // so pixels without reflected geometry do not darken the marble.
    let smoothness = 1.0 - roughness;
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
