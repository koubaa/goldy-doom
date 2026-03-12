use super::name::WadName;
use super::types::{SpecialType, ThingType, WadCoord};
use anyhow::{Context, Result};
use indexmap::IndexMap;
use log::{error, warn};
use regex::Regex;
use serde::{de::Error as SerdeDeError, Deserialize, Deserializer};
use std::fmt;
use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::result::Result as StdResult;
use std::str::FromStr;

#[derive(Deserialize)]
pub struct SkyMetadata {
    #[serde(deserialize_with = "deserialize_name_from_str")]
    pub texture_name: WadName,
    #[serde(deserialize_with = "deserialize_regex_from_str")]
    pub level_pattern: Regex,
    pub tiled_band_size: f32,
}

impl fmt::Debug for SkyMetadata {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SkyMetadata")
            .field("texture_name", &self.texture_name)
            .field("tiled_band_size", &self.tiled_band_size)
            .finish()
    }
}

#[derive(Debug, Deserialize)]
pub struct AnimationMetadata {
    #[serde(deserialize_with = "deserialize_name_from_vec_vec_str")]
    pub flats: Vec<Vec<WadName>>,
    #[serde(deserialize_with = "deserialize_name_from_vec_vec_str")]
    pub walls: Vec<Vec<WadName>>,
}

#[derive(Debug, Deserialize)]
pub struct ThingMetadata {
    pub thing_type: ThingType,
    #[serde(deserialize_with = "deserialize_name_from_str")]
    pub sprite: WadName,
    pub sequence: String,
    pub hanging: bool,
    pub radius: u32,
}

#[derive(Debug, Deserialize)]
pub struct ThingDirectoryMetadata {
    pub decorations: Vec<ThingMetadata>,
    pub weapons: Vec<ThingMetadata>,
    pub powerups: Vec<ThingMetadata>,
    pub artifacts: Vec<ThingMetadata>,
    pub ammo: Vec<ThingMetadata>,
    pub keys: Vec<ThingMetadata>,
    pub monsters: Vec<ThingMetadata>,
}

#[derive(Debug, Deserialize, Copy, Clone)]
pub enum TriggerType {
    Any,
    Push,
    Switch,
    WalkOver,
    Gun,
}

#[derive(Debug, Copy, Clone, Deserialize)]
pub enum HeightRef {
    LowestFloor,
    NextFloor,
    HighestFloor,
    LowestCeiling,
    HighestCeiling,
    Floor,
    Ceiling,
}

#[derive(Debug, Copy, Clone, Deserialize)]
pub struct HeightDef {
    pub to: HeightRef,
    #[serde(default, rename = "off")]
    pub offset: WadCoord,
}

#[derive(Debug, Copy, Clone, Deserialize)]
pub struct HeightEffectDef {
    pub first: HeightDef,
    pub second: Option<HeightDef>,
}

#[derive(Debug, Copy, Clone, Deserialize)]
pub struct MoveEffectDef {
    pub floor: Option<HeightEffectDef>,
    pub ceiling: Option<HeightEffectDef>,
    #[serde(default)]
    pub repeat: bool,
    #[serde(default)]
    pub wait: f32,
    #[serde(default, deserialize_with = "deserialize_move_speed")]
    pub speed: f32,
}

#[derive(Debug, Deserialize, Copy, Clone)]
pub enum ExitEffectDef {
    Normal,
    Secret,
}

#[derive(Debug, Deserialize)]
pub struct LinedefMetadata {
    pub special_type: SpecialType,
    pub trigger: TriggerType,
    #[serde(default)]
    pub monsters: bool,
    #[serde(default)]
    pub only_once: bool,
    #[serde(rename = "move")]
    pub move_effect: Option<MoveEffectDef>,
    #[serde(rename = "exit")]
    pub exit_effect: Option<ExitEffectDef>,
}

#[derive(Deserialize)]
pub struct WadMetadata {
    pub sky: Vec<SkyMetadata>,
    pub animations: AnimationMetadata,
    pub things: ThingDirectoryMetadata,
    #[serde(default, deserialize_with = "deserialize_linedefs")]
    pub linedef: IndexMap<SpecialType, LinedefMetadata>,
}

impl fmt::Debug for WadMetadata {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WadMetadata")
            .field("sky", &self.sky)
            .field("linedef_count", &self.linedef.len())
            .finish()
    }
}

