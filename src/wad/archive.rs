use super::meta::WadMetadata;
use super::name::IntoWadName;
use super::types::{WadInfo, WadLump, WadName};
use anyhow::{ensure, Context, Result};
use indexmap::IndexMap;
use log::info;
use serde::de::DeserializeOwned;
use std::borrow::Borrow;
use std::cell::RefCell;
use std::fmt::Debug;
use std::fs::File;
use std::hash::Hash;
use std::io::{BufReader, Read, Seek, SeekFrom, Take};
use std::mem;
use std::path::Path;

#[derive(Debug)]
pub struct Archive {
    file: RefCell<BufReader<File>>,
    index_map: IndexMap<WadName, usize>,
    lumps: Vec<LumpInfo>,
    levels: Vec<usize>,
    meta: WadMetadata,
}

struct OpenWad {
    file: RefCell<BufReader<File>>,
    index_map: IndexMap<WadName, usize>,
    lumps: Vec<LumpInfo>,
    levels: Vec<usize>,
}

impl Archive {
    pub fn open<W, M>(wad_path: &W, meta_path: &M) -> Result<Archive>
    where
        W: AsRef<Path> + Debug,
        M: AsRef<Path> + Debug,
    {
        let wad_path = wad_path.as_ref().to_owned();
        let meta_path = meta_path.as_ref().to_owned();
        info!("Loading wad file '{:?}'...", wad_path);
        let OpenWad {
            file,
            index_map,
            lumps,
            levels,
        } = Archive::open_wad(&wad_path)?;
        info!("Loading metadata file '{:?}'...", meta_path);
        let meta = WadMetadata::from_file(&meta_path)?;

        Ok(Archive {
            file,
            index_map,
            lumps,
            levels,
            meta,
        })
    }

    fn open_wad(wad_path: &Path) -> Result<OpenWad> {
        let mut file = BufReader::new(
            File::open(wad_path).with_context(|| format!("Failed to open {:?}", wad_path))?,
        );

        let header: WadInfo = bincode::deserialize_from(&mut file)
            .context("Could not read WAD header")?;

        ensure!(
            header.identifier == *IWAD_HEADER,
            "Invalid header identifier: {}",
            String::from_utf8_lossy(&header.identifier)
        );

        let mut lumps = Vec::with_capacity(header.num_lumps as usize);
        let mut levels = Vec::with_capacity(64);
        let mut index_map = IndexMap::new();

        file.seek(SeekFrom::Start(header.info_table_offset as u64))
            .with_context(|| format!("Seeking to info_table_offset at {}", header.info_table_offset))?;

        for i_lump in 0..header.num_lumps {
            let fileinfo: WadLump = bincode::deserialize_from(&mut file)
                .with_context(|| format!("Invalid lump info for lump {}", i_lump))?;

            index_map.insert(fileinfo.name, lumps.len());
            lumps.push(LumpInfo {
                name: fileinfo.name,
                offset: fileinfo.file_pos as u64,
                size: fileinfo.size as usize,
            });

            if &fileinfo.name == b"THINGS\0\0" {
                assert!(i_lump > 0);
                levels.push((i_lump - 1) as usize);
            }
        }

        Ok(OpenWad {
            file: RefCell::new(file),
            index_map,
            lumps,
            levels,
        })
    }

    pub fn metadata(&self) -> &WadMetadata {
        &self.meta
    }

    pub fn num_levels(&self) -> usize {
        self.levels.len()
    }

