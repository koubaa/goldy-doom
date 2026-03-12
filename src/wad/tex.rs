use super::archive::Archive;
use super::image::Image;
use super::name::WadName;
use super::types::{Colormap, Palette, WadTextureHeader, WadTexturePatchRef};
use anyhow::{ensure, Context, Result};
use byteorder::{LittleEndian, ReadBytesExt};
use indexmap::IndexMap;
use log::{error, info};
use std::cmp;
use std::mem;
use std::time::Instant;

#[derive(Copy, Clone, Debug)]
pub struct Bounds {
    pub pos: [f32; 2],
    pub size: [f32; 2],
    pub num_frames: usize,
    pub row_height: usize,
}

pub type Flat = Vec<u8>;
pub type BoundsLookup = IndexMap<WadName, Bounds>;

pub struct TextureDirectory {
    textures: IndexMap<WadName, Image>,
    patches: Vec<(WadName, Option<Image>)>,
    palettes: Vec<Palette>,
    colormaps: Vec<Colormap>,
    flats: IndexMap<WadName, Flat>,
    animated_walls: Vec<Vec<WadName>>,
    animated_flats: Vec<Vec<WadName>>,
}

pub struct MappedPalette {
    pub pixels: Vec<u8>,
    pub colormaps: usize,
}

pub struct TransparentImage {
    pub pixels: Vec<u16>,
    pub size: [usize; 2],
}

pub struct OpaqueImage {
    pub pixels: Vec<u8>,
    pub size: [usize; 2],
}

impl TextureDirectory {
    pub fn from_archive(wad: &Archive) -> Result<TextureDirectory> {
        info!("Reading texture directory...");
        let palettes: Vec<Palette> = wad.required_named_lump(b"PLAYPAL\0")?.read_blobs()?;
        let colormaps: Vec<Colormap> = wad.required_named_lump(b"COLORMAP")?.read_blobs()?;
        info!("  {:4} palettes", palettes.len());
        info!("  {:4} colormaps", colormaps.len());

        let patches = read_patches(wad)?;
        info!("  {:4} patches", patches.len());

        let start_time = Instant::now();
        info!("Reading & assembling textures...");
        let mut textures = IndexMap::new();
        let mut textures_buffer = Vec::new();
        for &lump_name in TEXTURE_LUMP_NAMES {
            let lump = match wad.named_lump(lump_name)? {
                Some(i) => i,
                None => {
                    info!("     0 textures in {}", String::from_utf8_lossy(lump_name));
                    continue;
                }
            };
            textures_buffer.clear();
            lump.read_bytes_into(&mut textures_buffer)?;
            let num_textures = read_textures(&textures_buffer, &patches, &mut textures)?;
            info!("  {:4} textures in {}", num_textures, String::from_utf8_lossy(lump_name));
        }
        info!("Done in {:.2}ms.", start_time.elapsed().as_secs_f64() * 1000.0);

        let flats = read_flats(wad)?;
        info!("  {:4} flats", flats.len());

        let num_sprites = read_sprites(wad, &mut textures)?;
        info!("  {:4} sprites", num_sprites);

        Ok(TextureDirectory {
            patches,
            textures,
            palettes,
            colormaps,
            flats,
            animated_walls: wad.metadata().animations.walls.clone(),
            animated_flats: wad.metadata().animations.flats.clone(),
        })
    }

    pub fn texture(&self, name: WadName) -> Option<&Image> {
        self.textures.get(&name)
    }
    pub fn flat(&self, name: WadName) -> Option<&Flat> {
        self.flats.get(&name)
    }
    pub fn num_palettes(&self) -> usize { self.palettes.len() }
    pub fn palette(&self, index: usize) -> &Palette { &self.palettes[index] }
    pub fn num_colormaps(&self) -> usize { self.colormaps.len() }
    pub fn colormap(&self, index: usize) -> &Colormap { &self.colormaps[index] }

