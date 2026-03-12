use super::level::{Level, NeighbourHeights};
use super::light::{self, Contrast, LightInfo};
use super::meta::{
    ExitEffectDef, HeightDef, HeightEffectDef, HeightRef, MoveEffectDef, TriggerType, WadMetadata,
};
use super::tex::TextureDirectory;
use super::types::{
    ChildId, SectorId, SpecialType, ThingType, WadCoord, WadLinedef, WadName, WadNode, WadSector,
    WadSeg, WadThing,
};
use super::util::{
    from_wad_coords, from_wad_height, is_sky_flat, is_untextured, parse_child_id, to_wad_height,
};
use glam::Vec2;
use indexmap::IndexMap;
use log::{debug, error, info, warn};
use std::cmp::Ordering;
use std::f32::EPSILON;
use std::mem;

pub struct StaticQuad<'a> {
    pub object_id: ObjectId,
    pub vertices: (Vec2, Vec2),
    pub tex_start: (f32, f32),
    pub tex_end: (f32, f32),
    pub height_range: (f32, f32),
    pub light_info: &'a LightInfo,
    pub scroll: f32,
    pub tex_name: Option<WadName>,
    pub blocker: bool,
}

pub struct StaticPoly<'a> {
    pub object_id: ObjectId,
    pub vertices: &'a [Vec2],
    pub height: f32,
    pub light_info: &'a LightInfo,
    pub tex_name: WadName,
}

pub struct SkyQuad {
    pub object_id: ObjectId,
    pub vertices: (Vec2, Vec2),
    pub height_range: (f32, f32),
}

pub struct SkyPoly<'a> {
    pub object_id: ObjectId,
    pub vertices: &'a [Vec2],
    pub height: f32,
}

pub struct Decor<'a> {
    pub object_id: ObjectId,
    pub low: [f32; 3],
    pub high: [f32; 3],
    pub half_width: f32,
    pub light_info: &'a LightInfo,
    pub tex_name: WadName,
}

pub trait LevelVisitor: Sized {
    fn visit_wall_quad(&mut self, _quad: &StaticQuad) {}
    fn visit_floor_poly(&mut self, _poly: &StaticPoly) {}
    fn visit_ceil_poly(&mut self, _poly: &StaticPoly) {}
    fn visit_floor_sky_poly(&mut self, _poly: &SkyPoly) {}
    fn visit_ceil_sky_poly(&mut self, _poly: &SkyPoly) {}
    fn visit_sky_quad(&mut self, _quad: &SkyQuad) {}
    fn visit_marker(&mut self, _pos: [f32; 3], _yaw: f32, _marker: Marker) {}
    fn visit_decor(&mut self, _decor: &Decor) {}
    fn visit_bsp_root(&mut self, _line: &Line2) {}
    fn visit_bsp_node(&mut self, _line: &Line2, _branch: Branch) {}
    fn visit_bsp_leaf(&mut self, _branch: Branch) {}
    fn visit_bsp_leaf_end(&mut self) {}
    fn visit_bsp_node_end(&mut self) {}

    fn chain<'a, 'b, V: LevelVisitor>(&'a mut self, other: &'b mut V) -> VisitorChain<'a, 'b, Self, V> {
        VisitorChain { first: self, second: other }
    }
}

#[derive(Eq, PartialEq, Debug, Copy, Clone)]
pub enum Branch { Positive, Negative }

#[derive(Eq, PartialEq, Debug, Copy, Clone)]
pub enum Marker {
    StartPos { player: usize },
    TeleportStart,
    TeleportEnd,
}

#[derive(Eq, PartialEq, Debug, Copy, Clone, Default)]
pub struct ObjectId(pub u32);

// A 2D line defined by origin + displacement for BSP partitioning and geometry ops.
#[derive(Clone, Copy, Debug)]
pub struct Line2 {
    pub origin: Vec2,
    pub displace: Vec2,
}

impl Line2 {
    pub fn from_two_points(a: Vec2, b: Vec2) -> Self {
        Self { origin: a, displace: b - a }
    }

    pub fn signed_distance(&self, point: Vec2) -> f32 {
        let d = point - self.origin;
        let n = Vec2::new(-self.displace.y, self.displace.x);
        let len = n.length();
        if len < 1e-10 { return 0.0; }
        d.dot(n) / len
    }

    pub fn inverted_halfspaces(&self) -> Self {
        Self { origin: self.origin, displace: -self.displace }
    }

    pub fn intersect_point(&self, other: &Line2) -> Option<Vec2> {
        let d = self.displace;
        let e = other.displace;
        let cross = d.x * e.y - d.y * e.x;
        if cross.abs() < 1e-10 { return None; }
        let delta = other.origin - self.origin;
        let t = (delta.x * e.y - delta.y * e.x) / cross;
        Some(self.origin + d * t)
    }

    pub fn segment_intersect_offset(&self, other: &Line2) -> Option<f32> {
        let d = self.displace;
        let e = other.displace;
        let cross = d.x * e.y - d.y * e.x;
        if cross.abs() < 1e-10 { return None; }
        let delta = other.origin - self.origin;
        let t = (delta.x * e.y - delta.y * e.x) / cross;
        let u = (delta.x * d.y - delta.y * d.x) / cross;
        if (0.0..=1.0).contains(&t) && (0.0..=1.0).contains(&u) {
            Some(t)
        } else {
            None
        }
    }

