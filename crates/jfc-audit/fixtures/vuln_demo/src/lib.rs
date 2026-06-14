/// Command injection: user input flows directly to Command::new.
pub fn run_cmd(user_input: &str) {
    std::process::Command::new(user_input).output().unwrap();
}

/// Missing bounds check: direct indexing without validation.
pub fn at(v: &[u32], i: usize) -> u32 {
    v[i]
}

/// Division by zero: no check on denominator.
pub fn divide(a: i32, b: i32) -> i32 {
    a / b
}

/// Unsafe memory: Vec::from_raw_parts with capacity 0 (UB if len > 0).
pub unsafe fn bad_vec(ptr: *mut u8, len: usize) -> Vec<u8> {
    Vec::from_raw_parts(ptr, len, 0)
}

/// Dead code panic — no callers.
pub(crate) fn dead_panic() {
    panic!("unreachable")
}
