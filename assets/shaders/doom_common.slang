// Common types and resource access for all DOOM shaders.
// Uses goldy_exp virtual-main wrapper types for bindless resource binding.

import goldy_exp;

// ============================================================================
// Pipeline payload — defined once, linked structurally by semantic
// ============================================================================
// Raster IA uses `StaticVertexIn` in doom_static.slang (not this GpuType storage
// layout). Mesh shaders read `BufRO<StaticVertex>` instead of vertex attributes.

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
// Virtual-main resources
// ============================================================================
// Each graphics stage lists only the parameters it reads. Goldy merges them by
// name into one pipeline contract and remaps wrappers onto shared push slots.
//
//   SceneUniforms        scene   — SceneUniforms constant buffer
//   BufRO<float>         lights  — Light levels buffer (read-only)
//   Interpolated<float4> wall    — Wall/sprite atlas
//   Interpolated<float4> flat    — Flat (floor/ceiling) atlas
//   Interpolated<float4> palette — Palette
//   Interpolated<float4> sky     — Sky
//   Filter               nearest — Sampler
//
// Mesh-only extras: BufRO<StaticVertex> static_verts, BufRO<uint> static_indices.

// ============================================================================
// Uniform data uploaded once per frame
// ============================================================================


// ============================================================================
// Cross-platform resource access helpers
// ============================================================================
// Each helper takes a typed wrapper from goldy_exp so that entry points can
// forward their virtual-main parameters directly.

SceneUniforms get_scene(SceneUniforms scene) { return scene; }

Texture2D<float4> wall_atlas(Interpolated<float4> wall)      { return wall; }
Texture2D<float4> flat_atlas(Interpolated<float4> flat)      { return flat; }
Texture2D<float4> palette_tex(Interpolated<float4> palette)  { return palette; }
Texture2D<float4> sky_texture(Interpolated<float4> sky)      { return sky; }
SamplerState      nearest_samp(Filter nearest)               { return nearest; }

float read_light(BufRO<float> lights, int light_index) { return lights[light_index]; }

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
    Interpolated<float4> palette, Filter nearest
) {
    float dist_term = min(1.0, 1.0 - dist_scale / (dist + dist_scale));
    float light = light_level * LIGHT_SCALE - dist_term;
    light = clamp(light, 0.0, 1.0);
    return palette_tex(palette).Sample(nearest_samp(nearest), float2(palette_index, 1.0 - light)).rgb;
}