    pub fn from_origin_and_displace(origin: Vec2, displace: Vec2) -> Self {
        Self { origin, displace }
    }
}

struct SectorInfo {
    floor_id: ObjectId,
    ceiling_id: ObjectId,
    floor_range: (WadCoord, WadCoord),
    ceiling_range: (WadCoord, WadCoord),
}

impl SectorInfo {
    fn max_height(&self) -> WadCoord {
        self.ceiling_range.1 - self.floor_range.0
    }
}

#[derive(Debug, Default)]
struct DynamicSectorInfo {
    floor_id: ObjectId,
    ceiling_id: ObjectId,
    neighbour_heights: Option<NeighbourHeights>,
    floor_range: Option<(WadCoord, WadCoord)>,
    ceiling_range: Option<(WadCoord, WadCoord)>,
}

impl DynamicSectorInfo {
    fn update(
        &mut self,
        next_dynamic_object_id: &mut ObjectId,
        level: &Level,
        sector_id: SectorId,
        trigger: &mut Trigger,
    ) {
        let sector = &level.sectors[sector_id as usize];
        let effect_def = match trigger.move_effect_def {
            Some(def) => def,
            None => return,
        };
        let heights = if let Some(h) = self.neighbour_heights {
            h
        } else if let Some(h) = level.neighbour_heights(sector) {
            self.neighbour_heights = Some(h);
            h
        } else {
            error!("Sector {} has no neighbours", sector_id);
            return;
        };
        let (first_floor, second_floor) = HeightEffectDef::option_to_heights(effect_def.floor, sector, &heights);
        let (first_ceiling, second_ceiling) = HeightEffectDef::option_to_heights(effect_def.ceiling, sector, &heights);
        let repeat = effect_def.repeat;

        merge_range(&mut self.floor_range, sector.floor_height, first_floor.into_iter().chain(second_floor));
        merge_range(&mut self.ceiling_range, sector.ceiling_height, first_ceiling.into_iter().chain(second_ceiling));

        if self.ceiling_range.is_some() && self.ceiling_id == ObjectId(0) {
            self.ceiling_id = *next_dynamic_object_id;
            next_dynamic_object_id.0 += 1;
        }
        if self.floor_range.is_some() && self.floor_id == ObjectId(0) {
            self.floor_id = *next_dynamic_object_id;
            next_dynamic_object_id.0 += 1;
        }

        if let Some(first_floor) = first_floor {
            trigger.move_effects.push(MoveEffect {
                object_id: self.floor_id,
                wait: effect_def.wait,
                speed: effect_def.speed,
                first_height_offset: from_wad_height(first_floor - sector.floor_height),
                second_height_offset: second_floor.map(|f| from_wad_height(f - sector.floor_height)),
                repeat,
            });
        }
        if let Some(first_ceiling) = first_ceiling {
            trigger.move_effects.push(MoveEffect {
                object_id: self.ceiling_id,
                wait: effect_def.wait,
                speed: effect_def.speed,
                first_height_offset: from_wad_height(first_ceiling - sector.ceiling_height),
                second_height_offset: second_ceiling.map(|c| from_wad_height(c - sector.ceiling_height)),
                repeat,
            });
        }
    }
}

fn merge_range<I: IntoIterator<Item = WadCoord>>(
    range: &mut Option<(WadCoord, WadCoord)>,
    current: WadCoord,
    with: I,
) {
    *range = with.into_iter().fold(*range, |range, coord| {
        Some(match range {
            Some((min, max)) => (min.min(coord), max.max(coord)),
            None => (coord, coord),
        })
    }).map(|(min, max)| (min.min(current), max.max(current)));
}

#[derive(Debug, Copy, Clone)]
pub struct MoveEffect {
    pub object_id: ObjectId,
    pub first_height_offset: f32,
    pub second_height_offset: Option<f32>,
    pub speed: f32,
    pub wait: f32,
    pub repeat: bool,
}

impl HeightDef {
    fn to_height(self, sector: &WadSector, heights: &NeighbourHeights) -> Option<WadCoord> {
        let base = match self.to {
            HeightRef::LowestFloor => heights.lowest_floor,
            HeightRef::NextFloor => heights.next_floor?,
            HeightRef::HighestFloor => heights.highest_floor,
            HeightRef::LowestCeiling => heights.lowest_ceiling,
            HeightRef::HighestCeiling => heights.highest_ceiling,
            HeightRef::Floor => sector.floor_height,
            HeightRef::Ceiling => sector.ceiling_height,
        };
        Some(base + self.offset)
    }
}

impl HeightEffectDef {
    fn option_to_heights(
        this: Option<Self>,
        sector: &WadSector,
        heights: &NeighbourHeights,
    ) -> (Option<WadCoord>, Option<WadCoord>) {
        this.map_or((None, None), |def| {
            (def.first.to_height(sector, heights), def.second.and_then(|d| d.to_height(sector, heights)))
        })
    }
}

