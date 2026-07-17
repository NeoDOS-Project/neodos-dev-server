/// NeoDOS Registry Hive (NEOH) parser.
/// Ported from scripts/mcp_server/parsers/registry_hive.py

pub const HIVE_MAGIC: u32 = 0x484F454E; // b"NEOH"

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CellType {
    Free,
    Key,
    Value,
    Security,
}

#[derive(Debug, Clone)]
pub struct KeyCell {
    pub name: String,
    pub parent: u64,
    pub subkey_count: u32,
    pub value_count: u32,
}

#[derive(Debug, Clone)]
pub struct ValueCell {
    pub name: String,
    pub value_type: u32,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct SecurityCell {
    pub sid: Vec<u8>,
    pub acl: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct RegistryHive {
    pub magic: u32,
    pub version: u32,
    pub entry_count: u32,
    pub checksum: u32,
    pub keys: Vec<KeyCell>,
    pub values: Vec<ValueCell>,
}

pub fn parse_hive(data: &[u8]) -> Option<RegistryHive> {
    if data.len() < 16 { return None; }
    let magic = crate::parsers::elf::read_u32(data, 0);
    if magic != HIVE_MAGIC { return None; }
    Some(RegistryHive {
        magic,
        version: crate::parsers::elf::read_u32(data, 4),
        entry_count: crate::parsers::elf::read_u32(data, 8),
        checksum: crate::parsers::elf::read_u32(data, 12),
        keys: Vec::new(),
        values: Vec::new(),
    })
}
