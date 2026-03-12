// Floor/ceiling flat shader variant.
// Uses the same vertex format as doom_static but samples from the flat atlas
// (R8_UNORM, opaque, no transparency check) instead of the wall atlas (RG8_UNORM).
//
// NOTE: In the current CPU-side LevelBuilder, walls and flats share one vertex
// buffer and one index buffer with mixed indices. If we want separate flat
// rendering, we'd need to split the index buffers. For now, the static shader
// handles both (the flat atlas texels just have G=0 so the transparency check
// passes). This shader exists for when/if we split the draw calls for flats
// to avoid the unnecessary transparency check.

import doom_common;

struct StaticVarying {
    float4 position   : SV_Position;
    float  dist       : TEXCOORD0;
    float2 tile_uv    : TEXCOORD1;
    float2 atlas_uv   : TEXCOORD2;
    float2 tile_size  : TEXCOORD3;
    float  light      : TEXCOORD4;
};

[shader("fragment")]
float4 fs_main(StaticVarying input) : SV_Target {
    SceneUniforms scene = get_scene();

    float2 uv = fmod(input.tile_uv, input.tile_size) + input.atlas_uv;

    // Flat atlas is R8_UNORM — sample returns palette index directly, no transparency
    float palette_index = flat_atlas().Sample(nearest_samp(), uv / scene.atlas_size);

    float3 color = palette_lookup(palette_index, input.light, input.dist, DIST_SCALE_STATIC);
    return float4(color, 1.0);
}