#[derive(Debug, Clone)]
pub struct Trigger {
    pub trigger_type: TriggerType,
    pub line: Line2,
    pub special_type: SpecialType,
    pub only_once: bool,
    pub unimplemented: bool,
    pub move_effect_def: Option<MoveEffectDef>,
    pub exit_effect: Option<ExitEffectDef>,
    pub move_effects: Vec<MoveEffect>,
}

pub struct LevelAnalysis {
    dynamic_info: IndexMap<SectorId, DynamicSectorInfo>,
    triggers: Vec<Trigger>,
    num_objects: usize,
}

impl LevelAnalysis {
    pub fn new(level: &Level, meta: &WadMetadata) -> Self {
        let mut this = Self { dynamic_info: IndexMap::new(), triggers: Vec::new(), num_objects: 0 };
        this.compute_dynamic_sectors(level, meta);
        this
    }

    pub fn num_objects(&self) -> usize { self.num_objects }
    pub fn take_triggers(&mut self) -> Vec<Trigger> { mem::take(&mut self.triggers) }

    fn compute_dynamic_sectors(&mut self, level: &Level, meta: &WadMetadata) {
        info!("Computing dynamic sectors...");
        let mut num_dynamic_linedefs = 0;

        let mut sector_tags_and_ids: Vec<(u16, SectorId)> = level.sectors.iter().enumerate()
            .filter_map(|(i, s)| if s.tag > 0 { Some((s.tag, i as SectorId)) } else { None })
            .collect();
        sector_tags_and_ids.sort_unstable();

        let max_tag = if let Some(&(t, _)) = sector_tags_and_ids.last() { t } else { return; };
        let mut tag_to_first: Vec<Option<usize>> = vec![None; max_tag as usize + 1];
        let mut last_tag = !0u16;
        for (i, &(tag, _)) in sector_tags_and_ids.iter().enumerate() {
            if tag != last_tag { tag_to_first[tag as usize] = Some(i); last_tag = tag; }
        }

        let mut next_id = ObjectId(1);
        for (i_linedef, linedef) in level.linedefs.iter().enumerate() {
            let mut trigger = match self.linedef_to_trigger(level, meta, linedef) {
                Some(t) => t,
                None => continue,
            };
            num_dynamic_linedefs += 1;

            let tag = linedef.sector_tag;
            if tag == 0 {
                if let Some(sidedef) = level.left_sidedef(linedef) {
                    let sid = sidedef.sector;
                    debug!("Sector {} zero-tag dynamic from linedef {}", sid, i_linedef);
                    self.dynamic_info.entry(sid).or_default().update(&mut next_id, level, sid, &mut trigger);
                }
                self.triggers.push(trigger);
                continue;
            }

            if let Some(&Some(first)) = tag_to_first.get(tag as usize) {
                for &(ct, csid) in &sector_tags_and_ids[first..] {
                    if ct != tag { break; }
                    debug!("Sector {} tag {} dynamic from linedef {}", csid, tag, i_linedef);
                    self.dynamic_info.entry(csid).or_default().update(&mut next_id, level, csid, &mut trigger);
                }
            } else {
                warn!("No sector with tag {} for linedef {}", tag, i_linedef);
            }
            self.triggers.push(trigger);
        }
        self.num_objects = next_id.0 as usize;
        info!("Dynamic sectors: {} objects, {} linedefs", self.num_objects, num_dynamic_linedefs);
    }

    fn linedef_to_trigger(&self, level: &Level, meta: &WadMetadata, linedef: &WadLinedef) -> Option<Trigger> {
        let special_type = linedef.special_type;
        if special_type == 0 { return None; }
        let line = match (level.vertex(linedef.start_vertex), level.vertex(linedef.end_vertex)) {
            (Some(s), Some(e)) => Line2::from_two_points(s, e),
            _ => { error!("Missing vertices for linedef"); return None; }
        };
        Some(if let Some(meta) = meta.linedef.get(&special_type) {
            Trigger {
                trigger_type: meta.trigger, only_once: meta.only_once,
                move_effect_def: meta.move_effect, exit_effect: meta.exit_effect,
                unimplemented: false, special_type, line, move_effects: Vec::new(),
            }
        } else {
            error!("Unknown linedef special type: {}", special_type);
            Trigger {
                trigger_type: TriggerType::Any, only_once: false,
                move_effect_def: None, exit_effect: None,
                unimplemented: true, special_type, line, move_effects: Vec::new(),
            }
        })
    }
}

pub struct LevelWalker<'a, V: LevelVisitor + 'a> {
    level: &'a Level,
    tex: &'a TextureDirectory,
    meta: &'a WadMetadata,
    visitor: &'a mut V,
    height_range: (WadCoord, WadCoord),
    bsp_lines: Vec<Line2>,
    dynamic_info: &'a IndexMap<SectorId, DynamicSectorInfo>,
    subsector_points: Vec<Vec2>,
    subsector_seg_lines: Vec<Line2>,
    light_cache: IndexMap<usize, LightInfo>,
}

