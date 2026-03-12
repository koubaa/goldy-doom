// Static geometry shader (walls + floors/ceilings).
// Handles palette-indexed textures, texture animation, scroll, and lighting.

import doom_common;

static const float ANIM_FPS = 8.0 / 35.0;

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

    // Texture animation: advance atlas position to the correct frame
    if (input.num_frames <= 1) {
        output.atlas_uv = input.atlas_uv;
    } else {
        float frame_index = scene.time / ANIM_FPS;
        frame_index = floor(fmod(frame_index, float(input.num_frames)));

        float atlas_u = input.atlas_uv.x + frame_index * input.tile_size.x;
        float n_rows_down = ceil((atlas_u + input.tile_size.x) / scene.atlas_size.x) - 1.0;
        atlas_u += fmod(scene.atlas_size.x - input.atlas_uv.x, input.tile_size.x) * n_rows_down;
        float atlas_v = input.atlas_uv.y + n_rows_down * input.row_height;

        output.atlas_uv = float2(atlas_u, atlas_v);
    }

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
    // Positive modulo — HLSL fmod can return negative for negative inputs
    float2 tile_mod = fmod(fmod(input.tile_uv, input.tile_size) + input.tile_size, input.tile_size);
    float2 uv = tile_mod + input.atlas_uv;
    float2 atlas_sz = input.use_flat_atlas != 0 ? scene.flat_atlas_size : scene.atlas_size;
    float4 atlas_sample = input.use_flat_atlas != 0
        ? flat_atlas().Sample(nearest_samp(), uv / atlas_sz)
        : wall_atlas().Sample(nearest_samp(), uv / atlas_sz);
    if (input.use_flat_atlas == 0 && atlas_sample.g > 0.5) { discard; }
    float3 color = palette_lookup(atlas_sample.r, input.light, input.dist, DIST_SCALE_STATIC);
    return float4(color, 1.0);
}
