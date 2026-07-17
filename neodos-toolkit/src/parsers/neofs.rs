/// NeoFS v2 (NE2) offline parser.
/// Ported from scripts/mcp_server/parsers/neodos_v2_fs.py

pub const BLOCK_SIZE: usize = 4096;
pub const SECTOR_SIZE: usize = 512;
pub const SUPERBLOCK_MAGIC: u32 = 0x0032454E; // b"NE2\0"

#[derive(Debug, Clone)]
pub struct SuperblockNE2 {
    pub magic: u32,
    pub version: u32,
    pub total_blocks: u64,
    pub root_inode: u64,
    pub free_blocks: u64,
    pub block_size: u32,
}

#[derive(Debug, Clone)]
pub struct DirEntryV2 {
    pub name: String,
    pub inode: u64,
    pub mode: u16,
    pub size: u64,
}

#[derive(Debug, Clone)]
pub struct ResolvedInfo {
    pub inode: u64,
    pub size: u64,
    pub is_dir: bool,
}

pub fn parse_superblock(data: &[u8]) -> Option<SuperblockNE2> {
    if data.len() < 32 { return None; }
    let magic = crate::parsers::elf::read_u32(data, 0);
    if magic != SUPERBLOCK_MAGIC { return None; }
    Some(SuperblockNE2 {
        magic,
        version: crate::parsers::elf::read_u32(data, 4),
        total_blocks: crate::parsers::elf::read_u64(data, 8),
        root_inode: crate::parsers::elf::read_u64(data, 16),
        free_blocks: crate::parsers::elf::read_u64(data, 24),
        block_size: crate::parsers::elf::read_u32(data, 28),
    })
}
