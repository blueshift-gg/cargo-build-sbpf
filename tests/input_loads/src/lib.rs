#![cfg_attr(target_arch = "bpf", no_std)]

#[cfg(target_arch = "bpf")]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}

const PUBKEY_LEN: usize = 32;

static EXPECTED: [u8; PUBKEY_LEN] = [3u8; PUBKEY_LEN];

#[unsafe(no_mangle)]
pub extern "C" fn entrypoint(input: *mut u8) -> u64 {
    let pubkey =
        unsafe { core::slice::from_raw_parts(input.add(16), PUBKEY_LEN) };
    let expected =
        unsafe { core::slice::from_raw_parts(EXPECTED.as_ptr(), PUBKEY_LEN) };

    if pubkey.ne(expected) {
        1
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use mollusk_svm::Mollusk;
    use solana_instruction::Instruction;
    use solana_pubkey::Pubkey;

    const PROGRAM_STEM: &str = "target/deploy/input_loads";

    const MATCHING_INPUT_COMPUTE_UNITS: u64 = 11;
    const DIFFERING_INPUT_COMPUTE_UNITS: u64 = 6;

    fn run(data: Vec<u8>) -> (String, u64) {
        assert!(
            Path::new(&format!("{PROGRAM_STEM}.so")).is_file(),
            "{PROGRAM_STEM}.so is missing; run `cargo build-sbpf` first"
        );

        let program_id = Pubkey::new_unique();
        let mollusk = Mollusk::new(&program_id, PROGRAM_STEM);
        let instruction = Instruction { program_id, accounts: vec![], data };
        let result = mollusk.process_instruction(&instruction, &[]);

        (format!("{:?}", result.program_result), result.compute_units_consumed)
    }

    #[test]
    fn matching_input_succeeds_at_a_fixed_cost() {
        let (program_result, compute_units) = run(vec![3u8; 32]);

        assert_eq!(program_result, "Success");
        assert_eq!(compute_units, MATCHING_INPUT_COMPUTE_UNITS);
    }

    #[test]
    fn differing_input_fails_at_a_fixed_cost() {
        let (program_result, compute_units) = run(vec![4u8; 32]);

        assert_eq!(program_result, "Failure(Custom(1))");
        assert_eq!(compute_units, DIFFERING_INPUT_COMPUTE_UNITS);
    }
}