impl<'a, V: LevelVisitor> LevelWalker<'a, V> {
    pub fn new(
        level: &'a Level,
        analysis: &'a LevelAnalysis,
        tex: &'a TextureDirectory,
        meta: &'a WadMetadata,
        visitor: &'a mut V,
    ) -> Self {
        Self {
            level, tex, meta, visitor,
            height_range: min_max_height(level),
            bsp_lines: Vec::with_capacity(32),
            subsector_points: Vec::with_capacity(32),
            subsector_seg_lines: Vec::with_capacity(32),
            light_cache: IndexMap::new(),
            dynamic_info: &analysis.dynamic_info,
        }
    }

    pub fn walk(&mut self) {
        let root = match self.level.nodes.last() {
            Some(n) => n,
            None => { warn!("Level contains no nodes"); return; }
        };
        let partition = partition_line(root);
        self.visitor.visit_bsp_root(&partition);
        self.children(root, partition);
        self.visitor.visit_bsp_node_end();
        self.things();
    }

    fn floor_id(&self, sector: &WadSector) -> ObjectId {
        self.dynamic_info.get(&self.level.sector_id(sector)).map_or(ObjectId(0), |d| d.floor_id)
    }

    fn ceiling_id(&self, sector: &WadSector) -> ObjectId {
        self.dynamic_info.get(&self.level.sector_id(sector)).map_or(ObjectId(0), |d| d.ceiling_id)
    }

    fn sector_info(&self, sector: &WadSector) -> SectorInfo {
        let fr = (sector.floor_height, sector.floor_height);
        let cr = (sector.ceiling_height, sector.ceiling_height);
        self.dynamic_info.get(&self.level.sector_id(sector)).map_or_else(
            || SectorInfo { floor_id: ObjectId(0), ceiling_id: ObjectId(0), floor_range: fr, ceiling_range: cr },
            |d| SectorInfo {
                floor_id: d.floor_id, ceiling_id: d.ceiling_id,
                floor_range: d.floor_range.unwrap_or(fr), ceiling_range: d.ceiling_range.unwrap_or(cr),
            },
        )
    }

    fn node(&mut self, id: ChildId, branch: Branch) {
        let (id, is_leaf) = parse_child_id(id);
        if is_leaf {
            self.visitor.visit_bsp_leaf(branch);
            self.subsector(id);
            self.visitor.visit_bsp_leaf_end();
            return;
        }
        let node = match self.level.nodes.get(id) {
            Some(n) => n,
            None => { warn!("Missing node {}", id); return; }
        };
        let partition = partition_line(node);
        self.visitor.visit_bsp_node(&partition, branch);
        self.children(node, partition);
        self.visitor.visit_bsp_node_end();
    }

    fn children(&mut self, node: &WadNode, partition: Line2) {
        self.bsp_lines.push(partition.inverted_halfspaces());
        self.node(node.left, Branch::Positive);
        self.bsp_lines.pop();
        self.bsp_lines.push(partition);
        self.node(node.right, Branch::Negative);
        self.bsp_lines.pop();
    }

    fn subsector(&mut self, id: usize) {
        let subsector = match self.level.ssector(id) {
            Some(s) => s, None => { warn!("Missing subsector {}", id); return; }
        };
        let segs = match self.level.ssector_segs(subsector) {
            Some(s) => s, None => { warn!("Missing segs for subsector {}", id); return; }
        };
        if segs.is_empty() { warn!("Zero segs for subsector {}", id); return; }
        let sector = match self.level.seg_sector(&segs[0]) {
            Some(s) => s, None => { warn!("Missing sector for subsector {}", id); return; }
        };
        let sector_info = self.sector_info(sector);

        self.subsector_seg_lines.clear();
        self.subsector_seg_lines.reserve(segs.len());
        self.subsector_points.clear();
        self.subsector_points.reserve(segs.len() * 3);

        for seg in segs {
            let (v1, v2) = match self.level.seg_vertices(seg) {
                Some(v) => v, None => { warn!("Missing seg vertices subsector {}", id); return; }
            };
            self.subsector_points.push(v1);
            self.subsector_points.push(v2);
            self.subsector_seg_lines.push(Line2::from_two_points(v1, v2));
            self.seg(sector, &sector_info, seg, (v1, v2));
        }

        let seg_point_count = self.subsector_points.len();
        let (seg_bb_min, seg_bb_max) = if seg_point_count >= 2 {
            let pts = &self.subsector_points[..seg_point_count];
            let mut mn = pts[0];
            let mut mx = pts[0];
            for &p in &pts[1..] { mn = mn.min(p); mx = mx.max(p); }
            (mn, mx)
        } else { (Vec2::ZERO, Vec2::ZERO) };

        for i in 0..(self.bsp_lines.len().saturating_sub(1)) {
            for j in (i + 1)..self.bsp_lines.len() {
                let point = match self.bsp_lines[i].intersect_point(&self.bsp_lines[j]) {
                    Some(p) => p, None => continue,
                };
                let inside_bsp = self.bsp_lines.iter().all(|l| l.signed_distance(point) >= -BSP_TOLERANCE);
                let inside_seg = self.subsector_seg_lines.iter().all(|l| l.signed_distance(point) <= SEG_TOLERANCE);
                if inside_bsp && inside_seg {
                    self.subsector_points.push(point);
                }
            }
        }

        if self.subsector_points.len() < 3 {
            warn!("Degenerate polygon {} ({} verts)", id, self.subsector_points.len());
        }
        points_to_polygon(&mut self.subsector_points);

        if seg_point_count >= 2 {
            let margin = Vec2::splat(10.0);
            let lo = seg_bb_min - margin;
            let hi = seg_bb_max + margin;
            let mut poly = vec![
                Vec2::new(lo.x, lo.y),
                Vec2::new(hi.x, lo.y),
                Vec2::new(hi.x, hi.y),
                Vec2::new(lo.x, hi.y),
            ];
            for bl in &self.bsp_lines {
                poly = clip_polygon_to_halfplane(&poly, bl);
                if poly.len() < 3 { break; }
            }
            if poly.len() >= 3 {
                self.subsector_points = poly;
                self.flat_poly(sector, &sector_info);
            }
        }
    }

