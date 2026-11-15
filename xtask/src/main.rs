//! Thin binary entry point; all logic lives in the `xtask` library so the
//! coverage tooling sees exactly one instrumented copy of each function.

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = match xtask::run(&args) {
        Ok(msg) => {
            println!("{msg}");
            0
        }
        Err(e) => {
            eprintln!("error: {e}");
            2
        }
    };
    std::process::exit(code);
}
