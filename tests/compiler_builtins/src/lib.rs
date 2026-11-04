#![cfg_attr(target_arch = "bpf", no_std)]

#[cfg(target_arch = "bpf")]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}

#[inline(never)]
fn float_add(a: f64, b: f64) -> f64 {
    a + b
}

#[inline(never)]
fn float_mul(a: f64, b: f64) -> f64 {
    a * b
}

#[inline(never)]
fn divmod128(n: u128, d: u128) -> (u128, u128) {
    (n / d, n % d)
}

const N: u128 = 0x1234_5678_9abc_def0_0fed_cba9_8765_4321;

/// Divisors covering each path of `__udivti3`/`__umodti3`: a 32-bit divisor,
/// a 64-bit one, one with a high half and a quotient of up to 32 bits, and
/// one with a high half and a wider quotient.
const DIVISORS: [u128; 4] = [
    0xffff_fff1,
    0x1_0000_0003_0000_0007,
    0x0123_4567_0000_0000_0000_0000_0000_0001,
    0x3_0000_0000_0000_0005,
];

/// The expected results, computed by rustc's const evaluator rather than by
/// the builtins under test.
const EXPECTED: [(u128, u128); 4] = {
    let mut expected = [(0, 0); 4];
    let mut i = 0;
    while i < DIVISORS.len() {
        expected[i] = (N / DIVISORS[i], N % DIVISORS[i]);
        i += 1;
    }
    expected
};

fn u128_division_works() -> bool {
    let mut i = 0;
    while i < DIVISORS.len() {
        let n = core::hint::black_box(N);
        let d = core::hint::black_box(DIVISORS[i]);
        if divmod128(n, d) != EXPECTED[i] {
            return false;
        }
        i += 1;
    }
    true
}

/// Overrides `__muldf3` builtin and returns hard-coded 7.0
#[unsafe(no_mangle)]
pub extern "C" fn __muldf3(_a: f64, _b: f64) -> f64 {
    7.0
}

#[unsafe(no_mangle)]
pub extern "C" fn entrypoint(input: *mut u8) -> u64 {
    let a = unsafe { core::ptr::read_volatile(input.add(16) as *const f64) };
    let b = unsafe { core::ptr::read_volatile(input.add(24) as *const f64) };
    let op = unsafe { core::ptr::read_volatile(input.add(32)) };

    if op == 2 {
        return if u128_division_works() { 0 } else { 1 };
    }

    let expected = if op == 0 {
        // 2.25 + 3.25 = 5.5
        5.5f64
    } else {
        // override always returns 7.0
        7.0f64
    };
    let result = if op == 0 { float_add(a, b) } else { float_mul(a, b) };

    if result.to_bits() == expected.to_bits() {
        0
    } else {
        1
    }
}

#[cfg(test)]
mod tests {
    use mollusk_svm::Mollusk;
    use solana_instruction::Instruction;
    use solana_pubkey::Pubkey;

    fn run(op: u8) -> String {
        let program_id = Pubkey::new_unique();
        let mollusk =
            Mollusk::new(&program_id, "target/deploy/compiler_builtins");
        let mut data = vec![0u8; 17];
        data[0..8].copy_from_slice(&2.25f64.to_le_bytes());
        data[8..16].copy_from_slice(&3.25f64.to_le_bytes());
        data[16] = op;
        let instruction = Instruction { program_id, accounts: vec![], data };
        let result = mollusk.process_instruction(&instruction, &[]);

        format!("{:?}", result.program_result)
    }

    #[test]
    fn test_compiler_builtins() {
        assert_eq!(run(0), "Success");
    }

    #[test]
    fn test_override_compiler_builtins() {
        assert_eq!(run(1), "Success");
    }

    #[test]
    fn test_u128_division() {
        assert_eq!(run(2), "Success");
    }
}
