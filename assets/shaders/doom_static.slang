// Static geometry shader (walls + floors/ceilings).
// Handles palette-indexed textures, texture animation, scroll, and lighting.

import doom_common;
import goldy_exp;

// ============================================================================
// Vertex layout — must match render::vertex::StaticVertex (bytemuck)
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

struct StaticVarying {
    float4 position       : SV_Position;
    float  dist           : TEXCOORD0;
    float2 tile_uv        : TEXCOORD1;
    float2 atlas_uv       : TEXCOORD2;
    float2 tile_size      : TEXCOORD3;
    float  light          : TEXCOORD4;
    uint   use_flat_atlas : TEXCOORD5;
};

// ============================================================================
// Vertex shader
// ============================================================================

[shader("vertex")]
StaticVarying vs_main(StaticVertexIn input) {
    SceneUniforms scene = get_scene();
    StaticVarying output;

    // Animated tile UV (scrolling walls like conveyor belts)
    output.tile_uv = input.tile_uv + float2(scene.time * input.scroll_rate, 0.0);

    output.atlas_uv = animate_atlas_uv(input.atlas_uv, input.tile_size, input.row_height, input.num_frames, scene.atlas_size.x, scene.time);

    output.tile_size = input.tile_size;
    output.use_flat_atlas = input.use_flat_atlas;

    float4 projected = mul(scene.projection, mul(scene.modelview, float4(input.pos, 1.0)));
    output.position = projected;
    output.dist = projected.w;

    output.light = read_light(int(input.light));

    return output;
}

// ============================================================================
// Fragment shader
// ============================================================================

[shader("fragment")]
float4 fs_main(StaticVarying input) : SV_Target {
    SceneUniforms scene = get_scene();
    float2 tile_mod = positive_mod(input.tile_uv, input.tile_size);
    float2 uv = tile_mod + input.atlas_uv;
    float2 atlas_sz = input.use_flat_atlas != 0 ? scene.flat_atlas_size : scene.atlas_size;
    float palette_index;
    if (input.use_flat_atlas != 0) {
        palette_index = flat_atlas().Sample(nearest_samp(), uv / atlas_sz);
    } else {
        float2 wall_sample = wall_atlas().Sample(nearest_samp(), uv / atlas_sz);
        if (wall_sample.g > 0.5) { discard; }
        palette_index = wall_sample.r;
    }
    float3 color = palette_lookup(palette_index, input.light, input.dist, DIST_SCALE_STATIC);
    return float4(color, 1.0);
}
