## 1. R8/RG8 Texture Formats Already Exist — 150+ lines of CPU conversion are unnecessary
<<DONE>>
---

## 2. BufferPool — Replace 6 Allocations With 1
<<DONE>>
---

## 3. `write_data<T>` Instead of Manual bytemuck
<<DONE>>
---

## 4. Register `doom_common` as a Shader Library
<<DONE>>

`doom_common.slang` is now registered via `Device::register_library()` before shader compilation. Shaders use `ShaderModule::from_slang()` with no filesystem paths. Also: `positive_mod` and `billboard_cylindrical_offset`/`modelview_right` added to goldy_exp; `animate_atlas_uv` centralized in doom_common; doom_sky uses goldy_exp's PI.

---

## 5. Surface Format Validation
<<DONE>>
---

## 6. Vertex Layout — The Biggest Ergonomic Gap (Feedback to Goldy)
<<DONE>>
---

## 7. Cross-Language Type Coherence — The SceneUniforms Problem

`SceneUniforms` is defined in both Rust and Slang with no validation:

```17:27:goldy-doom/src/render/renderer.rs
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct SceneUniforms {
    pub projection: [[f32; 4]; 4],
    pub modelview: [[f32; 4]; 4],
    pub atlas_size: [f32; 2],
    pub flat_atlas_size: [f32; 2],
    pub time: f32,
    pub tiled_band_size: f32,
}
```

```31:38:goldy-doom/assets/shaders/doom_common.slang
struct SceneUniforms {
    float4x4 projection;
    float4x4 modelview;
    float2   atlas_size;
    float2   flat_atlas_size;
    float    time;
    float    tiled_band_size;
};
```

The abstract-gpu.md explicitly calls this out:

> "If either side changes — a field is added, reordered, or the packing rules differ — the GPU silently reads garbage."

Goldy's design doc proposes runtime layout validation via Slang reflection. goldy-doom is the perfect test case to implement this — compare `std::mem::size_of::<SceneUniforms>()` and `offset_of!` against Slang's reflection data at shader load time.

---

## Summary: What Goldy-Doom Proves (and What It Should Push)

| Area | Current (1:1 Port) | Goldy-Idiomatic | Impact |
|------|-------------------|-----------------|--------|
| Texture formats | CPU expand u8/u16→RGBA8 | Use R8Unorm, Rg8Unorm natively | -150 LOC, -4x memory |
| Buffer allocation | 6 separate allocations | BufferPool + views | Cleaner, fewer allocs |
| Buffer writes | Manual bytemuck::bytes_of | `write_data<T>()` | Less error-prone |
| Shader loading | Filesystem search paths | `register_library()` | Cleaner |
| Format safety | None | `validate_pipeline_format()` | Catches bugs at init |
| Vertex layouts | Manual offset math | Needs `#[derive(VertexLayout)]` | **Feedback to goldy** |
| Type coherence | Hope and prayer | Slang reflection validation | **Feedback to goldy** |

Items 1-5 are things goldy already supports that goldy-doom doesn't use. Items 6-7 are gaps this experiment surfaces that goldy should fill. That's exactly what "stress-testing goldy with a real game" was supposed to produce.