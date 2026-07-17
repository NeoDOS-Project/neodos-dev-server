/// ABI compatibility checking for NEM drivers and NXL libraries.

#[derive(Debug, Clone, Copy)]
pub struct AbiVersion {
    pub min: u16,
    pub target: u16,
    pub max: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AbiCompatibility {
    Compatible,
    TooOld { required: u16, available: u16 },
    TooNew { required: u16, available: u16 },
}

pub const KERNEL_ABI_VERSION: u16 = 8;

pub fn check_nem_abi(driver: &AbiVersion) -> AbiCompatibility {
    if KERNEL_ABI_VERSION < driver.min {
        AbiCompatibility::TooOld {
            required: driver.min,
            available: KERNEL_ABI_VERSION,
        }
    } else if KERNEL_ABI_VERSION > driver.max {
        AbiCompatibility::TooNew {
            required: driver.max,
            available: KERNEL_ABI_VERSION,
        }
    } else {
        AbiCompatibility::Compatible
    }
}

pub fn format_compatibility(result: &AbiCompatibility) -> String {
    match result {
        AbiCompatibility::Compatible => "✓ Compatible".to_string(),
        AbiCompatibility::TooOld { required, available } => {
            format!("✗ Driver requires ABI ≥{required}, kernel has {available}")
        }
        AbiCompatibility::TooNew { required, available } => {
            format!("✗ Driver requires ABI ≤{required}, kernel has {available}")
        }
    }
}
