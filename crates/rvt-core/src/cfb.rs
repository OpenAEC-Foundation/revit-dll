//! Minimal compound file reader following [MS-CFB] (Microsoft Open Specifications).
//!
//! Supports the header, DIFAT (including extra DIFAT sectors), FAT, directory, regular streams
//! and mini streams through the MiniFAT. Port of `cfb_own.py` from the clean-room project.

use std::collections::BTreeMap;

use crate::{le_u16, le_u32, le_u64, Error, Result};

const SIGNATURE: [u8; 8] = [0xd0, 0xcf, 0x11, 0xe0, 0xa1, 0xb1, 0x1a, 0xe1];
const MAXREGSECT: u32 = 0xFFFF_FFFA;
const NOSTREAM: u32 = 0xFFFF_FFFF;

#[derive(Debug, Clone)]
pub struct DirEntry {
    pub name: String,
    pub kind: u8,
    left: u32,
    right: u32,
    child: u32,
    start: u32,
    pub size: u64,
}

/// An opened compound file, held fully in memory.
pub struct Cfb {
    data: Vec<u8>,
    sector_size: usize,
    mini_sector_size: usize,
    cutoff: u64,
    fat: Vec<u32>,
    minifat: Vec<u32>,
    ministream: Vec<u8>,
    entries: Vec<DirEntry>,
}

impl Cfb {
    pub fn open(path: impl AsRef<std::path::Path>) -> Result<Self> {
        Self::from_bytes(std::fs::read(path)?)
    }

    pub fn from_bytes(data: Vec<u8>) -> Result<Self> {
        if data.len() < 512 || data[..8] != SIGNATURE {
            return Err(Error::Format("no compound file signature".into()));
        }
        let sector_size = 1usize << le_u16(&data, 0x1E)?;
        let mini_sector_size = 1usize << le_u16(&data, 0x20)?;
        let n_fat = le_u32(&data, 0x2C)? as usize;
        let dir_start = le_u32(&data, 0x30)?;
        let cutoff = le_u32(&data, 0x38)? as u64;
        let minifat_start = le_u32(&data, 0x3C)?;
        let n_minifat = le_u32(&data, 0x40)?;
        let difat_start = le_u32(&data, 0x44)?;
        let n_difat = le_u32(&data, 0x48)?;

        let mut cfb = Cfb {
            data,
            sector_size,
            mini_sector_size,
            cutoff,
            fat: Vec::new(),
            minifat: Vec::new(),
            ministream: Vec::new(),
            entries: Vec::new(),
        };

        let mut difat = Vec::with_capacity(109);
        for i in 0..109 {
            difat.push(le_u32(&cfb.data, 0x4C + 4 * i)?);
        }
        let mut s = difat_start;
        for _ in 0..n_difat {
            let sec = cfb.sector(s)?;
            let per = sector_size / 4 - 1;
            for i in 0..per {
                difat.push(le_u32(sec, 4 * i)?);
            }
            s = le_u32(sec, sector_size - 4)?;
        }
        let fat_sectors: Vec<u32> = difat.into_iter().filter(|&x| x < MAXREGSECT).take(n_fat).collect();
        let mut fat = Vec::with_capacity(fat_sectors.len() * sector_size / 4);
        for fs in fat_sectors {
            let sec = cfb.sector(fs)?;
            for i in 0..sector_size / 4 {
                fat.push(le_u32(sec, 4 * i)?);
            }
        }
        cfb.fat = fat;
        cfb.entries = cfb.read_dir(dir_start)?;
        let root = cfb.entries.first().ok_or_else(|| Error::Format("empty directory".into()))?.clone();
        cfb.ministream = cfb.chain_bytes(root.start, Some(root.size))?;
        if n_minifat > 0 {
            let mf = cfb.chain_bytes(minifat_start, None)?;
            cfb.minifat = (0..mf.len() / 4).map(|i| le_u32(&mf, 4 * i)).collect::<Result<_>>()?;
        }
        Ok(cfb)
    }

    fn sector(&self, sid: u32) -> Result<&[u8]> {
        let off = (sid as usize + 1) * self.sector_size;
        self.data
            .get(off..off + self.sector_size)
            .ok_or_else(|| Error::Format(format!("sector {sid} out of range")))
    }

    fn chain(&self, start: u32) -> Result<Vec<u32>> {
        let mut out = Vec::new();
        let mut s = start;
        while s < MAXREGSECT {
            out.push(s);
            s = *self.fat.get(s as usize).ok_or_else(|| Error::Format("FAT index out of range".into()))?;
            if out.len() > self.fat.len() {
                return Err(Error::Format("FAT loop".into()));
            }
        }
        Ok(out)
    }

    fn chain_bytes(&self, start: u32, size: Option<u64>) -> Result<Vec<u8>> {
        let mut data = Vec::new();
        for s in self.chain(start)? {
            data.extend_from_slice(self.sector(s)?);
        }
        if let Some(n) = size {
            data.truncate(n as usize);
        }
        Ok(data)
    }

    fn read_dir(&self, dir_start: u32) -> Result<Vec<DirEntry>> {
        let raw = self.chain_bytes(dir_start, None)?;
        let mut ents = Vec::with_capacity(raw.len() / 128);
        for e in raw.chunks_exact(128) {
            let nl = le_u16(e, 64)? as usize;
            let units: Vec<u16> = e[..nl.saturating_sub(2).min(64)]
                .chunks_exact(2)
                .map(|c| u16::from_le_bytes([c[0], c[1]]))
                .collect();
            ents.push(DirEntry {
                name: String::from_utf16_lossy(&units),
                kind: e[66],
                left: le_u32(e, 68)?,
                right: le_u32(e, 72)?,
                child: le_u32(e, 76)?,
                start: le_u32(e, 116)?,
                size: le_u64(e, 120)?,
            });
        }
        Ok(ents)
    }

    /// Path -> entry, walking the red-black tree (child/left/right).
    pub fn paths(&self) -> BTreeMap<String, DirEntry> {
        let mut out = BTreeMap::new();
        let mut stack = vec![(self.entries[0].child, String::new())];
        while let Some((idx, prefix)) = stack.pop() {
            if idx == NOSTREAM || idx as usize >= self.entries.len() || out.len() > self.entries.len() {
                continue;
            }
            let e = &self.entries[idx as usize];
            let p = format!("{prefix}{}", e.name);
            stack.push((e.left, prefix.clone()));
            stack.push((e.right, prefix.clone()));
            if e.kind == 1 {
                stack.push((e.child, format!("{p}/")));
            }
            out.insert(p, e.clone());
        }
        out
    }

    pub fn stream(&self, path: &str) -> Result<Vec<u8>> {
        let e = self.paths().remove(path).ok_or_else(|| Error::Format(format!("no stream {path}")))?;
        self.entry_bytes(&e)
    }

    pub fn entry_bytes(&self, e: &DirEntry) -> Result<Vec<u8>> {
        if e.size < self.cutoff {
            let mut out = Vec::with_capacity(e.size as usize);
            let mut s = e.start;
            while s < MAXREGSECT && (out.len() as u64) < e.size {
                let off = s as usize * self.mini_sector_size;
                out.extend_from_slice(
                    self.ministream
                        .get(off..off + self.mini_sector_size)
                        .ok_or_else(|| Error::Format("mini sector out of range".into()))?,
                );
                s = *self.minifat.get(s as usize).ok_or_else(|| Error::Format("MiniFAT index out of range".into()))?;
            }
            out.truncate(e.size as usize);
            return Ok(out);
        }
        self.chain_bytes(e.start, Some(e.size))
    }
}