    pub fn build_palette_texture(
        &self,
        palette: usize,
        colormap_start: usize,
        colormap_end: usize,
    ) -> MappedPalette {
        let num_colormaps = colormap_end - colormap_start;
        let mut mapped = vec![0u8; 256 * num_colormaps * 3];
        let palette = &self.palettes[palette];
        for (i_colormap, colormap) in self.colormaps.iter().enumerate().take(colormap_end).skip(colormap_start) {
            let offset = i_colormap * 256 * 3;
            for (i_color, color) in colormap.0.iter().enumerate() {
                mapped[i_color * 3 + offset..][..3]
                    .copy_from_slice(&palette.0[usize::from(*color) * 3..][..3]);
            }
        }
        MappedPalette { pixels: mapped, colormaps: colormap_end - colormap_start + 1 }
    }

    pub fn build_texture_atlas<T>(&self, names_iter: T) -> (TransparentImage, BoundsLookup)
    where T: IntoIterator<Item = WadName>
    {
        let entries = ordered_atlas_entries(&self.animated_walls, |n| self.texture(n), names_iter);
        let max_image_width = if let Some(w) = entries.iter().map(|e| e.image.width()).max() {
            w
        } else {
            return (TransparentImage { pixels: Vec::new(), size: [0, 0] }, BoundsLookup::new());
        };
        let num_pixels: usize = entries.iter().map(|e| e.image.num_pixels()).sum();
        let min_atlas_size = [cmp::min(128, next_pow2(max_image_width)), 128usize];
        let max_size = 4096;

        let mut atlas_size = min_atlas_size;
        loop {
            if atlas_size[0] <= atlas_size[1] {
                if atlas_size[0] == max_size { panic!("Could not fit wall atlas."); }
                atlas_size[0] *= 2;
                atlas_size[1] = 128;
            } else {
                atlas_size[1] *= 2;
            }
            if atlas_size[0] * atlas_size[1] >= num_pixels { break; }
        }

        let mut transposed = false;
        let mut positions: Vec<AtlasPosition> = Vec::with_capacity(entries.len());
        loop {
            let mut offset = [0usize; 2];
            let mut failed = false;
            let mut row_height = 0usize;
            for entry in &entries {
                let size = entry.image.size();
                if offset[0] + size[0] > atlas_size[0] {
                    offset[0] = 0;
                    offset[1] += row_height;
                    row_height = 0;
                }
                if size[1] > row_height { row_height = size[1]; }
                if offset[1] + size[1] > atlas_size[1] { failed = true; break; }
                positions.push(AtlasPosition { offset: [offset[0] as isize, offset[1] as isize], row_height });
                offset[0] += size[0];
            }
            if failed {
                positions.clear();
                atlas_size = [atlas_size[1], atlas_size[0]];
                transposed = !transposed;
                if transposed && atlas_size[0] != atlas_size[1] { continue; }
                transposed = false;
                loop {
                    if atlas_size[0] <= atlas_size[1] {
                        if atlas_size[0] == max_size { panic!("Could not fit wall atlas."); }
                        atlas_size[0] *= 2;
                        atlas_size[1] = 128;
                    } else {
                        atlas_size[1] *= 2;
                    }
                    if atlas_size[0] * atlas_size[1] >= num_pixels { break; }
                }
            } else {
                break;
            }
        }

        assert_eq!(positions.len(), entries.len());
        let mut atlas = Image::new(atlas_size[0], atlas_size[1]).expect("atlas too big");
        let mut bound_map = IndexMap::new();
        for (i, entry) in entries.iter().enumerate() {
            atlas.blit(entry.image, positions[i].offset, true);
            let ref_pos = &positions[i - entry.frame_offset];
            bound_map.insert(entry.name, img_bound(ref_pos, entry));
        }

        let tex = TransparentImage { size: atlas_size, pixels: atlas.into_pixels() };
        info!("Texture atlas size: {:?}", atlas_size);
        (tex, bound_map)
    }

