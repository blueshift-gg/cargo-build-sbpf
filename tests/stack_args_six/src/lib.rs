#![cfg_attr(target_arch = "bpf", no_std)]

#[cfg(target_arch = "bpf")]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}

#[inline(never)]
#[unsafe(no_mangle)]
pub extern "C" fn add_six(
    a: u64,
    b: u64,
    c: u64,
    d: u64,
    e: u64,
    f: u64,
) -> u64 {
    a + b + c + d + e + f
}

#[unsafe(no_mangle)]
pub extern "C" fn entrypoint(input: *mut u8) -> u64 {
    let seed = u64::from(unsafe { *input.add(16) });

    let spilled =
        add_six(seed, seed ^ 2, seed ^ 3, seed ^ 4, seed ^ 5, seed ^ 6);
    let expected =
        seed + (seed ^ 2) + (seed ^ 3) + (seed ^ 4) + (seed ^ 5) + (seed ^ 6);

    if spilled == expected {
        0
    } else {
        1
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use mollusk_svm::Mollusk;
    use solana_instruction::Instruction;
    use solana_pubkey::Pubkey;

    const PROGRAM_STEM: &str = "target/deploy/stack_args_six";

    const COMPUTE_UNITS: u64 = 39;

    fn run(seed: u8) -> (String, u64) {
        assert!(
            Path::new(&format!("{PROGRAM_STEM}.so")).is_file(),
            "{PROGRAM_STEM}.so is missing; run `cargo build-sbpf` first"
        );

        let program_id = Pubkey::new_unique();
        let mollusk = Mollusk::new(&program_id, PROGRAM_STEM);
        let instruction =
            Instruction { program_id, accounts: vec![], data: vec![seed] };
        let result = mollusk.process_instruction(&instruction, &[]);

        (format!("{:?}", result.program_result), result.compute_units_consumed)
    }

    #[test]
    fn the_spilled_argument_arrives_intact_at_a_fixed_cost() {
        for seed in [0u8, 7, 200, 255] {
            let (program_result, compute_units) = run(seed);

            assert_eq!(program_result, "Success", "seed {seed}");
            assert_eq!(compute_units, COMPUTE_UNITS, "seed {seed}");
        }
    }
}
