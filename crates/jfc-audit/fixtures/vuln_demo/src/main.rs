use vuln_demo::{at, bad_vec, divide, run_cmd};

fn main() {
    let input = std::env::args().nth(1).unwrap_or_else(|| "echo hello".to_string());

    run_cmd(&input);

    let data = vec![1, 2, 3, 4, 5];
    let val = at(&data, 2);
    println!("at(2) = {val}");

    let result = divide(10, 2);
    println!("divide(10, 2) = {result}");

    // Unsafe usage demo (don't actually run this with real data)
    let mut buf = vec![0u8; 16];
    let ptr = buf.as_mut_ptr();
    let len = buf.len();
    std::mem::forget(buf);
    let recovered = unsafe { bad_vec(ptr, len) };
    println!("recovered {} bytes", recovered.len());
}
