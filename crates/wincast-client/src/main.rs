mod cli;
mod errors;
mod render_loop;
mod runtime;
mod stream;

#[cfg(test)]
mod runtime_tests;
#[cfg(test)]
mod test_support;

fn main() -> std::process::ExitCode {
    cli::main_entry()
}