impl WadMetadata {
    pub fn from_file<P: AsRef<Path>>(path: &P) -> Result<WadMetadata> {
        let mut contents = String::new();
        File::open(path.as_ref())
            .and_then(|mut f| f.read_to_string(&mut contents))
            .context("Failed to read metadata file")?;
        WadMetadata::from_text(&contents)
    }

    pub fn from_text(text: &str) -> Result<WadMetadata> {
        toml::from_str(text).context("Failed to parse metadata file")
    }

    pub fn sky_for(&self, name: WadName) -> Option<&SkyMetadata> {
        self.sky
            .iter()
            .find(|sky| sky.level_pattern.is_match(name.as_ref()))
            .or_else(|| {
                if let Some(sky) = self.sky.first() {
                    warn!("No sky found for level {}, using {}.", name, sky.texture_name);
                    Some(sky)
                } else {
                    error!("No sky metadata provided.");
                    None
                }
            })
    }

    pub fn find_thing(&self, thing_type: ThingType) -> Option<&ThingMetadata> {
        let things = &self.things;
        things.decorations.iter().find(|t| t.thing_type == thing_type)
            .or_else(|| things.weapons.iter().find(|t| t.thing_type == thing_type))
            .or_else(|| things.powerups.iter().find(|t| t.thing_type == thing_type))
            .or_else(|| things.artifacts.iter().find(|t| t.thing_type == thing_type))
            .or_else(|| things.ammo.iter().find(|t| t.thing_type == thing_type))
            .or_else(|| things.keys.iter().find(|t| t.thing_type == thing_type))
            .or_else(|| things.monsters.iter().find(|t| t.thing_type == thing_type))
    }
}

fn deserialize_regex_from_str<'de, D>(deserializer: D) -> StdResult<Regex, D::Error>
where D: Deserializer<'de> {
    let s: String = Deserialize::deserialize(deserializer)?;
    Regex::new(&s).map_err(D::Error::custom)
}

fn deserialize_name_from_str<'de, D>(deserializer: D) -> StdResult<WadName, D::Error>
where D: Deserializer<'de> {
    let s: String = String::deserialize(deserializer)?;
    WadName::from_str(&s).map_err(D::Error::custom)
}

fn deserialize_move_speed<'de, D>(deserializer: D) -> StdResult<f32, D::Error>
where D: Deserializer<'de> {
    Ok(f32::deserialize(deserializer)? / 8.0 * 0.7)
}

fn deserialize_name_from_vec_vec_str<'de, D>(deserializer: D) -> StdResult<Vec<Vec<WadName>>, D::Error>
where D: Deserializer<'de> {
    let strings: Vec<Vec<String>> = Deserialize::deserialize(deserializer)?;
    strings
        .iter()
        .map(|inner| inner.iter().map(|s| WadName::from_str(s)).collect::<Result<Vec<_>>>())
        .collect::<Result<Vec<Vec<_>>>>()
        .map_err(D::Error::custom)
}

fn deserialize_linedefs<'de, D>(deserializer: D) -> StdResult<IndexMap<SpecialType, LinedefMetadata>, D::Error>
where D: Deserializer<'de> {
    let linedefs = <Vec<LinedefMetadata>>::deserialize(deserializer)?;
    Ok(linedefs.into_iter().map(|l| (l.special_type, l)).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sky_for_level_matching() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/meta/doom.toml");
        let meta = WadMetadata::from_file(&path).expect("parse doom.toml");
        let e1m1 = WadName::from_str("E1M1").unwrap();
        let sky = meta.sky_for(e1m1).expect("E1M1 should match E1M.");
        assert_eq!(sky.texture_name.as_ref().trim_end_matches('\0'), "SKY1");
        assert!((sky.tiled_band_size - 0.186).abs() < 1e-6);
        let map01 = WadName::from_str("MAP01").unwrap();
        let sky2 = meta.sky_for(map01).expect("MAP01 should match");
        assert_eq!(sky2.texture_name.as_ref().trim_end_matches('\0'), "SKY1");
        assert!((sky2.tiled_band_size - 1.0).abs() < 1e-6);
        let map15 = WadName::from_str("MAP15").unwrap();
        let sky3 = meta.sky_for(map15).expect("MAP15 should match");
        assert_eq!(sky3.texture_name.as_ref().trim_end_matches('\0'), "SKY2");
        assert!((sky3.tiled_band_size - 0.25).abs() < 1e-6);
    }
}