    pub fn build_flat_atlas<T>(&self, names_iter: T) -> (OpaqueImage, BoundsLookup)
    where T: IntoIterator<Item = WadName>
    {
        let names = ordered_atlas_entries(&self.animated_flats, |n| self.flat(n), names_iter);
        let num_names = names.len();
        let width = next_pow2((num_names as f64).sqrt().ceil() as usize * 64);
        let flats_per_row = width / 64;
        let num_rows = (num_names as f64 / flats_per_row as f64).ceil() as usize;
        let height = next_pow2(num_rows * 64);

        let mut offsets = IndexMap::new();
        let mut data = vec![255u8; width * height];
        let (mut row, mut column) = (0, 0);
        info!("Flat atlas size: {}x{}", width, height);
        let mut anim_start_pos = [0.0f32; 2];
        for entry in names {
            let off = [column * 64, row * 64];
            if entry.frame_offset == 0 {
                anim_start_pos = [off[0] as f32, off[1] as f32];
            }
            offsets.insert(entry.name, Bounds {
                pos: anim_start_pos,
                size: [64.0, 64.0],
                num_frames: entry.num_frames,
                row_height: 64,
            });
            for y in 0..64 {
                for x in 0..64 {
                    data[off[0] + x + (y + off[1]) * width] = entry.image[x + y * 64];
                }
            }
            column += 1;
            if column == flats_per_row { column = 0; row += 1; }
        }
        let tex = OpaqueImage { pixels: data, size: [width, height] };
        (tex, offsets)
    }
}

struct AtlasEntry<'a, ImageType> {
    name: WadName,
    image: &'a ImageType,
    frame_offset: usize,
    num_frames: usize,
}

struct AtlasPosition {
    offset: [isize; 2],
    row_height: usize,
}

fn next_pow2(x: usize) -> usize {
    let mut pow2 = 1;
    while pow2 < x { pow2 *= 2; }
    pow2
}

const TEXTURE_LUMP_NAMES: &[&[u8; 8]] = &[b"TEXTURE1", b"TEXTURE2"];

fn read_patches(wad: &Archive) -> Result<Vec<(WadName, Option<Image>)>> {
    let pnames_buffer = wad.required_named_lump(b"PNAMES\0\0")?.read_bytes()?;
    let mut lump = &pnames_buffer[..];
    let num_patches = lump.read_u32::<LittleEndian>().context("Missing PNAMES count")? as usize;
    let mut patches = Vec::with_capacity(num_patches);
    info!("Reading {} patches....", num_patches);
    let start_time = Instant::now();
    let mut image_buffer = Vec::new();
    for i_patch in 0..num_patches {
        let name: WadName = match bincode::deserialize_from(&mut lump) {
            Ok(name) => name,
            Err(e) => { error!("Failed to read patch name {}: {}", i_patch, e); continue; }
        };
        match wad.named_lump(&name)? {
            Some(lump) => {
                image_buffer.clear();
                lump.read_bytes_into(&mut image_buffer)?;
                let image = match Image::from_buffer(&image_buffer) {
                    Ok(i) => Some(i),
                    Err(e) => { error!("Skipping patch `{}`: {}", name, e); None }
                };
                patches.push((name, image));
            }
            None => patches.push((name, None)),
        }
    }
    info!("Done in {:.2}ms.", start_time.elapsed().as_secs_f64() * 1000.0);
    Ok(patches)
}

fn img_bound(pos: &AtlasPosition, entry: &AtlasEntry<Image>) -> Bounds {
    Bounds {
        pos: [pos.offset[0] as f32, pos.offset[1] as f32],
        size: [entry.image.width() as f32, entry.image.height() as f32],
        num_frames: entry.num_frames,
        row_height: pos.row_height,
    }
}

fn ordered_atlas_entries<'a, N, I, L>(
    animations: &'a [Vec<WadName>],
    image_lookup: L,
    names_iter: N,
) -> Vec<AtlasEntry<'a, I>>
where
    N: IntoIterator<Item = WadName>,
    L: Fn(WadName) -> Option<&'a I>,
{
    let mut frames_by_first = IndexMap::new();
    for name in names_iter {
        let maybe = search_for_frame(name, animations);
        let first = maybe.map_or(name, |f| f[0]);
        frames_by_first.insert(first, maybe);
    }
    let mut entries = Vec::with_capacity(frames_by_first.len());
    for (name, maybe_frames) in frames_by_first {
        match maybe_frames {
            Some(frames) => {
                for (off, &n) in frames.iter().enumerate() {
                    if let Some(image) = image_lookup(n) {
                        entries.push(AtlasEntry { name: n, image, frame_offset: off, num_frames: frames.len() });
                    } else {
                        error!("Unable to find texture/sprite: {}", n);
                    }
                }
            }
            None => {
                if let Some(image) = image_lookup(name) {
                    entries.push(AtlasEntry { name, image, frame_offset: 0, num_frames: 1 });
                }
            }
        }
    }
    entries
}

