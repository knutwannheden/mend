use std::io::Read;
use std::io::Write;

fn main() {
    std::process::exit(run());
}

fn run() -> i32 {
    // Expect either one argument (the full patch) or read it from stdin.
    let mut args = std::env::args_os();
    let _argv0 = args.next();

    let patch = match args.next() {
        Some(arg) => match arg.into_string() {
            Ok(s) => s,
            Err(_) => {
                eprintln!("Error: patchwork requires a UTF-8 PATCH argument.");
                return 1;
            }
        },
        None => {
            let mut buf = String::new();
            match std::io::stdin().read_to_string(&mut buf) {
                Ok(_) => {
                    if buf.is_empty() {
                        eprintln!("Usage: patchwork 'PATCH'\n       echo 'PATCH' | patchwork");
                        return 2;
                    }
                    buf
                }
                Err(err) => {
                    eprintln!("Error: Failed to read PATCH from stdin.\n{err}");
                    return 1;
                }
            }
        }
    };

    // Refuse extra args to avoid ambiguity.
    if args.next().is_some() {
        eprintln!("Error: patchwork accepts exactly one argument.");
        return 2;
    }

    let cwd = match std::env::current_dir() {
        Ok(cwd) => cwd,
        Err(err) => {
            eprintln!("Error: Failed to determine current directory.\n{err}");
            return 1;
        }
    };
    let mut stdout = std::io::stdout();
    let mut stderr = std::io::stderr();
    match patchwork::apply_patch(&patch, &cwd, &mut stdout, &mut stderr) {
        Ok(()) => {
            // Flush to ensure output ordering when used in pipelines.
            let _ = stdout.flush();
            0
        }
        Err(_) => 1,
    }
}