    fn seg(&mut self, sector: &WadSector, info: &SectorInfo, seg: &WadSeg, vertices: (Vec2, Vec2)) {
        let line = match self.level.seg_linedef(seg) {
            Some(l) => l, None => return,
        };
        let sidedef = match self.level.seg_sidedef(seg) {
            Some(s) => s, None => return,
        };
        let (min, max) = (self.height_range.0, self.height_range.1);
        let (floor, ceiling) = (sector.floor_height, sector.ceiling_height);
        let unpeg_lower = line.lower_unpegged();
        let back_sector = match self.level.seg_back_sector(seg) {
            None => {
                self.wall_quad(InternalWallQuad {
                    sector, seg, vertices,
                    object_id: if unpeg_lower { info.floor_id } else { info.ceiling_id },
                    height_range: if unpeg_lower { (floor, floor + info.max_height()) } else { (ceiling - info.max_height(), ceiling) },
                    texture_name: sidedef.middle_texture,
                    peg: if unpeg_lower { Peg::Bottom } else { Peg::Top },
                    blocker: true,
                });
                if is_sky_flat(sector.ceiling_texture) { self.sky_quad(info.ceiling_id, vertices, (ceiling, max)); }
                if is_sky_flat(sector.floor_texture) { self.sky_quad(info.floor_id, vertices, (min, floor)); }
                return;
            }
            Some(s) => s,
        };
        let (back_floor, back_ceiling) = (back_sector.floor_height, back_sector.ceiling_height);
        let back_info = self.sector_info(back_sector);

        if is_sky_flat(sector.ceiling_texture) && !is_sky_flat(back_sector.ceiling_texture) {
            self.sky_quad(info.ceiling_id, vertices, (ceiling, max));
        }
        if is_sky_flat(sector.floor_texture) && !is_sky_flat(back_sector.floor_texture) {
            self.sky_quad(info.floor_id, vertices, (min, floor));
        }

        let unpeg_upper = line.upper_unpegged();
        let floor = if back_info.floor_range.1 > info.floor_range.0 {
            self.wall_quad(InternalWallQuad {
                sector, seg, vertices,
                object_id: back_info.floor_id,
                height_range: (back_floor - back_info.floor_range.1 + info.floor_range.0, back_floor),
                texture_name: sidedef.lower_texture,
                peg: if unpeg_lower { Peg::BottomLower } else { Peg::Top },
                blocker: true,
            });
            back_floor
        } else { floor };

        let ceil = if back_ceiling < ceiling {
            if !is_sky_flat(back_sector.ceiling_texture) {
                self.wall_quad(InternalWallQuad {
                    sector, seg, vertices,
                    object_id: back_info.ceiling_id,
                    height_range: (back_ceiling, ceiling),
                    texture_name: sidedef.upper_texture,
                    peg: if unpeg_upper { Peg::Top } else { Peg::Bottom },
                    blocker: true,
                });
            }
            back_ceiling
        } else { ceiling };

        self.wall_quad(InternalWallQuad {
            sector, seg, vertices,
            object_id: if unpeg_lower { info.floor_id } else { info.ceiling_id },
            height_range: (floor, ceil),
            texture_name: sidedef.middle_texture,
            peg: if unpeg_lower {
                if is_untextured(sidedef.upper_texture) { Peg::TopFloat } else { Peg::Bottom }
            } else {
                if is_untextured(sidedef.lower_texture) { Peg::BottomFloat } else { Peg::Top }
            },
            blocker: line.impassable(),
        });
    }

