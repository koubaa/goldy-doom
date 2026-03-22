// Sky rendering shader.
// Projects sky geometry and samples a cylindrically-mapped sky texture with
// tiling/mirroring for vertical look-up/down.

import doom_common;
import goldy_exp;

// ============================================================================
// Vertex layout — must match render::vertex::SkyVertex (bytemuck)
// ============================================================================

struct SkyVertexIn {
    float3 pos  : POSITION;
    float  _pad : TEXCOORD0;
};

struct SkyVarying {
    float4 position : SV_Position;
    float2 rot      : TEXCOORD0;  // (yaw, pitch-term) from the camera
    float4 proj_pos : TEXCOORD1;  // clip-space position for screen-space UV
};

// ============================================================================
// Vertex shader
// ============================================================================

[shader("vertex")]
SkyVarying vs_main(SkyVertexIn input) {
    SceneUniforms scene = get_scene();
    SkyVarying output;

    float4x4 transform = mul(scene.projection, scene.modelview);

    // Extract camera forward direction from the combined matrix.
    // Column 2 of the combined matrix encodes the forward vector projection.
    float4 forward = float4(transform[0][2], transform[1][2], transform[2][2], transform[3][2]);
    output.rot = float2(atan2(forward.x, forward.z), forward.y / forward.w);

    float4 projected = mul(transform, float4(input.pos, 1.0));
    output.proj_pos = projected;
    output.position = projected;

    return output;
}

// ============================================================================
// Fragment shader
// ============================================================================

[shader("fragment")]
float4 fs_main(SkyVarying input) : SV_Target {
    SceneUniforms scene = get_scene();

    // Screen-space UV from clip-space (perspective divide)
    float2 uv = float2(input.proj_pos.x, input.proj_pos.y) / input.proj_pos.w * float2(1.0, -1.0);

    // Cylindrical mapping: horizontal wraps with camera yaw, vertical shifts with pitch
    uv = float2(uv.x - 4.0 * input.rot.x / PI, uv.y + 1.0 + input.rot.y);

    float tbs = scene.tiled_band_size;

    // Vertical tiling/mirroring for when the player looks up or down
    if (uv.y < 0.0) {
        uv.y = abs(fmod(-uv.y + tbs, tbs * 2.0) - tbs);
    } else if (uv.y >= 2.0) {
        uv.y = abs(fmod(uv.y - 2.0 + tbs, tbs * 2.0) - tbs);
    } else if (uv.y >= 1.0) {
        uv.y = 1.0 - uv.y;
    }

    float palette_index = sky_texture().Sample(nearest_samp(), uv).r;
    float3 color = palette_tex().Sample(nearest_samp(), float2(palette_index, 0.0)).rgb;

    return float4(color, 1.0);
}
