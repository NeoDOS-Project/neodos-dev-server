/// NEM v3 driver format parser.
/// Ported from scripts/mcp_server/parsers/nem_parser.py

pub const NEM_MAGIC_V3: &[u8; 4] = b"NEM3";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NemDriverType {
    Null,
    Echo,
    Lifecycle,
    Mutation,
    Fault,
    Burst,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NemCategory {
    Boot,
    System,
    Demand,
}

#[derive(Debug, Clone)]
pub struct NemHeaderV3 {
    pub magic: [u8; 4],
    pub abi_min: u16,
    pub abi_target: u16,
    pub abi_max: u16,
    pub driver_type: u32,
    pub category: u32,
    pub capabilities: u64,
    pub code_size: u32,
    pub reloc_count: u32,
    pub symbol_count: u32,
    pub hst_imports: u32,
}

#[derive(Debug, Clone)]
pub struct NemReloc {
    pub offset: u64,
    pub addend: i64,
    pub kind: u32,
}

#[derive(Debug, Clone)]
pub struct NemSymbol {
    pub name: String,
    pub address: u64,
    pub size: u64,
}

#[derive(Debug, Clone)]
pub struct NemDriver {
    pub header: NemHeaderV3,
    pub code: Vec<u8>,
    pub relocations: Vec<NemReloc>,
    pub symbols: Vec<NemSymbol>,
}

impl NemDriver {
    pub fn driver_type_name(&self) -> &'static str {
        match self.header.driver_type {
            0 => "Null", 1 => "Echo", 2 => "Lifecycle",
            3 => "Mutation", 4 => "Fault", 5 => "Burst",
            _ => "Unknown",
        }
    }

    pub fn category_name(&self) -> &'static str {
        match self.header.category {
            0 => "BOOT", 1 => "SYSTEM", 2 => "DEMAND",
            _ => "UNKNOWN",
        }
    }

    pub fn abi_compatible(&self, kernel_abi: u16) -> bool {
        self.header.abi_min <= kernel_abi && kernel_abi <= self.header.abi_max
    }
}

pub fn parse_nem_v3(data: &[u8]) -> Option<NemDriver> {
    if data.len() < 80 || &data[..4] != NEM_MAGIC_V3 {
        return None;
    }

    let header = NemHeaderV3 {
        magic: [data[0], data[1], data[2], data[3]],
        abi_min: crate::parsers::elf::read_u16(data, 8),
        abi_target: crate::parsers::elf::read_u16(data, 10),
        abi_max: crate::parsers::elf::read_u16(data, 12),
        driver_type: crate::parsers::elf::read_u32(data, 16),
        category: crate::parsers::elf::read_u32(data, 20),
        capabilities: crate::parsers::elf::read_u64(data, 24),
        code_size: crate::parsers::elf::read_u32(data, 32),
        reloc_count: crate::parsers::elf::read_u32(data, 36),
        symbol_count: crate::parsers::elf::read_u32(data, 40),
        hst_imports: crate::parsers::elf::read_u32(data, 44),
    };

    let code_start = 80usize;
    let code_end = code_start + header.code_size as usize;
    if code_end > data.len() { return None; }
    let code = data[code_start..code_end].to_vec();

    let mut offset = code_end;
    let mut relocations = Vec::new();
    for _ in 0..header.reloc_count {
        if offset + 20 > data.len() { break; }
        relocations.push(NemReloc {
            offset: crate::parsers::elf::read_u64(data, offset),
            addend: {
                let bytes = crate::parsers::elf::read_u64(data, offset + 8);
                i64::from_le_bytes(bytes.to_le_bytes())
            },
            kind: crate::parsers::elf::read_u32(data, offset + 16),
        });
        offset += 20;
    }

    let mut symbols = Vec::new();
    for _ in 0..header.symbol_count {
        if offset + 24 > data.len() { break; }
        let name_off = crate::parsers::elf::read_u64(data, offset) as usize;
        let addr = crate::parsers::elf::read_u64(data, offset + 8);
        let size = crate::parsers::elf::read_u64(data, offset + 16);
        let name = if name_off < data.len() {
            let end = data[name_off..].iter().position(|&b| b == 0).unwrap_or(0);
            String::from_utf8_lossy(&data[name_off..name_off + end]).to_string()
        } else {
            format!("sym_{}", symbols.len())
        };
        symbols.push(NemSymbol { name, address: addr, size });
        offset += 24;
    }

    Some(NemDriver { header, code, relocations, symbols })
}
