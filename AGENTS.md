# goldy-doom Agent Notes

## What This Is
DOOM (1993) port using the `goldy` GPU library. Stress-tests goldy with a real game to surface API gaps and bugs.

## Current State: Boots and renders with doom1.wad

The project **compiles and runs**. Tested against `doom1.wad` (DOOM Shareware, E1M1–E1M9). A window opens, geometry renders, the camera moves with WASD + mouse. The full pipeline works: WAD file → BSP walk → geometry extraction → GPU upload → goldy render loop. Slang shaders use goldy's bindless resource model via push constants.

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
        └── renderer.rs             # Goldy renderer (fully implemented)
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
| 5 | Sky texture | Interpolated | RGBA8 (R=palette_idx), loaded from WAD |
| 6 | Sampler | Filter | Nearest-neighbor, repeat U, clamp V |

**Texture format notes (goldy only supports RGBA8/BGRA8):**
- All textures are converted to RGBA8 on CPU before upload.
- Wall atlas: u16 → RGBA8 (R = low byte / palette index, G = high byte / transparency, B=0, A=255).
- Flat atlas: u8 → RGBA8 (R = palette index, G=0, B=0, A=255).
- Palette: RGB → RGBA8 (R, G, B from source, A=255).

## Known Gaps Surfaced

1. **Texture formats**: goldy only supports RGBA8/BGRA8/RGBA16F/RGBA32F. No R8, RG8, R16 formats. DOOM needs single/two-channel textures for palette-indexed rendering. Workaround: expand to RGBA8 on CPU. *(resolved with workaround)*
2. **Surface depth**: `Surface::new_with_depth` and `DepthStencilState` are now used — depth testing is active. *(resolved)*
3. **Sky texture**: Loaded from WAD via `archive.metadata().sky_for()` and uploaded as a real texture. *(resolved)*

## Known Remaining Issues (from doom1.wad test run)

1. **Unknown linedef special types**: The visitor logs errors for trigger/action linedef types it doesn't handle: 8, 9, 35, 48, 97. These are interactive types (scrolling walls, doors, teleporters) — non-fatal for passive rendering, but doors/scrollers won't animate.
2. **No game logic**: Enemies, items, doors, lifts, and pickups are not simulated — this is a viewer, not a playable game.

## What Needs To Happen Next

1. **Implement missing linedef specials** — At minimum: type 48 (scroll texture left) is cosmetic and common in E1M1. Types 9/35/97 are triggers.
2. **Light animation** — `lights.rs` is wired but the per-frame `light_levels` passed to `render_frame` are all `1.0`. Hook up the actual animated light values from `LightInfo`.
3. **Sprite sorting** — Sprites currently rely on BSP order; add distance-based sort for correct sprite-on-sprite overlap.

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
