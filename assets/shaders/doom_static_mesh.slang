// Mesh-shader static geometry for `--render mesh`.
// Same clip transform + varyings as doom_static.slang vs_main; reuses fs_main logic.

import doom_common;
import goldy_exp;

// Must match render::vertex::StaticVertex (bytemuck / StructuredBuffer).
struct StaticVertex {
    float3 pos;
    float  _pad; // matches Rust StaticVertex / HLSL float3 packing
    float2 atlas_uv;
    float2 tile_uv;
    float2 tile_size;
    float  scroll_rate;
    float  row_height;
    uint   num_frames;
    uint   light;
    uint   use_flat_atlas;
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

StaticVarying shade_vertex(SceneUniforms scene_data, BufRO<float> lights, StaticVertex input) {
    StaticVarying output;
    output.tile_uv = input.tile_uv + float2(scene_data.time * input.scroll_rate, 0.0);
    output.atlas_uv = animate_atlas_uv(
        input.atlas_uv, input.tile_size, input.row_height, input.num_frames,
        scene_data.atlas_size.x, scene_data.time);
    output.tile_size = input.tile_size;
    output.use_flat_atlas = input.use_flat_atlas;
    float4 projected = mul(scene_data.projection, mul(scene_data.modelview, float4(input.pos, 1.0)));
    output.position = projected;
    output.dist = projected.w;
    output.light = read_light(lights, int(input.light));
    return output;
}

// Each workgroup consumes 64 consecutive triangles (192 verts). Indices must be
// padded to a multiple of 64 triangles so every lane is in-bounds.
[goldy_mesh]
[numthreads(64, 1, 1)]
[outputtopology("triangle")]
void mesh_main(
    SceneUniforms scene,
    BufRO<float> lights,
    Interpolated<float4> wall,
    Interpolated<float4> flat,
    Interpolated<float4> palette,
    Interpolated<float4> sky,
    Filter nearest,
    BufRO<StaticVertex> static_verts,
    BufRO<uint> static_indices,
    out vertices StaticVarying verts[192],
    out indices uint3 tris[64],
    GroupId gid,
    GroupThreadId ltid
) {
    SetMeshOutputCounts(192, 64);

    SceneUniforms scene_data = get_scene(scene);
    // ThreadId is SV_DispatchThreadID (global) — only group 0 stays in 0..63.
    // Mesh output arrays are per-group; must use GroupThreadId (SV_GroupThreadID).
    uint tri = gid.x * 64 + ltid.x;
    uint i0 = static_indices[tri * 3 + 0];
    uint i1 = static_indices[tri * 3 + 1];
    uint i2 = static_indices[tri * 3 + 2];

    uint base = ltid.x * 3;
    verts[base + 0] = shade_vertex(scene_data, lights, static_verts[i0]);
    verts[base + 1] = shade_vertex(scene_data, lights, static_verts[i1]);
    verts[base + 2] = shade_vertex(scene_data, lights, static_verts[i2]);
    tris[ltid.x] = uint3(base + 0, base + 1, base + 2);
}

[goldy_fragment]
float4 fs_main(
    SceneUniforms scene, BufRO<float> lights,
    Interpolated<float4> wall, Interpolated<float4> flat,
    Interpolated<float4> palette, Interpolated<float4> sky,
    Filter nearest,
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
