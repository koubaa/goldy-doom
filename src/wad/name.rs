use anyhow::{bail, ensure, Result};
use serde::de::{Deserialize, Deserializer, Error as SerdeDeError};
use std::borrow::Borrow;
use std::fmt;
use std::result::Result as StdResult;
use std::str::{self, FromStr};

#[derive(Clone, Copy, PartialEq, PartialOrd, Ord, Eq, Hash)]
pub struct WadName([u8; 8]);

impl WadName {
    pub fn push(&mut self, new_byte: u8) -> Result<()> {
        let new_byte = match new_byte.to_ascii_uppercase() {
            b @ b'A'..=b'Z'
            | b @ b'0'..=b'9'
            | b @ b'_'
            | b @ b'%'
            | b @ b'-'
            | b @ b'['
            | b @ b']'
            | b @ b'\\' => b,
            b => bail!("Invalid character `{}` in wad name", char::from(b)),
        };

        for byte in &mut self.0 {
            if *byte == 0 {
                *byte = new_byte;
                return Ok(());
            }
        }

        bail!("Wad name too long")
    }

    pub fn from_bytes(value: &[u8]) -> Result<WadName> {
        let mut name = [0u8; 8];
        let mut nulled = false;
        for (dest, &src) in name.iter_mut().zip(value.iter()) {
            ensure!(src.is_ascii(), "Non-ASCII byte in wad name");
            let new_byte = match src.to_ascii_uppercase() {
                b @ b'A'..=b'Z'
                | b @ b'0'..=b'9'
                | b @ b'_'
                | b @ b'-'
                | b @ b'['
                | b @ b']'
                | b @ b'%'
                | b @ b'\\' => b,
                b'\0' => {
                    nulled = true;
                    break;
                }
                b => bail!("Invalid character `{}` in wad name", char::from(b)),
            };
            *dest = new_byte;
        }
        ensure!(nulled || value.len() <= 8, "Wad name too long");
        Ok(WadName(name))
    }
}

impl FromStr for WadName {
    type Err = anyhow::Error;
    fn from_str(value: &str) -> Result<WadName> {
        WadName::from_bytes(value.as_bytes())
    }
}

impl fmt::Display for WadName {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let s = str::from_utf8(&self.0).unwrap_or("???");
        let trimmed = s.trim_end_matches('\0');
        write!(f, "{}", trimmed)
    }
}

impl std::ops::Deref for WadName {
    type Target = [u8; 8];
    fn deref(&self) -> &[u8; 8] {
        &self.0
    }
}

impl fmt::Debug for WadName {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "WadName({})", self)
    }
}

impl PartialEq<[u8; 8]> for WadName {
    fn eq(&self, rhs: &[u8; 8]) -> bool {
        &self.0 == rhs
    }
}

impl Borrow<[u8; 8]> for WadName {
    fn borrow(&self) -> &[u8; 8] {
        &self.0
    }
}

impl AsRef<str> for WadName {
    fn as_ref(&self) -> &str {
        str::from_utf8(&self.0).expect("wad name is not valid utf-8")
    }
}

impl<'de> Deserialize<'de> for WadName {
    fn deserialize<D>(deserializer: D) -> StdResult<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        WadName::from_bytes(&<[u8; 8]>::deserialize(deserializer)?).map_err(D::Error::custom)
    }
}

pub trait IntoWadName {
    fn into_wad_name(self) -> Result<WadName>;
}

impl IntoWadName for &[u8] {
    fn into_wad_name(self) -> Result<WadName> {
        WadName::from_bytes(self)
    }
}

impl IntoWadName for &[u8; 8] {
    fn into_wad_name(self) -> Result<WadName> {
        WadName::from_bytes(self)
    }
}

impl IntoWadName for &str {
    fn into_wad_name(self) -> Result<WadName> {
        WadName::from_str(self)
    }
}

impl IntoWadName for WadName {
    fn into_wad_name(self) -> Result<WadName> {
        Ok(self)
    }
}
