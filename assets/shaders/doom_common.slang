// Common types and resource access for all DOOM shaders.
// Uses goldy_exp push constants to pass bindless resource indices per-draw.

import goldy_exp;

// ============================================================================
// Push constant slot assignments
// ============================================================================
// Rust side: pass.set_push_constants_raw(&[scene, lights, wall, flat, palette, sky, sampler])
//
// Slot 0: SceneUniforms buffer index  (Broadcast)
// Slot 1: Light levels buffer index   (Scattered, StructuredBuffer<float>)
// Slot 2: Wall/sprite atlas tex index (Interpolated)
// Slot 3: Flat atlas tex index        (Interpolated)
// Slot 4: Palette tex index           (Interpolated)
// Slot 5: Sky tex index               (Interpolated)
// Slot 6: Sampler index               (Filter)

static const uint PC_SCENE   = 0;
static const uint PC_LIGHTS  = 1;
static const uint PC_WALL    = 2;
static const uint PC_FLAT    = 3;
static const uint PC_PALETTE = 4;
static const uint PC_SKY     = 5;
static const uint PC_SAMPLER = 6;

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
// Cross-platform push constant access
// ============================================================================

// Use goldy_exp's dynamic access functions (goldy_dyn_*) which read push
// constant slot indices and return typed resources. These handle DX12/SPIRV/Metal
// platform differences internally.

SceneUniforms get_scene() { return goldy_dyn_broadcast<SceneUniforms>(PC_SCENE); }

Texture2D<float4> wall_atlas()   { return goldy_dyn_interpolated<float4>(PC_WALL); }
Texture2D<float4> flat_atlas()   { return goldy_dyn_interpolated<float4>(PC_FLAT); }
Texture2D<float4> palette_tex()  { return goldy_dyn_interpolated<float4>(PC_PALETTE); }
Texture2D<float4> sky_texture()  { return goldy_dyn_interpolated<float4>(PC_SKY); }
SamplerState      nearest_samp() { return goldy_dyn_filter(PC_SAMPLER); }

float read_light(int light_index) {
    RWStructuredBuffer<float> lights = goldy_dyn_scattered<float>(PC_LIGHTS);
    return lights[light_index];
}

// ============================================================================
// Palette-based color lookup (shared by static + sprite fragment shaders)
// ============================================================================

static const float DIST_SCALE_STATIC = 0.9;
static const float DIST_SCALE_SPRITE = 1.0;
static const float LIGHT_SCALE       = 2.0;

float3 palette_lookup(float palette_index, float light_level, float dist, float dist_scale) {
    float dist_term = min(1.0, 1.0 - dist_scale / (dist + dist_scale));
    float light = light_level * LIGHT_SCALE - dist_term;
    light = clamp(light, 0.0, 1.0);
    return palette_tex().Sample(nearest_samp(), float2(palette_index, 1.0 - light)).rgb;
}
