## 1. R8/RG8 Texture Formats Already Exist — 150+ lines of CPU conversion are unnecessary
<<DONE>>

---

## 2. BufferPool — Replace 6 Allocations With 1
<<DONE>>

---

## 3. `write_data<T>` Instead of Manual bytemuck

Small, but it adds up for ergonomics. Currently:

```383:384:goldy-doom/src/render/renderer.rs
        self.scene_buf
            .write(0, bytemuck::bytes_of(&uniforms))?;
```

Goldy already has:

```124:126:goldy/src/buffer.rs
    pub fn write_data<T: bytemuck::Pod>(&self, offset: u64, data: &[T]) -> Result<()> {
        self.write(offset, bytemuck::cast_slice(data))
    }
```

So: `self.scene_buf.write_data(0, &[uniforms])?;` — one less import, one less way to get the byte cast wrong.

---

## 4. Register `doom_common` as a Shader Library

Currently the shaders use filesystem search paths:

```133:137:goldy-doom/src/render/renderer.rs
        let shader_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("assets")
            .join("shaders");
        let shader_path = shader_dir.to_string_lossy().to_string();
```

Goldy has a proper library registration system:

```112:119:goldy/shaders/goldy_exp/access.slang
// (from the shader docs)
device.register_library(ShaderLibrary::from_source("myeffects", r#"
    module myeffects;
    public float3 glow(float i) { return float3(i, i * 0.8, i * 0.3); }
"#))?;
```

`doom_common.slang` could be registered as a shader library. Then `doom_static.slang`, `doom_sky.slang`, etc. would `import doom_common` via the library system rather than filesystem paths. This is cleaner and is the pattern goldy was designed for.

---

## 5. Surface Format Validation

goldy-doom creates pipelines without checking format compatibility:

```167:178:goldy-doom/src/render/renderer.rs
        let static_pipeline = RenderPipeline::new(
            &self.device,
            &static_shader,
            &static_shader,
            &RenderPipelineDesc {
                vertex_layout: StaticVertex::layout(),
                target_format,
                depth_stencil: static_depth,
                ..Default::default()
            },
        )
```

Goldy has explicit validation for this:

```214:225:goldy/src/surface.rs
    pub fn validate_pipeline_format(&self, pipeline_format: TextureFormat) -> Result<()> {
        let surface_format = self.format();
        if pipeline_format != surface_format {
            anyhow::bail!(
                "Pipeline format mismatch: pipeline uses {:?} but surface uses {:?}.\n\
                 Set RenderPipelineDesc::target_format = surface.format() to fix this.",
```

Adding `surface.validate_pipeline_format(target_format)?` would make format mismatches a clear error at startup.

---

## 6. Vertex Layout — The Biggest Ergonomic Gap (Feedback to Goldy)

The most error-prone code in the whole port is the manual vertex layout specification:

```19:35:goldy-doom/src/render/vertex.rs
impl StaticVertex {
    pub fn layout() -> VertexBufferLayout {
        VertexBufferLayout {
            stride: std::mem::size_of::<Self>() as u32,
            attributes: vec![
                VertexAttribute { location: 0, format: VertexFormat::Float32x3, offset: 0 },   // pos
                VertexAttribute { location: 1, format: VertexFormat::Float32x2, offset: 12 },  // atlas_uv
                VertexAttribute { location: 2, format: VertexFormat::Float32x2, offset: 20 }, // tile_uv
                VertexAttribute { location: 3, format: VertexFormat::Float32x2, offset: 28 }, // tile_size
                VertexAttribute { location: 4, format: VertexFormat::Float32,   offset: 36 }, // scroll_rate
                VertexAttribute { location: 5, format: VertexFormat::Float32,   offset: 40 },  // row_height
                VertexAttribute { location: 6, format: VertexFormat::Uint32,    offset: 44 }, // num_frames
                VertexAttribute { location: 7, format: VertexFormat::Uint32,    offset: 48 }, // light
                VertexAttribute { location: 8, format: VertexFormat::Uint32,    offset: 52 }, // use_flat_atlas
            ],
        }
    }
}
```

9 attributes, 9 manual offsets. If you add a field and forget to update the offsets, **silent corruption**. This is the OpenGL `glVertexAttribPointer` pattern verbatim. Goldy doesn't have a `#[derive(VertexLayout)]` macro yet, but this is exactly the feedback the experiment should produce — goldy needs one. A derive macro that reads `#[repr(C)]` fields and generates the layout automatically would eliminate this entire class of bugs.

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