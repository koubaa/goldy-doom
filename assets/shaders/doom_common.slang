// Common types and resource access for all DOOM shaders.
// Uses goldy_exp push constants to pass bindless resource indices per-draw.

import goldy_exp;

// ============================================================================
// Push constant slot layout
// ============================================================================
// Every entry point receives these 7 uniform uint params in declaration order:
//
//   uniform uint scene_slot    — SceneUniforms broadcast buffer
//   uniform uint lights_slot   — Light levels buffer (StructuredBuffer<float> / read-only SRV on DX12)
//   uniform uint wall_slot     — Wall/sprite atlas Texture2D
//   uniform uint flat_slot     — Flat (floor/ceiling) atlas Texture2D
//   uniform uint palette_slot  — Palette Texture2D
//   uniform uint sky_slot      — Sky Texture2D
//   uniform uint sampler_slot  — Sampler
//
// Rust side: pass.set_push_constants_raw(&[scene, lights, wall, flat, palette, sky, sampler])

// ============================================================================
// Uniform data uploaded once per frame
// ============================================================================

struct SceneUniforms {
    float4x4 projection;
    float4x4 modelview;
    float2   atlas_size;      // wall/sprite atlas dimensions in pixels
    float2   flat_atlas_size; // flat (floor/ceiling) atlas dimensions in pixels
    float    time;
    float    tiled_band_size; // sky tiling parameter
};

// ============================================================================
// Cross-platform resource access helpers
// ============================================================================
// Each helper takes its bindless slot index as an explicit parameter so that
// entry points can forward their uniform uint params here.

SceneUniforms get_scene(uint scene_slot) { return goldy_broadcast<SceneUniforms>(scene_slot); }

Texture2D<float4> wall_atlas(uint wall_slot)     { return goldy_interpolated<float4>(wall_slot); }
Texture2D<float4> flat_atlas(uint flat_slot)     { return goldy_interpolated<float4>(flat_slot); }
Texture2D<float4> palette_tex(uint palette_slot) { return goldy_interpolated<float4>(palette_slot); }
Texture2D<float4> sky_texture(uint sky_slot)     { return goldy_interpolated<float4>(sky_slot); }
SamplerState      nearest_samp(uint sampler_slot){ return goldy_filter(sampler_slot); }

float read_light(uint lights_slot, int light_index) {
    ReadOnlyBuffer<float> lights = goldy_buf_ro<float>(lights_slot);
    return lights[light_index];
}

// ============================================================================
// Palette-based color lookup (shared by static + sprite fragment shaders)
// ============================================================================

static const float DIST_SCALE_STATIC = 0.9;
static const float DIST_SCALE_SPRITE = 1.0;
static const float LIGHT_SCALE       = 2.0;
static const float ANIM_FPS         = 8.0 / 35.0;

// Texture atlas animation for sprite sheets. Returns animated atlas UV.
// row_height: vertical advance per row (static uses row_height, sprite uses tile_size.y)
float2 animate_atlas_uv(float2 atlas_uv, float2 tile_size, float row_height, uint num_frames, float atlas_width, float time) {
    if (num_frames <= 1) return atlas_uv;
    float frame_index = floor(fmod(time / ANIM_FPS, float(num_frames)));
    float atlas_u = atlas_uv.x + frame_index * tile_size.x;
    float n_rows_down = ceil((atlas_u + tile_size.x) / atlas_width) - 1.0;
    atlas_u += fmod(atlas_width - atlas_uv.x, tile_size.x) * n_rows_down;
    float atlas_v = atlas_uv.y + n_rows_down * row_height;
    return float2(atlas_u, atlas_v);
}

float3 palette_lookup(
    float palette_index, float light_level, float dist, float dist_scale,
    uint palette_slot, uint sampler_slot
) {
    float dist_term = min(1.0, 1.0 - dist_scale / (dist + dist_scale));
    float light = light_level * LIGHT_SCALE - dist_term;
    light = clamp(light, 0.0, 1.0);
    return palette_tex(palette_slot).Sample(nearest_samp(sampler_slot), float2(palette_index, 1.0 - light)).rgb;
}
