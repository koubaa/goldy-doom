# goldy-doom Agent Notes

## What This Is
DOOM (1993) port using the `goldy` GPU library. Stress-tests goldy with a real game to surface API gaps and bugs.

## Current State: Fully wired — ready to test with a WAD

The project **compiles cleanly** (`cargo check` = 0 errors, 0 warnings). The full pipeline is wired: WAD file → BSP walk → geometry extraction → GPU upload → goldy render loop. Slang shaders are ported from the original GLSL and use goldy's bindless resource model via push constants.

## Project Structure
```
goldy-doom/
├── Cargo.toml
├── AGENTS.md
├── assets/
│   ├── meta/doom.toml             # Level metadata (copied from rust-doom)
│   └── shaders/
│       ├── doom_common.slang      # Shared types, resource slots, palette lookup
│       ├── doom_static.slang      # Wall + ceiling/floor geometry (vert + frag)
│       ├── doom_flat.slang        # Floor/ceiling variant (flat atlas, no alpha)
│       ├── doom_sky.slang         # Sky rendering (cylindrical projection)
│       └── doom_sprite.slang      # Billboarded sprites (vert + frag)
└── src/
    ├── main.rs                     # Entry point: CLI args, WAD load, winit event loop
    ├── player.rs                   # FPS camera with WASD + mouse look
    ├── wad/                        # WAD parsing (ported from cristicbz/rust-doom)
    │   ├── mod.rs                  # Public re-exports
    │   ├── archive.rs              # WAD file I/O (anyhow, bincode)
    │   ├── types.rs                # Binary WAD structures (serde/bincode)
    │   ├── name.rs                 # WadName type (8-byte, ASCII-uppercased)
    │   ├── image.rs                # Column-based image format (patches/sprites)
    │   ├── tex.rs                  # Texture atlas building
    │   ├── level.rs                # Level data loading from lumps
    │   ├── light.rs                # Sector light effects (glow, strobe, flicker)
    │   ├── meta.rs                 # doom.toml metadata (sky, animations, linedefs)
    │   ├── util.rs                 # Coord conversion helpers
    │   └── visitor.rs              # BSP walker + geometry emission (1300+ lines)
    └── render/
        ├── mod.rs
        ├── vertex.rs               # StaticVertex, SpriteVertex, SkyVertex (bytemuck)
        ├── lights.rs               # CPU-side light level animation
        ├── level_builder.rs        # LevelVisitor impl → vertex/index buffers
        └── renderer.rs             # Goldy renderer (STUBBED — see TODOs inside)
```

## Porting From rust-doom
- **Error handling**: `failure`/`failchain` → `anyhow`
- **Math**: custom `cgmath`-based `math` crate → `glam` (Vec2, Vec3, Mat4, Quat)
- **2D line geometry**: Inlined `Line2` in `visitor.rs` (signed_distance, intersect_point)
- **Vertex types**: `glium::implement_vertex!` → `bytemuck::Pod + Zeroable`
- **Light cache**: `VecMap<LightInfo>` → `IndexMap<usize, LightInfo>` (free fn, not method, to avoid borrow conflicts)

## Running
```bash
cargo run -- --wad /path/to/doom.wad --meta assets/meta/doom.toml --level 0
```
Opens a window and renders using goldy. Run with a DOOM WAD to see the level.

## Shader Resource Layout (defined in doom_common.slang)

All shaders `import doom_common` which uses push constants to pass bindless resource indices:

| Push Constant | Resource | Access | Contents |
|--------------|----------|--------|----------|
| 0 | SceneUniforms | Broadcast | projection, modelview, atlas_size, time, tiled_band_size |
| 1 | Light levels | Scattered | `StructuredBuffer<float>`, 256 entries, updated per-frame |
| 2 | Wall/sprite atlas | Interpolated | RGBA8 (R=palette_idx, G=transparency) |
| 3 | Flat atlas | Interpolated | RGBA8 (R=palette_idx) |
| 4 | Palette lookup | Interpolated | RGBA8, 256×num_colormaps |
| 5 | Sky texture | Interpolated | (not yet wired) |
| 6 | Sampler | Filter | Nearest-neighbor, repeat U, clamp V |

**Texture format notes (goldy only supports RGBA8/BGRA8):**
- All textures are converted to RGBA8 on CPU before upload.
- Wall atlas: u16 → RGBA8 (R = low byte / palette index, G = high byte / transparency, B=0, A=255).
- Flat atlas: u8 → RGBA8 (R = palette index, G=0, B=0, A=255).
- Palette: RGB → RGBA8 (R, G, B from source, A=255).

## Known Gaps Surfaced

1. **Surface depth**: goldy `Surface` doesn't support depth attachments. Only `RenderTarget` does. DOOM needs depth testing for sprites. Current workaround: no depth, rely on BSP back-to-front ordering.
2. **Texture formats**: goldy only supports RGBA8/BGRA8/RGBA16F/RGBA32F. No R8, RG8, R16 formats. DOOM needs single/two-channel textures for palette-indexed rendering. Workaround: expand to RGBA8 on CPU.
3. **Sky texture**: Not yet loaded from the WAD.

## What Needs To Happen Next

1. **Test with a WAD** — Run `cargo run -- --wad doom.wad` and see what happens
2. **Sky texture** — Load the sky texture from the WAD and wire it up
3. **Depth testing** — When goldy surfaces support depth, add `DepthStencilState` to pipelines

## Key Goldy API Features This Will Stress
- `create_buffer` / `create_buffer_init` (vertex, index, uniform)
- `create_texture` (2D, various formats — u16 palette needs attention)
- `create_render_pipeline` (3 pipelines with different depth/blend/cull)
- `begin_render_pass` with depth target
- `set_push_constants` (bindless resource indices)
- `draw_indexed`
- Surface acquire/present loop

## Dependencies
Defined in Cargo.toml. Key ones: `goldy` (path dep), `winit 0.30`, `glam`, `bytemuck`, `anyhow`, `bincode 1`, `serde`, `toml`, `indexmap`, `byteorder`, `clap`, `log`, `env_logger`.
