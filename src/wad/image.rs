use super::types::WadTextureHeader;
use anyhow::{ensure, Context, Result};
use byteorder::{LittleEndian, ReadBytesExt};
use log::{debug, warn};

pub const MAX_IMAGE_SIZE: usize = 4096;

pub struct Image {
    width: usize,
    height: usize,
    x_offset: isize,
    y_offset: isize,
    pixels: Vec<u16>,
}

impl Image {
    pub fn new(width: usize, height: usize) -> Result<Self> {
        ensure!(
            width <= MAX_IMAGE_SIZE && height <= MAX_IMAGE_SIZE,
            "Image too large {}x{}",
            width,
            height,
        );
        Ok(Self {
            width,
            height,
            x_offset: 0,
            y_offset: 0,
            pixels: vec![0xff00; width * height],
        })
    }

    pub fn new_from_header(header: &WadTextureHeader) -> Result<Self> {
        Self::new(header.width as usize, header.height as usize)
    }

    pub fn from_buffer(buffer: &[u8]) -> Result<Self> {
        let mut reader = buffer;
        let width = reader.read_u16::<LittleEndian>().context("Image missing width")? as usize;
        let height = reader.read_u16::<LittleEndian>().context("Image missing height")? as usize;
        ensure!(width <= MAX_IMAGE_SIZE && height <= MAX_IMAGE_SIZE, "Image too large {}x{}", width, height);

        let x_offset = reader.read_i16::<LittleEndian>().context("Image missing x offset")? as isize;
        let y_offset = reader.read_i16::<LittleEndian>().context("Image missing y offset")? as isize;

        let mut pixels = vec![!0u16; width * height];

        for i_column in 0..width {
            let offset = reader.read_u32::<LittleEndian>()
                .with_context(|| format!("Unfinished column {} header", i_column))? as isize;
            ensure!(
                (offset as usize) < buffer.len(),
                "Invalid image column offset in {}, offset={}, size={}",
                i_column, offset, buffer.len()
            );
            let mut source = buffer[offset as usize..].iter();
            loop {
                let row_start = *source.next().ok_or_else(|| anyhow::anyhow!("Unfinished column {}", i_column))? as usize;
                if row_start == 255 {
                    break;
                }
                let run_length = *source.next().ok_or_else(|| anyhow::anyhow!("Missing run length col {}", i_column))? as usize;
                ensure!(row_start + run_length <= height, "Image run too big col {}", i_column);
                source.next(); // padding byte
                for row in row_start..(row_start + run_length) {
                    let pixel = *source.next().ok_or_else(|| anyhow::anyhow!("Missing pixel col {}", i_column))?;
                    pixels[row * width + i_column] = u16::from(pixel);
                }
                source.next(); // trailing padding
            }
        }

        Ok(Self { width, height, x_offset, y_offset, pixels })
    }

    pub fn blit(&mut self, source: &Self, offset: [isize; 2], ignore_transparency: bool) {
        if offset[0] >= self.width as isize || offset[1] >= self.height as isize {
            warn!("Fully out of bounds blit {:?} in {}x{}", offset, self.width, self.height);
            return;
        }

        let y_start = if offset[1] < 0 { (-offset[1]) as usize } else { 0 };
        let x_start = if offset[0] < 0 { (-offset[0]) as usize } else { 0 };
        let y_end = if self.height as isize > source.height as isize + offset[1] {
            source.height
        } else {
            (self.height as isize - offset[1]) as usize
        };
        let x_end = if self.width as isize > source.width as isize + offset[0] {
            source.width
        } else {
            (self.width as isize - offset[0]) as usize
        };

        debug!("Blit {}x{} <- {}x{} +{}x{}", self.width, self.height, source.width, source.height, offset[0], offset[1]);

        let src_pitch = source.width;
        let dest_pitch = self.width;
        let copy_width = x_end - x_start;
        let copy_height = y_end - y_start;

        for dy in 0..copy_height {
            let sy = y_start + dy;
            let dest_y = (sy as isize + offset[1]) as usize;
            let src_row = &source.pixels[sy * src_pitch + x_start..sy * src_pitch + x_start + copy_width];
            let dest_x = (x_start as isize + offset[0]) as usize;
            let dest_row = &mut self.pixels[dest_y * dest_pitch + dest_x..dest_y * dest_pitch + dest_x + copy_width];
            if ignore_transparency {
                dest_row.copy_from_slice(src_row);
            } else {
                for (d, &s) in dest_row.iter_mut().zip(src_row.iter()) {
                    let blend = 0u16.wrapping_sub(s >> 15);
                    *d = (s & !blend) | (*d & blend);
                }
            }
        }
    }

    pub fn x_offset(&self) -> isize { self.x_offset }
    pub fn y_offset(&self) -> isize { self.y_offset }
    pub fn width(&self) -> usize { self.width }
    pub fn height(&self) -> usize { self.height }
    pub fn size(&self) -> [usize; 2] { [self.width, self.height] }
    pub fn num_pixels(&self) -> usize { self.pixels.len() }
    pub fn pixels(&self) -> &[u16] { &self.pixels }
    pub fn into_pixels(self) -> Vec<u16> { self.pixels }
}
