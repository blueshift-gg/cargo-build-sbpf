#![cfg_attr(target_arch = "bpf", no_std)]

#[cfg(target_arch = "bpf")]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}

#[cfg(target_arch = "bpf")]
mod program {
    const SMALL_LEN: usize = 32;
    const LARGE_LEN: usize = 128;

    static SMALL_EXPECTED: [u8; SMALL_LEN] = [3u8; SMALL_LEN];
    static LARGE_EXPECTED: [u8; LARGE_LEN] = [3u8; LARGE_LEN];

    #[inline(never)]
    fn compare_small(input: *const u8) -> bool {
        let actual =
            unsafe { core::slice::from_raw_parts(input.add(16), SMALL_LEN) };
        let expected = unsafe {
            core::slice::from_raw_parts(SMALL_EXPECTED.as_ptr(), SMALL_LEN)
        };
        !actual.ne(expected)
    }

    #[inline(never)]
    fn compare_large(input: *const u8) -> bool {
        let len = core::hint::black_box(LARGE_LEN);
        let actual =
            unsafe { core::slice::from_raw_parts(input.add(16), len) };
        let expected = unsafe {
            core::slice::from_raw_parts(LARGE_EXPECTED.as_ptr(), len)
        };
        !actual.ne(expected)
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn entrypoint(input: *mut u8) -> u64 {
        let len =
            unsafe { core::ptr::read_volatile((input as *const u64).add(1)) }
                as usize;

        let matched = match len {
            SMALL_LEN => compare_small(input),
            LARGE_LEN => compare_large(input),
            _ => return 2,
        };

        if matched {
            0
        } else {
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use mollusk_svm::Mollusk;
    use solana_instruction::Instruction;
    use solana_pubkey::Pubkey;

    const PROGRAM_STEM: &str = "target/deploy/input_loads";

    // Byte-wise loads of the unaligned input would cost far more than this.
    const SMALL_MATCH_COMPUTE_UNITS: u64 = 19;
    const LARGE_MATCH_COMPUTE_UNITS: u64 = 31;

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
        for (len, expected_compute_units) in
            [(32, SMALL_MATCH_COMPUTE_UNITS), (128, LARGE_MATCH_COMPUTE_UNITS)]
        {
            let (program_result, compute_units) = run(vec![3u8; len]);

            assert_eq!(program_result, "Success", "len: {len}");
            assert_eq!(compute_units, expected_compute_units, "len: {len}");
        }
    }

    #[test]
    fn corrupted_input_fails() {
        for len in [32, 128] {
            let mut data = vec![3u8; len];
            data[len / 2] = 4;
            let (program_result, _) = run(data);

            assert_eq!(program_result, "Failure(Custom(1))", "len: {len}");
        }
    }

    #[test]
    fn unexpected_length_fails() {
        for len in [0, 16, 64, 129] {
            let (program_result, _) = run(vec![3u8; len]);

            assert_eq!(program_result, "Failure(Custom(2))", "len: {len}");
        }
    }
}
