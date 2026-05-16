use std::process::ExitCode;

fn main() -> ExitCode {
    match wincast_client::run_default_client() {
        Ok(message) => {
            println!("{message}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}
