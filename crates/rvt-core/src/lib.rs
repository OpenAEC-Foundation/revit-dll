//! Container layer of the RVT reader.
//!
//! Section numbers (§) refer to the clean-room specification (`SPEC_v0.11`); step numbers refer
//! to the clean-room notebook entry where a rule was found and verified.

pub mod cfb;
pub mod chain;
pub mod records;
pub mod schema;
pub mod tags;
mod tags_gen;

pub use cfb::Cfb;
pub use schema::Schema;
pub use chain::{chains, section_a_b, strip_inserted, Chain, Element};
pub use tags::vtag;

#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    Format(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Io(e) => write!(f, "I/O error: {e}"),
            Error::Format(m) => write!(f, "format error: {m}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}

pub type Result<T> = std::result::Result<T, Error>;

/// One `Partitions/*` stream with its chains.
pub struct Partition {
    pub stream: String,
    pub chains: Vec<Chain>,
    /// Chains whose element table could not be read (step 1448); they keep no elements.
    pub unreadable: Vec<(usize, String)>,
}

/// Opens a file and parses every partition down to element tables.
pub fn read_partitions(cfb: &Cfb) -> Result<Vec<Partition>> {
    let mut out = Vec::new();
    for (path, entry) in cfb.paths() {
        if !path.starts_with("Partitions/") {
            continue;
        }
        let raw = strip_inserted(&cfb.entry_bytes(&entry)?);
        let mut chs = chains(&raw)?;
        let mut unreadable = Vec::new();
        for ch in chs.iter_mut() {
            if let Err(e) = section_a_b(ch) {
                // step 1448: an unreadable (mini) chain must not stop the whole export
                ch.elements.clear();
                ch.bits = None;
                unreadable.push((ch.index, e.to_string().chars().take(120).collect()));
            }
        }
        out.push(Partition { stream: path, chains: chs, unreadable });
    }
    Ok(out)
}

macro_rules! le_reader {
    ($name:ident, $t:ty, $n:expr) => {
        #[inline]
        pub fn $name(b: &[u8], o: usize) -> Result<$t> {
            b.get(o..o + $n)
                .map(|s| <$t>::from_le_bytes(s.try_into().expect("slice length")))
                .ok_or_else(|| Error::Format(format!("read of {} bytes at {o} past end {}", $n, b.len())))
        }
    };
}

le_reader!(le_u16, u16, 2);
le_reader!(le_u32, u32, 4);
le_reader!(le_i32, i32, 4);
le_reader!(le_u64, u64, 8);
le_reader!(le_i64, i64, 8);
le_reader!(le_f64, f64, 8);
