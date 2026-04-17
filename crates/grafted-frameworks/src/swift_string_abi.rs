//! Swift String ABI translation layer.

use crate::cf::string::CFStringInner;

#[repr(C)]
#[derive(Copy, Clone, Debug)]
pub struct DarwinString {
    pub word0: u64,
    pub word1: u64,
}

#[repr(C)]
#[derive(Copy, Clone, Debug)]
pub struct LinuxString {
    pub word0: u64,
    pub word1: u64,
}

impl DarwinString {
    /// Detects if this is a Darwin "Small String" (packed UTF-8).
    /// On Darwin x86_64, small strings have the high nibble of word1 set to 0xE.
    pub fn is_small(&self) -> bool {
        (self.word1 >> 60) == 0xE
    }

    /// Detects if this is an NSString-bridged string.
    /// Bit 62 of word1 is the "is-bridged" bit on Darwin.
    pub fn is_bridged(&self) -> bool {
        (self.word1 >> 62) & 1 == 1
    }

    /// Translates this Darwin string into a Linux-compatible Swift String.
    pub unsafe fn to_linux_string(&self) -> LinuxString {
        let mut bytes = [0u8; 15];
        let mut count = 0usize;

        if self.is_small() {
            count = ((self.word1 >> 56) & 0x0F) as usize;
            
            // Extract bytes from word0 and word1
            let w0_bytes = self.word0.to_le_bytes();
            let w1_bytes = self.word1.to_le_bytes();
            
            bytes[0..8].copy_from_slice(&w0_bytes);
            bytes[8..15].copy_from_slice(&w1_bytes[0..7]);
        } else if self.is_bridged() {
            // Word0 is the NSString pointer (toll-free bridged to CFString)
            let ns_ptr = self.word0 as *const CFStringInner;
            if !ns_ptr.is_null() {
                let s_bytes = unsafe { &(*ns_ptr).bytes };
                count = s_bytes.len().min(15);
                bytes[..count].copy_from_slice(&s_bytes[..count]);
            }
        } else {
            // Fallback: treat as large native string.
            log::warn!("DarwinString: fallback for non-small/non-bridged string: w0={:x} w1={:x}", self.word0, self.word1);
            return LinuxString { word0: self.word0, word1: self.word1 };
        }

        // Pack into Linux small string format (x86_64)
        let mut lw0 = 0u64;
        let mut lw1 = 0u64;
        
        for i in 0..8 {
            if i < count {
                lw0 |= (bytes[i] as u64) << (i * 8);
            }
        }
        for i in 8..15 {
            if i < count {
                lw1 |= (bytes[i] as u64) << ((i - 8) * 8);
            }
        }
        lw1 |= (0xE0 | (count as u64)) << 56;

        LinuxString { word0: lw0, word1: lw1 }
    }
}
