// Sprite (billboard) shader.
// Expands sprite quads to always face the camera using the modelview
// matrix's right vector, then performs the same palette-indexed texture
// lookup as the static shader.

import doom_common;
import goldy_exp;

// ============================================================================
// Vertex layout — must match render::vertex::SpriteVertex (bytemuck)
// ============================================================================

struct SpriteVertexIn {
    float3 pos        : POSITION;
    float2 atlas_uv   : TEXCOORD0;
    float2 tile_uv    : TEXCOORD1;
    float2 tile_size  : TEXCOORD2;
    float  local_x    : TEXCOORD3;
    uint   num_frames : TEXCOORD4;
    uint   light      : TEXCOORD5;
    uint   _pad       : TEXCOORD6;
};

struct SpriteVarying {
    float4 position   : SV_Position;
    float  dist       : TEXCOORD0;
    float2 tile_uv    : TEXCOORD1;
    float2 atlas_uv   : TEXCOORD2;
    float2 tile_size  : TEXCOORD3;
    float  light      : TEXCOORD4;
};

// ============================================================================
// Vertex shader
// ============================================================================

[shader("vertex")]
SpriteVarying vs_main(SpriteVertexIn input) {
    SceneUniforms scene = get_scene();
    SpriteVarying output;

    output.tile_uv = input.tile_uv;
    output.atlas_uv = animate_atlas_uv(input.atlas_uv, input.tile_size, input.tile_size.y, input.num_frames, scene.atlas_size.x, scene.time);
    output.tile_size = input.tile_size;

    // Cylindrical billboard: expand along camera-right
    float3 world_pos = billboard_cylindrical_offset(input.pos, modelview_right(scene.modelview), input.local_x);

    float4 projected = mul(scene.projection, mul(scene.modelview, float4(world_pos, 1.0)));
    output.position = projected;
    output.dist = projected.w;

    output.light = read_light(int(input.light));

    return output;
}

// ============================================================================
// Fragment shader
// ============================================================================

[shader("fragment")]
float4 fs_main(SpriteVarying input) : SV_Target {
    SceneUniforms scene = get_scene();

    float2 uv = fmod(input.tile_uv, input.tile_size) + input.atlas_uv;
    float4 atlas_sample = wall_atlas().Sample(nearest_samp(), uv / scene.atlas_size);

    if (atlas_sample.g > 0.5) {
        discard;
    }

    // Sprites use min(light, light * SCALE - dist_term) to avoid over-brightening
    float dist_term = min(1.0, 1.0 - DIST_SCALE_SPRITE / (input.dist + DIST_SCALE_SPRITE));
    float light = min(input.light, input.light * LIGHT_SCALE - dist_term);
    light = clamp(light, 0.0, 1.0);
    float3 color = palette_tex().Sample(nearest_samp(), float2(atlas_sample.r, 1.0 - light)).rgb;

    return float4(color, 1.0);
}
