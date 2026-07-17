/// ELF64 binary parser.
/// Ported from tools/nxdump and scripts/mcp_server/parsers/elf_parser.py

#[derive(Debug, Clone)]
pub struct ElfHeader {
    pub e_type: u16,
    pub e_machine: u16,
    pub e_entry: u64,
    pub e_phoff: u64,
    pub e_shoff: u64,
    pub e_phnum: u16,
    pub e_shnum: u16,
    pub e_shentsize: u16,
}

#[derive(Debug, Clone)]
pub struct ProgramHeader {
    pub p_type: u32,
    pub p_flags: u32,
    pub p_offset: u64,
    pub p_vaddr: u64,
    pub p_paddr: u64,
    pub p_filesz: u64,
    pub p_memsz: u64,
    pub p_align: u64,
}

#[derive(Debug, Clone)]
pub struct SectionHeader {
    pub sh_name: u32,
    pub sh_type: u32,
    pub sh_flags: u64,
    pub sh_addr: u64,
    pub sh_offset: u64,
    pub sh_size: u64,
    pub sh_link: u32,
    pub sh_info: u32,
    pub sh_addralign: u64,
    pub sh_entsize: u64,
}

#[derive(Debug, Clone)]
pub struct ElfBinary {
    pub header: ElfHeader,
    pub segments: Vec<ProgramHeader>,
    pub sections: Vec<SectionHeader>,
    pub data: Vec<u8>,
}

pub fn parse_elf(data: &[u8]) -> Option<ElfBinary> {
    if data.len() < 64 || &data[..4] != b"\x7fELF" {
        return None;
    }

    let header = ElfHeader {
        e_type: read_u16(data, 16),
        e_machine: read_u16(data, 18),
        e_entry: read_u64(data, 24),
        e_phoff: read_u64(data, 32),
        e_shoff: read_u64(data, 40),
        e_phnum: read_u16(data, 56),
        e_shnum: read_u16(data, 60),
        e_shentsize: read_u16(data, 58),
    };

    let mut segments = Vec::new();
    for i in 0..header.e_phnum as u64 {
        let off = (header.e_phoff + i * 56) as usize;
        if off + 56 > data.len() { break; }
        segments.push(ProgramHeader {
            p_type: read_u32(data, off),
            p_flags: read_u32(data, off + 4),
            p_offset: read_u64(data, off + 8),
            p_vaddr: read_u64(data, off + 16),
            p_paddr: read_u64(data, off + 24),
            p_filesz: read_u64(data, off + 32),
            p_memsz: read_u64(data, off + 40),
            p_align: read_u64(data, off + 48),
        });
    }

    let mut sections = Vec::new();
    if header.e_shoff > 0 && header.e_shentsize >= 64 {
        for i in 0..header.e_shnum as u64 {
            let off = (header.e_shoff + i * header.e_shentsize as u64) as usize;
            if off + 64 > data.len() { break; }
            sections.push(SectionHeader {
                sh_name: read_u32(data, off),
                sh_type: read_u32(data, off + 4),
                sh_flags: read_u64(data, off + 8),
                sh_addr: read_u64(data, off + 12),
                sh_offset: read_u64(data, off + 24),
                sh_size: read_u64(data, off + 32),
                sh_link: read_u32(data, off + 40),
                sh_info: read_u32(data, off + 44),
                sh_addralign: read_u64(data, off + 48),
                sh_entsize: read_u64(data, off + 56),
            });
        }
    }

    Some(ElfBinary { header, segments, sections, data: data.to_vec() })
}

pub fn read_u16(data: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([data[offset], data[offset + 1]])
}

pub fn read_u32(data: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([data[offset], data[offset + 1], data[offset + 2], data[offset + 3]])
}

pub fn read_u64(data: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes([
        data[offset], data[offset + 1], data[offset + 2], data[offset + 3],
        data[offset + 4], data[offset + 5], data[offset + 6], data[offset + 7],
    ])
}