    fn wall_quad(&mut self, quad: InternalWallQuad) {
        let InternalWallQuad { object_id, sector, seg, vertices: (v1, v2), height_range: (low, high), texture_name, peg, blocker } = quad;
        if low >= high { return; }
        let size = if is_untextured(texture_name) {
            None
        } else if let Some(image) = self.tex.texture(texture_name) {
            Some(Vec2::new(image.width() as f32, image.height() as f32))
        } else {
            warn!("wall_quad: No such wall texture '{}'", texture_name);
            return;
        };
        let line = match self.level.seg_linedef(seg) { Some(l) => l, None => return };
        let sidedef = match self.level.seg_sidedef(seg) { Some(s) => s, None => return };
        let dir = (v2 - v1).normalize_or_zero();
        let bias = dir * POLY_BIAS;
        let (v1, v2) = (v1 - bias, v2 + bias);
        let (low, high) = match (size, peg) {
            (Some(sz), Peg::TopFloat) => (from_wad_height(low + sidedef.y_offset), from_wad_height(low + sz.y as i16 + sidedef.y_offset)),
            (Some(sz), Peg::BottomFloat) => (from_wad_height(high + sidedef.y_offset - sz.y as i16), from_wad_height(high + sidedef.y_offset)),
            _ => (from_wad_height(low), from_wad_height(high)),
        };

        let li = light_info(&mut self.light_cache, self.level, sector).clone();
        let li = if li.effect.is_none() {
            if (v1.x - v2.x).abs() < EPSILON {
                light::with_contrast(&li, Contrast::Brighten)
            } else if (v1.y - v2.y).abs() < EPSILON {
                light::with_contrast(&li, Contrast::Darken)
            } else { li }
        } else { li };

        let height = to_wad_height(high - low);
        let s1 = f32::from(seg.offset) + f32::from(sidedef.x_offset);
        let s2 = s1 + to_wad_height((v2 - v1).length());
        let (t1, t2) = match (size, peg) {
            (Some(_), Peg::Top) | (None, _) => (height, 0.0),
            (Some(sz), Peg::Bottom) => (sz.y, sz.y - height),
            (Some(sz), Peg::BottomLower) => {
                let sh = f32::from(sector.ceiling_height - sector.floor_height);
                (sz.y + sh, sz.y - height + sh)
            }
            (Some(sz), Peg::TopFloat) | (Some(sz), Peg::BottomFloat) => (sz.y, 0.0),
        };
        let (t1, t2) = (t1 + f32::from(sidedef.y_offset), t2 + f32::from(sidedef.y_offset));
        let scroll = if line.special_type == 0x30 { 35.0 } else { 0.0 };
        let (low, high) = (low - POLY_BIAS, high + POLY_BIAS);

        self.visitor.visit_wall_quad(&StaticQuad {
            vertices: (v1, v2), tex_start: (s1, t1), tex_end: (s2, t2),
            height_range: (low, high), light_info: &li,
            tex_name: size.map(|_| texture_name), blocker, scroll, object_id,
        });
    }

    fn flat_poly(&mut self, sector: &WadSector, info: &SectorInfo) {
        let li = light_info(&mut self.light_cache, self.level, sector).clone();
        let (floor_tex, ceil_tex) = (sector.floor_texture, sector.ceiling_texture);
        let (floor_sky, ceil_sky) = (is_sky_flat(floor_tex), is_sky_flat(ceil_tex));
        let floor_y = from_wad_height(if floor_sky { self.height_range.0 } else { sector.floor_height });
        let ceil_y = from_wad_height(if ceil_sky { self.height_range.1 } else { sector.ceiling_height });

        if floor_sky {
            self.visitor.visit_floor_sky_poly(&SkyPoly { object_id: info.floor_id, vertices: &self.subsector_points, height: floor_y });
        } else {
            self.visitor.visit_floor_poly(&StaticPoly { object_id: info.floor_id, vertices: &self.subsector_points, height: floor_y, light_info: &li, tex_name: floor_tex });
        }
        if ceil_sky {
            self.visitor.visit_ceil_sky_poly(&SkyPoly { object_id: info.ceiling_id, vertices: &self.subsector_points, height: ceil_y });
        } else {
            self.visitor.visit_ceil_poly(&StaticPoly { object_id: info.ceiling_id, vertices: &self.subsector_points, height: ceil_y, light_info: &li, tex_name: ceil_tex });
        }
    }

    fn sky_quad(&mut self, object_id: ObjectId, (v1, v2): (Vec2, Vec2), (low, high): (WadCoord, WadCoord)) {
        if low >= high { return; }
        let edge = (v2 - v1).normalize_or_zero();
        let bias = edge * POLY_BIAS * 16.0;
        let normal = Vec2::new(-edge.y, edge.x);
        let nbias = normal * POLY_BIAS * 16.0;
        let (v1, v2) = (v1 + nbias - bias, v2 + nbias + bias);
        self.visitor.visit_sky_quad(&SkyQuad {
            object_id, vertices: (v1, v2),
            height_range: (from_wad_height(low), from_wad_height(high)),
        });
    }

    fn things(&mut self) {
        for thing in &self.level.things {
            let pos = from_wad_coords(thing.x, thing.y);
            let yaw_deg = f32::round(f32::from(thing.angle) / 45.0) * 45.0;
            if let Some(marker) = Marker::from(thing.thing_type) {
                // Markers (player starts, teleports) don't need a valid sector —
                // fall back to floor_height=0 if BSP lookup fails.
                let floor_y = self.sector_at(pos)
                    .map(|s| from_wad_height(s.floor_height))
                    .unwrap_or(0.0);
                let p = [pos.x, floor_y, pos.y];
                self.visitor.visit_marker(p, yaw_deg.to_radians(), marker);
            } else if let Some(sector) = self.sector_at(pos) {
                self.decor(thing, pos, sector);
            }
        }
    }

