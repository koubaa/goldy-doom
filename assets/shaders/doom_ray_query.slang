// Primary-ray compute pass for `--render ray-query`.
// Hits fetch StaticVertex attrs and shade like doom_static; misses sample sky.

import doom_common;
import goldy_exp;


float3 sample_sky(
    float2 ndc,
    RtUniforms u,
    Interpolated<float4> sky,
    Interpolated<float4> palette,
    Filter nearest
) {
    float2 uv = float2(ndc.x, ndc.y) * float2(1.0, -1.0);
    uv = float2(uv.x - 4.0 * u.sky_rot.x / PI, uv.y + 1.0 + u.sky_rot.y);
    float tbs = u.tiled_band_size;
    if (uv.y < 0.0) {
        uv.y = abs(fmod(-uv.y + tbs, tbs * 2.0) - tbs);
    } else if (uv.y >= 2.0) {
        uv.y = abs(fmod(uv.y - 2.0 + tbs, tbs * 2.0) - tbs);
    } else if (uv.y >= 1.0) {
        uv.y = 1.0 - uv.y;
    }
    float palette_index = sky_texture(sky).Sample(nearest_samp(nearest), uv).r;
    return palette_tex(palette).Sample(nearest_samp(nearest), float2(palette_index, 0.0)).rgb;
}

float3 shade_hit(
    StaticVertex v0,
    StaticVertex v1,
    StaticVertex v2,
    float2 bary,
    float dist,
    RtUniforms u,
    BufRO<float> lights,
    Interpolated<float4> wall,
    Interpolated<float4> flat,
    Interpolated<float4> palette,
    Filter nearest,
    out bool transparent
) {
    float w0 = 1.0 - bary.x - bary.y;
    float w1 = bary.x;
    float w2 = bary.y;

    float2 tile_uv = v0.tile_uv * w0 + v1.tile_uv * w1 + v2.tile_uv * w2;
    float2 atlas_uv = v0.atlas_uv * w0 + v1.atlas_uv * w1 + v2.atlas_uv * w2;
    float2 tile_size = v0.tile_size * w0 + v1.tile_size * w1 + v2.tile_size * w2;
    float scroll_rate = v0.scroll_rate * w0 + v1.scroll_rate * w1 + v2.scroll_rate * w2;
    float row_height = v0.row_height * w0 + v1.row_height * w1 + v2.row_height * w2;
    uint num_frames = v0.num_frames;
    uint light_idx = v0.light;
    uint use_flat = v0.use_flat_atlas;

    tile_uv = tile_uv + float2(u.time * scroll_rate, 0.0);
    atlas_uv = animate_atlas_uv(
        atlas_uv, tile_size, row_height, num_frames, u.atlas_size.x, u.time);

    float2 tile_mod = positive_mod(tile_uv, tile_size);
    float2 uv = tile_mod + atlas_uv;
    float2 atlas_sz = use_flat != 0 ? u.flat_atlas_size : u.atlas_size;

    float palette_index;
    transparent = false;
    if (use_flat != 0) {
        palette_index = flat_atlas(flat).Sample(nearest_samp(nearest), uv / atlas_sz).r;
    } else {
        float4 wall_sample = wall_atlas(wall).Sample(nearest_samp(nearest), uv / atlas_sz);
        if (wall_sample.g > 0.5) {
            transparent = true;
            return float3(0.0, 0.0, 0.0);
        }
        palette_index = wall_sample.r;
    }

    float light = read_light(lights, int(light_idx));
    return palette_lookup(palette_index, light, dist, DIST_SCALE_STATIC, palette, nearest);
}

[goldy_compute]
[numthreads(8, 8, 1)]
void cs_main(
    BufRO<RtUniforms> uniforms_buf,
    Accel scene,
    BufRO<StaticVertex> verts,
    BufRO<uint> indices,
    BufRO<float> lights,
    Interpolated<float4> wall,
    Interpolated<float4> flat,
    Interpolated<float4> palette,
    Interpolated<float4> sky,
    Filter nearest,
    DirectSpatial<float4> output,
    ThreadId tid
) {
    RtUniforms u = uniforms_buf[0];
    if (tid.x >= u.width || tid.y >= u.height)
        return;

    float2 uv = (float2(tid.xy) + 0.5) / float2(u.width, u.height);
    float2 ndc = uv * 2.0 - 1.0;
    ndc.y = -ndc.y;

    float4 clip = float4(ndc.x, ndc.y, 1.0, 1.0);
    float4 world_h = mul(u.inv_view_proj, clip);
    float3 world = world_h.xyz / world_h.w;

    RayDesc ray;
    ray.Origin = u.camera_pos;
    ray.TMin = 0.001;
    ray.Direction = normalize(world - u.camera_pos);
    ray.TMax = 10000.0;

    RayQuery<RAY_FLAG_FORCE_OPAQUE> q;
    q.TraceRayInline(scene, RAY_FLAG_FORCE_OPAQUE, 0xFF, ray);
    q.Proceed();

    float3 col = sample_sky(ndc, u, sky, palette, nearest);
    if (q.CommittedStatus() == COMMITTED_TRIANGLE_HIT) {
        uint prim = q.CommittedPrimitiveIndex();
        uint i0 = indices[prim * 3 + 0];
        uint i1 = indices[prim * 3 + 1];
        uint i2 = indices[prim * 3 + 2];
        float2 bary = q.CommittedTriangleBarycentrics();
        float dist = q.CommittedRayT();
        bool transparent = false;
        float3 shaded = shade_hit(
            verts[i0], verts[i1], verts[i2], bary, dist, u,
            lights, wall, flat, palette, nearest, transparent);
        if (!transparent) {
            col = shaded;
        }
    }
    output[tid.xy] = float4(col, 1.0);
}