    pub fn level_lump(&self, level_index: usize) -> Result<LumpReader<'_>> {
        self.lump_by_index(self.levels[level_index])
    }

    pub fn required_named_lump<'a, Q>(&self, name: &'a Q) -> Result<LumpReader>
    where
        &'a Q: IntoWadName,
    {
        let name: WadName = name.into_wad_name()?;
        self.named_lump(&name)?
            .ok_or_else(|| anyhow::anyhow!("Missing required lump {:?}", name))
    }

    pub fn named_lump<Q>(&self, name: &Q) -> Result<Option<LumpReader<'_>>>
    where
        WadName: Borrow<Q>,
        Q: Hash + Eq,
    {
        match self.index_map.get(name) {
            Some(&index) => self.lump_by_index(index).map(Some),
            None => Ok(None),
        }
    }

    pub fn lump_by_index(&self, index: usize) -> Result<LumpReader<'_>> {
        Ok(LumpReader {
            archive: self,
            info: self
                .lumps
                .get(index)
                .ok_or_else(|| anyhow::anyhow!("Lump index {} out of bounds", index))?,
            index,
        })
    }
}

#[derive(Copy, Clone, Debug)]
pub struct LumpReader<'a> {
    archive: &'a Archive,
    info: &'a LumpInfo,
    index: usize,
}

impl<'a> LumpReader<'a> {
    pub fn index(&self) -> usize {
        self.index
    }

    pub fn name(&self) -> WadName {
        self.info.name
    }

    pub fn is_virtual(&self) -> bool {
        self.info.size == 0
    }

    pub fn decode_vec<T: DeserializeOwned>(&self) -> Result<Vec<T>> {
        let info = *self.info;
        let index = self.index;
        self.read(|mut file| {
            let element_size = mem::size_of::<T>();
            let num_elements = info.size / element_size;
            ensure!(
                info.size > 0 && (info.size % element_size == 0),
                "Bad lump size in lump {} '{}': total={}, element={}",
                index,
                info.name,
                info.size,
                element_size,
            );
            (0..num_elements)
                .map(|i_element| {
                    bincode::deserialize_from(&mut file)
                        .with_context(|| format!("Bad element {} in lump {} '{}'", i_element, index, info.name))
                })
                .collect()
        })
    }

    pub fn read_blobs<B>(&self) -> Result<Vec<B>>
    where
        B: Default + AsMut<[u8]>,
    {
        let info = *self.info;
        let index = self.index;
        self.read(|file| {
            let blob_size = B::default().as_mut().len();
            assert!(blob_size > 0);
            ensure!(
                info.size > 0 && (info.size % blob_size) == 0,
                "Bad lump size in lump {} '{}': total={}, blob={}",
                index,
                info.name,
                info.size,
                blob_size,
            );
            let num_blobs = info.size / blob_size;
            let mut blobs = Vec::with_capacity(num_blobs);
            for _ in 0..num_blobs {
                blobs.push(B::default());
                file.read_exact(blobs.last_mut().unwrap().as_mut())
                    .with_context(|| format!("Reading lump {} '{}'", index, info.name))?;
            }
            Ok(blobs)
        })
    }

    pub fn read_bytes_into(&self, bytes: &mut Vec<u8>) -> Result<()> {
        let info = *self.info;
        let index = self.index;
        self.read(|file| {
            let old_size = bytes.len();
            bytes.resize(old_size + info.size, 0u8);
            file.read_exact(&mut bytes[old_size..])
                .with_context(|| format!("Reading lump {} '{}'", index, info.name))?;
            Ok(())
        })
    }

    pub fn read_bytes(&self) -> Result<Vec<u8>> {
        let mut bytes = Vec::new();
        self.read_bytes_into(&mut bytes)?;
        Ok(bytes)
    }

    fn read<F, T>(&self, with: F) -> Result<T>
    where
        F: FnOnce(&mut Take<&mut BufReader<File>>) -> Result<T>,
    {
        let info = *self.info;
        let index = self.index;
        let mut file = self.archive.file.borrow_mut();
        file.seek(SeekFrom::Start(info.offset))
            .with_context(|| format!("Seeking to lump {} '{}'", index, info.name))?;
        with(&mut Read::take(&mut *file, info.size as u64))
    }
}

#[derive(Copy, Clone, Debug)]
struct LumpInfo {
    name: WadName,
    offset: u64,
    size: usize,
}

const IWAD_HEADER: &[u8; 4] = b"IWAD";