    fn sector_at(&self, pos: Vec2) -> Option<&'a WadSector> {
        let mut child_id = (self.level.nodes.len() - 1) as ChildId;
        loop {
            let (id, is_leaf) = parse_child_id(child_id);
            if is_leaf {
                let segs = self.level.ssector(id)
                    .and_then(|ss| self.level.ssector_segs(ss))
                    .and_then(|s| if s.is_empty() { None } else { Some(s) });
                let segs = segs?;
                let sector = self.level.seg_sector(&segs[0])?;
                return if segs.iter()
                    .filter_map(|seg| self.level.seg_vertices(seg))
                    .map(|(v1, v2)| Line2::from_two_points(v1, v2))
                    .all(|line| line.signed_distance(pos) <= SEG_TOLERANCE)
                { Some(sector) } else { None };
            } else {
                let node = self.level.nodes.get(id)?;
                let partition = partition_line(node);
                child_id = if partition.signed_distance(pos) > 0.0 { node.left } else { node.right };
            }
        }
    }

    fn decor(&mut self, thing: &WadThing, pos: Vec2, sector: &WadSector) {
        let meta = match self.meta.find_thing(thing.thing_type) {
            Some(m) => m, None => { warn!("No metadata for thing type {}", thing.thing_type); return; }
        };
        let (name, size) = {
            let mut sprite0 = meta.sprite;
            let _ = sprite0.push(meta.sequence.as_bytes()[0]);
            let mut sprite1 = sprite0;
            let s0 = sprite0.push(b'0').ok().map(|_| sprite0);
            let s1 = sprite1.push(b'1').ok().map(|_| sprite1);
            match (s0, s1) {
                (Some(s0), Some(s1)) => {
                    if let Some(img) = self.tex.texture(s0) { (s0, img.size()) }
                    else if let Some(img) = self.tex.texture(s1) { (s1, img.size()) }
                    else { warn!("No sprite {} for thing {}", meta.sprite, thing.thing_type); return; }
                }
                _ => { warn!("Bad sprite name for thing {}", thing.thing_type); return; }
            }
        };
        let sz = Vec2::new(from_wad_height(size[0] as i16), from_wad_height(size[1] as i16));
        let (object_id, low, high) = if meta.hanging {
            (self.ceiling_id(sector),
             [pos.x, from_wad_height(sector.ceiling_height) - sz.y, pos.y],
             [pos.x, from_wad_height(sector.ceiling_height), pos.y])
        } else {
            (self.floor_id(sector),
             [pos.x, from_wad_height(sector.floor_height), pos.y],
             [pos.x, from_wad_height(sector.floor_height) + sz.y, pos.y])
        };
        let li = light_info(&mut self.light_cache, self.level, sector).clone();
        self.visitor.visit_decor(&Decor { object_id, low, high, half_width: sz.x * 0.5, light_info: &li, tex_name: name });
    }
}

fn light_info<'a>(
    cache: &'a mut IndexMap<usize, LightInfo>,
    level: &Level,
    sector: &WadSector,
) -> &'a LightInfo {
    let key = level.sector_id(sector) as usize;
    if !cache.contains_key(&key) {
        let info = light::new_light(level, sector);
        cache.insert(key, info);
    }
    &cache[&key]
}

fn partition_line(node: &WadNode) -> Line2 {
    Line2::from_two_points(
        from_wad_coords(node.line_x, node.line_y),
        from_wad_coords(node.line_x + node.step_x, node.line_y + node.step_y),
    )
}

const BSP_TOLERANCE: f32 = 1e-3;
const SEG_TOLERANCE: f32 = 0.1;
const POLY_BIAS: f32 = 0.64 * 3e-4;

fn clip_polygon_to_halfplane(polygon: &[Vec2], line: &Line2) -> Vec<Vec2> {
    if polygon.is_empty() { return Vec::new(); }
    let mut out = Vec::with_capacity(polygon.len() + 1);
    let n = polygon.len();
    for i in 0..n {
        let cur = polygon[i];
        let nxt = polygon[(i + 1) % n];
        let d_cur = line.signed_distance(cur);
        let d_nxt = line.signed_distance(nxt);
        if d_cur >= -BSP_TOLERANCE {
            out.push(cur);
        }
        if (d_cur >= -BSP_TOLERANCE) != (d_nxt >= -BSP_TOLERANCE) {
            let edge = Line2::from_two_points(cur, nxt);
            if let Some(p) = line.intersect_point(&edge) {
                out.push(p);
            }
        }
    }
    out
}

#[derive(Copy, Clone)]
enum Peg { Top, Bottom, BottomLower, TopFloat, BottomFloat }

fn min_max_height(level: &Level) -> (WadCoord, WadCoord) {
    let (min, max) = level.sectors.iter()
        .map(|s| (s.floor_height, s.ceiling_height))
        .fold((32_767i16, -32_768i16), |(mn, mx), (f, c)| (mn.min(f), mx.max(c)));
    (min - 512, max + 512)
}

fn polygon_center(points: &[Vec2]) -> Vec2 {
    let sum: Vec2 = points.iter().copied().sum();
    sum / points.len() as f32
}