fn search_for_frame(search_for: WadName, animations: &[Vec<WadName>]) -> Option<&[WadName]> {
    animations.iter()
        .find(|anim| anim.iter().any(|&f| f == search_for))
        .map(|a| &a[..])
}

fn read_sprites(wad: &Archive, textures: &mut IndexMap<WadName, Image>) -> Result<usize> {
    let start_index = wad.required_named_lump(b"S_START\0")?.index() + 1;
    let end_index = wad.required_named_lump(b"S_END\0\0\0")?.index();
    info!("Reading {} sprites....", end_index - start_index);
    let start_time = Instant::now();
    let mut image_buffer = Vec::new();
    for index in start_index..end_index {
        let lump = wad.lump_by_index(index)?;
        image_buffer.clear();
        lump.read_bytes_into(&mut image_buffer)?;
        match Image::from_buffer(&image_buffer) {
            Ok(texture) => { textures.insert(lump.name(), texture); }
            Err(e) => { error!("Skipping sprite {}: {}", lump.name(), e); }
        }
    }
    info!("Done in {:.2}ms.", start_time.elapsed().as_secs_f64() * 1000.0);
    Ok(end_index - start_index)
}

fn read_textures(
    lump_buffer: &[u8],
    patches: &[(WadName, Option<Image>)],
    textures: &mut IndexMap<WadName, Image>,
) -> Result<usize> {
    let mut lump = lump_buffer;
    let num_textures = lump.read_u32::<LittleEndian>().context("Missing texture count")? as usize;
    let offsets_end = num_textures * mem::size_of::<u32>();
    ensure!(offsets_end < lump.len(), "Textures lump too small for offsets");
    let mut offsets = &lump[..offsets_end];

    for i_texture in 0..num_textures {
        let offset = offsets.read_u32::<LittleEndian>().expect("offset buffer underrun") as usize;
        ensure!(offset < lump_buffer.len(), "Texture offset out of bounds");
        lump = &lump_buffer[offset..];
        let header: WadTextureHeader = match bincode::deserialize_from(&mut lump) {
            Ok(h) => h,
            Err(e) => { error!("Skipping texture {}: {}", i_texture, e); continue; }
        };
        let mut image = match Image::new_from_header(&header) {
            Ok(img) => img,
            Err(e) => { error!("Skipping texture {}: {}", header.name, e); continue; }
        };
        for i_patch in 0..header.num_patches {
            let pref: WadTexturePatchRef = match bincode::deserialize_from(&mut lump) {
                Ok(p) => p,
                Err(e) => { error!("Skipping patch {} in {}: {}", i_patch, header.name, e); continue; }
            };
            let offset = [
                pref.origin_x as isize,
                if pref.origin_y <= 0 { 0 } else { pref.origin_y as isize },
            ];
            match patches.get(pref.patch as usize) {
                Some((_, Some(patch))) => image.blit(patch, offset, i_patch == 0),
                Some((pn, None)) => error!("PatchRef {} required by {} is missing.", pn, header.name),
                None => error!("PatchRef index {} out of bounds in {}", pref.patch, header.name),
            }
        }
        textures.insert(header.name, image);
    }
    Ok(num_textures)
}

fn read_flats(wad: &Archive) -> Result<IndexMap<WadName, Flat>> {
    let start = wad.required_named_lump(b"F_START\0")?.index();
    let end = wad.required_named_lump(b"F_END\0\0\0")?.index();
    let mut flats = IndexMap::new();
    for i_lump in start..end {
        let lump = wad.lump_by_index(i_lump)?;
        if lump.is_virtual() { continue; }
        flats.insert(lump.name(), lump.read_bytes()?);
    }
    Ok(flats)
}
