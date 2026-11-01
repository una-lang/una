//! The arm64 instruction fields that relocations patch (ADR 0124 rule 24), each with its bounds:
//! a value that does not fit its field is U8001, a branch beyond ±128 MiB is U7015.

use crate::problem::{Problem, Result};

/// The page of an address (4 KiB).
fn page(address: u64) -> i64 {
    i64::try_from(address & !0xfff).unwrap_or(i64::MAX)
}

/// `B` or `BL` at `place` reaching `target`.
pub(crate) fn branch26(instruction: u32, place: u64, target: u64) -> Result<u32> {
    if instruction & 0x7c00_0000 != 0x1400_0000 {
        return Err(Problem::internal(format!(
            "ARM64_RELOC_BRANCH26 at {place:#x} patches an instruction ({instruction:#010x}) that is not a branch"
        )));
    }
    let delta = i128::from(target) - i128::from(place);
    if delta % 4 != 0 {
        return Err(Problem::internal(format!(
            "ARM64_RELOC_BRANCH26 at {place:#x} reaches an unaligned target {target:#x}"
        )));
    }
    let words = delta / 4;
    if !(-(1i128 << 25)..(1i128 << 25)).contains(&words) {
        return Err(Problem::layout(format!(
            "a branch at {place:#x} to {target:#x} is beyond the ±128 MiB reach of arm64"
        )));
    }
    let field = u32::try_from(words & 0x03ff_ffff).unwrap_or(0);
    Ok((instruction & 0xfc00_0000) | field)
}

/// `ADRP` at `place` reaching the page of `target`.
pub(crate) fn page21(instruction: u32, place: u64, target: u64) -> Result<u32> {
    if instruction & 0x9f00_0000 != 0x9000_0000 {
        return Err(Problem::internal(format!(
            "a page relocation at {place:#x} patches an instruction ({instruction:#010x}) that is not ADRP"
        )));
    }
    let delta = (page(target) - page(place)) >> 12;
    if !(-(1i64 << 20)..(1i64 << 20)).contains(&delta) {
        return Err(Problem::layout(format!(
            "an ADRP at {place:#x} cannot reach {target:#x}, beyond ±4 GiB"
        )));
    }
    let delta = u32::try_from(delta & 0x1f_ffff).unwrap_or(0);
    let immlo = (delta & 0x3) << 29;
    let immhi = ((delta >> 2) & 0x7_ffff) << 5;
    Ok((instruction & 0x9f00_001f) | immlo | immhi)
}

/// The low 12 bits of `target` in an `ADD` (immediate) or a load or store with an unsigned
/// offset, scaled by the access size.
pub(crate) fn pageoff12(instruction: u32, place: u64, target: u64) -> Result<u32> {
    let low = u32::try_from(target & 0xfff).unwrap_or(0);
    let scale = if instruction & 0x1f00_0000 == 0x1100_0000 {
        // ADD/ADDS/SUB/SUBS (immediate): unscaled.
        0
    } else if instruction & 0x3b00_0000 == 0x3900_0000 {
        let size = instruction >> 30;
        let vector = (instruction >> 26) & 1;
        let opc = (instruction >> 22) & 3;
        if vector == 1 && size == 0 && opc & 2 != 0 {
            4
        } else {
            size
        }
    } else {
        return Err(Problem::internal(format!(
            "a page-offset relocation at {place:#x} patches an instruction ({instruction:#010x}) that takes no 12-bit offset"
        )));
    };
    if low & ((1 << scale) - 1) != 0 {
        return Err(Problem::internal(format!(
            "a page-offset relocation at {place:#x} reaches {target:#x}, not aligned to its access of {} bytes",
            1u32 << scale
        )));
    }
    let field = (low >> scale) & 0xfff;
    Ok((instruction & !(0xfff << 10)) | (field << 10))
}

/// Whether `instruction` is `LDR Xt, [Xn, #imm]` (64-bit, unsigned offset), the load of a GOT
/// or thread-local-variable pointer.
pub(crate) fn is_ldr64(instruction: u32) -> bool {
    instruction & 0xffc0_0000 == 0xf940_0000
}