fn points_to_polygon(points: &mut Vec<Vec2>) {
    let center = polygon_center(points);
    points.sort_unstable_by(|a, b| {
        let ac = *a - center;
        let bc = *b - center;
        if ac.x >= 0.0 && bc.x < 0.0 { return Ordering::Less; }
        if ac.x < 0.0 && bc.x >= 0.0 { return Ordering::Greater; }
        if ac.x == 0.0 && bc.x == 0.0 {
            return if ac.y >= 0.0 || bc.y >= 0.0 {
                if a.y > b.y { Ordering::Less } else { Ordering::Greater }
            } else {
                if b.y > a.y { Ordering::Less } else { Ordering::Greater }
            };
        }
        if ac.perp_dot(bc) < 0.0 { Ordering::Less } else { Ordering::Greater }
    });

    let mut simplified = Vec::with_capacity(points.len());
    if points.is_empty() { return; }
    simplified.push(points[0]);
    if points.len() < 3 { points.clear(); return; }
    let mut current = points[1];
    let mut area = 0.0f32;
    for i in 2..points.len() {
        let next = points[i];
        let prev = simplified[simplified.len() - 1];
        let new_area = (next - current).perp_dot(current - prev) * 0.5;
        if new_area >= 0.0 {
            if area + new_area > 1.024e-5 { area = 0.0; simplified.push(current); }
            else { area += new_area; }
        }
        current = next;
    }
    simplified.push(points[points.len() - 1]);
    if simplified.len() < 3 { points.clear(); return; }
    while (simplified[0] - simplified[simplified.len() - 1]).length() < 0.0032 {
        simplified.pop();
    }
    let center = polygon_center(&simplified);
    for p in &mut simplified {
        *p += (*p - center).normalize_or_zero() * POLY_BIAS;
    }
    *points = simplified;
}

pub struct VisitorChain<'a, 'b, A: LevelVisitor + 'a, B: LevelVisitor + 'b> {
    first: &'a mut A,
    second: &'b mut B,
}

impl<'a, 'b, A: LevelVisitor, B: LevelVisitor> LevelVisitor for VisitorChain<'a, 'b, A, B> {
    fn visit_wall_quad(&mut self, q: &StaticQuad) { self.first.visit_wall_quad(q); self.second.visit_wall_quad(q); }
    fn visit_floor_poly(&mut self, p: &StaticPoly) { self.first.visit_floor_poly(p); self.second.visit_floor_poly(p); }
    fn visit_ceil_poly(&mut self, p: &StaticPoly) { self.first.visit_ceil_poly(p); self.second.visit_ceil_poly(p); }
    fn visit_floor_sky_poly(&mut self, p: &SkyPoly) { self.first.visit_floor_sky_poly(p); self.second.visit_floor_sky_poly(p); }
    fn visit_ceil_sky_poly(&mut self, p: &SkyPoly) { self.first.visit_ceil_sky_poly(p); self.second.visit_ceil_sky_poly(p); }
    fn visit_sky_quad(&mut self, q: &SkyQuad) { self.first.visit_sky_quad(q); self.second.visit_sky_quad(q); }
    fn visit_marker(&mut self, p: [f32; 3], y: f32, m: Marker) { self.first.visit_marker(p, y, m); self.second.visit_marker(p, y, m); }
    fn visit_decor(&mut self, d: &Decor) { self.first.visit_decor(d); self.second.visit_decor(d); }
    fn visit_bsp_root(&mut self, l: &Line2) { self.first.visit_bsp_root(l); self.second.visit_bsp_root(l); }
    fn visit_bsp_node(&mut self, l: &Line2, b: Branch) { self.first.visit_bsp_node(l, b); self.second.visit_bsp_node(l, b); }
    fn visit_bsp_leaf(&mut self, b: Branch) { self.first.visit_bsp_leaf(b); self.second.visit_bsp_leaf(b); }
    fn visit_bsp_leaf_end(&mut self) { self.first.visit_bsp_leaf_end(); self.second.visit_bsp_leaf_end(); }
    fn visit_bsp_node_end(&mut self) { self.first.visit_bsp_node_end(); self.second.visit_bsp_node_end(); }
}

#[derive(Copy, Clone)]
struct InternalWallQuad<'a> {
    object_id: ObjectId,
    sector: &'a WadSector,
    seg: &'a WadSeg,
    vertices: (Vec2, Vec2),
    height_range: (WadCoord, WadCoord),
    texture_name: WadName,
    peg: Peg,
    blocker: bool,
}

const THING_TYPE_PLAYER1_START: ThingType = 1;
const THING_TYPE_PLAYER2_START: ThingType = 2;
const THING_TYPE_PLAYER3_START: ThingType = 3;
const THING_TYPE_PLAYER4_START: ThingType = 4;
const THING_TYPE_TELEPORT_START: ThingType = 11;
const THING_TYPE_TELEPORT_END: ThingType = 14;

impl Marker {
    fn from(thing_type: ThingType) -> Option<Self> {
        match thing_type {
            THING_TYPE_PLAYER1_START => Some(Marker::StartPos { player: 0 }),
            THING_TYPE_PLAYER2_START => Some(Marker::StartPos { player: 1 }),
            THING_TYPE_PLAYER3_START => Some(Marker::StartPos { player: 2 }),
            THING_TYPE_PLAYER4_START => Some(Marker::StartPos { player: 3 }),
            THING_TYPE_TELEPORT_START => Some(Marker::TeleportStart),
            THING_TYPE_TELEPORT_END => Some(Marker::TeleportEnd),
            _ => None,
        }
    }
}
