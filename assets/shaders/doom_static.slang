// Static geometry shader (walls + floors/ceilings).
// Handles palette-indexed textures, texture animation, scroll, and lighting.

import doom_common;
import goldy_exp;

// ============================================================================
// Vertex layout — must match render::vertex::StaticVertex GpuType IA
// ============================================================================

struct StaticVertexIn {
    float3 pos            : POSITION;
    float2 atlas_uv       : TEXCOORD0;
    float2 tile_uv        : TEXCOORD1;
    float2 tile_size      : TEXCOORD2;
    float  scroll_rate    : TEXCOORD3;
    float  row_height     : TEXCOORD4;
    uint   num_frames     : TEXCOORD5;
    uint   light          : TEXCOORD6;
    uint   use_flat_atlas : TEXCOORD7;
};

// ============================================================================
// Vertex shader
// ============================================================================

[goldy_vertex]
StaticVarying vs_main(
    SceneUniforms scene, BufRO<float> lights,
    StaticVertexIn input
) {
    SceneUniforms scene_data = get_scene(scene);
    StaticVarying output;

    // Animated tile UV (scrolling walls like conveyor belts)
    output.tile_uv = input.tile_uv + float2(scene_data.time * input.scroll_rate, 0.0);

    output.atlas_uv = animate_atlas_uv(input.atlas_uv, input.tile_size, input.row_height, input.num_frames, scene_data.atlas_size.x, scene_data.time);

    output.tile_size = input.tile_size;
    output.use_flat_atlas = input.use_flat_atlas;

    float4 projected = mul(scene_data.projection, mul(scene_data.modelview, float4(input.pos, 1.0)));
    output.position = projected;
    output.dist = projected.w;

    output.light = read_light(lights, int(input.light));

    return output;
}

// ============================================================================
// Fragment shader
// ============================================================================

[goldy_fragment]
float4 fs_main(
    SceneUniforms scene,
    Interpolated<float4> wall, Interpolated<float4> flat,
    Interpolated<float4> palette, Filter nearest,
    StaticVarying input
) : SV_Target {
    SceneUniforms scene_data = get_scene(scene);
    float2 tile_mod = positive_mod(input.tile_uv, input.tile_size);
    float2 uv = tile_mod + input.atlas_uv;
    float2 atlas_sz = input.use_flat_atlas != 0 ? scene_data.flat_atlas_size : scene_data.atlas_size;
    float palette_index;
    if (input.use_flat_atlas != 0) {
        palette_index = flat_atlas(flat).Sample(nearest_samp(nearest), uv / atlas_sz).r;
    } else {
        float4 wall_sample = wall_atlas(wall).Sample(nearest_samp(nearest), uv / atlas_sz);
        if (wall_sample.g > 0.5) { discard; }
        palette_index = wall_sample.r;
    }
    float3 color = palette_lookup(palette_index, input.light, input.dist, DIST_SCALE_STATIC, palette, nearest);
    return float4(color, 1.0);
}