/// The load of a pointer to a local `target` relaxed to its address: `LDR Xt, [Xn, #off]`
/// becomes `ADD Xt, Xn, #lo12(target)`, as `ld64` does, so the GOT holds no local pointer.
pub(crate) fn relax_ldr(instruction: u32, place: u64, target: u64) -> Result<u32> {
    if !is_ldr64(instruction) {
        return Err(Problem::internal(format!(
            "a GOT or TLV load at {place:#x} patches an instruction ({instruction:#010x}) that is not a 64-bit LDR"
        )));
    }
    let rt = instruction & 0x1f;
    let rn = (instruction >> 5) & 0x1f;
    let low = u32::try_from(target & 0xfff).unwrap_or(0);
    Ok(0x9100_0000 | (low << 10) | (rn << 5) | rt)
}

/// The three instructions of a stub that jumps through the GOT slot at `slot`:
/// `ADRP x16, slot@PAGE; LDR x16, [x16, slot@PAGEOFF]; BR x16`.
pub(crate) fn stub(place: u64, slot: u64) -> Result<[u32; 3]> {
    let adrp = page21(0x9000_0010, place, slot)?;
    let ldr = pageoff12(0xf940_0210, place.saturating_add(4), slot)?;
    Ok([adrp, ldr, 0xd61f_0200])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::problem::Code;

    #[test]
    fn a_branch_encodes_its_word_distance_and_refuses_beyond_its_reach() {
        // bl from 0x1000 to 0x2000: +0x400 words.
        assert_eq!(branch26(0x9400_0000, 0x1000, 0x2000), Ok(0x9400_0400));
        // backwards: -1 word.
        assert_eq!(branch26(0x1400_0000, 0x1004, 0x1000), Ok(0x17ff_ffff));
        let far = branch26(0x9400_0000, 0, 128 << 20).unwrap_err();
        assert_eq!(far.code, Code::U7015);
        assert!(branch26(0x9400_0000, 0, (128 << 20) - 4).is_ok());
        assert_eq!(branch26(0xd503_201f, 0, 4).unwrap_err().code, Code::U8001);
        assert_eq!(branch26(0x9400_0000, 0, 2).unwrap_err().code, Code::U8001);
    }

    #[test]
    fn adrp_reaches_the_page_and_add_or_load_take_the_offset() {
        // adrp x0 at 0x1_0000_0000 to 0x1_0000_5678: 5 pages.
        let adrp = page21(0x9000_0000, 0x1_0000_0000, 0x1_0000_5678).unwrap();
        assert_eq!(adrp, 0x9000_0000 | (1 << 29) | (1 << 5));
        // add x0, x0, #0x678
        assert_eq!(
            pageoff12(0x9100_0000, 0, 0x5678),
            Ok(0x9100_0000 | (0x678 << 10))
        );
        // ldr x0, [x0, #off] scales by 8.
        assert_eq!(
            pageoff12(0xf940_0000, 0, 0x5678),
            Ok(0xf940_0000 | ((0x678 / 8) << 10))
        );
        // ldr q0 (128-bit) scales by 16, and an unaligned offset is refused.
        assert_eq!(
            pageoff12(0x3dc0_0000, 0, 0x5670),
            Ok(0x3dc0_0000 | ((0x670 / 16) << 10))
        );
        assert!(pageoff12(0x3dc0_0000, 0, 0x5678).is_err());
        assert!(pageoff12(0xd503_201f, 0, 0).is_err());
        assert!(page21(0x9100_0000, 0, 0).is_err());
    }

    #[test]
    fn a_got_load_of_a_local_becomes_an_add() {
        // ldr x1, [x2, #0] -> add x1, x2, #0x10
        assert_eq!(relax_ldr(0xf940_0041, 0, 0x4010), Ok(0x9100_4041));
        assert!(relax_ldr(0xb940_0041, 0, 0).is_err());
    }

    #[test]
    fn a_stub_loads_its_slot_and_branches() {
        let [adrp, ldr, br] = stub(0x1_0000_1000, 0x1_0000_4008).unwrap();
        assert_eq!(adrp, 0x9000_0010 | (3 << 29));
        assert_eq!(ldr, 0xf940_0210 | (1 << 10));
        assert_eq!(br, 0xd61f_0200);
    }
}
