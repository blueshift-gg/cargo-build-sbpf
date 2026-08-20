#![cfg_attr(target_arch = "bpf", no_std)]

#[cfg(target_arch = "bpf")]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}

const TABLE: [u8; 8] = [0x11, 1, 2, 3, 4, 5, 6, 7];

static NAMED: [u8; 8] = [0x22, 8, 9, 10, 11, 12, 13, 14];

#[unsafe(no_mangle)]
pub extern "C" fn entrypoint(input: *mut u8) -> u64 {
    let index = usize::from(unsafe { *input }) & 7;
    let from_const = TABLE[index];
    let from_static = NAMED[index];
    u64::from(from_const) + u64::from(from_static)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use mollusk_svm::Mollusk;
    use solana_instruction::Instruction;
    use solana_pubkey::Pubkey;

    const PROGRAM_STEM: &str = "target/deploy/const_rodata";

    const COMPUTE_UNITS: u64 = 10;

    fn run() -> (String, u64) {
        assert!(
            Path::new(&format!("{PROGRAM_STEM}.so")).is_file(),
            "{PROGRAM_STEM}.so is missing; run `cargo build-sbpf` first"
        );

        let program_id = Pubkey::new_unique();
        let mollusk = Mollusk::new(&program_id, PROGRAM_STEM);
        let instruction =
            Instruction { program_id, accounts: vec![], data: vec![] };
        let result = mollusk.process_instruction(&instruction, &[]);

        (format!("{:?}", result.program_result), result.compute_units_consumed)
    }

    #[test]
    fn returns_the_rodata_bytes_at_a_fixed_cost() {
        let (program_result, compute_units) = run();

        assert_eq!(program_result, "Failure(Custom(51))");
        assert_eq!(compute_units, COMPUTE_UNITS);
    }
}
