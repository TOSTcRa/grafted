use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;

use grafted_loader::MachOBinary;

/// Grafted — run Darwin binaries on Linux.
#[derive(Parser, Debug)]
#[command(name = "grafted", version, about)]
struct Args {
    /// Path to the Mach-O binary to execute.
    binary: PathBuf,

    /// Arguments to pass to the binary.
    #[arg(trailing_var_arg = true)]
    args: Vec<String>,

    /// Print parsed Mach-O info without executing.
    #[arg(long)]
    inspect: bool,

    /// Verbosity level (-v, -vv, -vvv).
    #[arg(short, long, action = clap::ArgAction::Count)]
    verbose: u8,
}

fn main() -> ExitCode {
    let args = Args::parse();

    // Init logging based on verbosity
    let log_level = match args.verbose {
        0 => "warn",
        1 => "info",
        2 => "debug",
        _ => "trace",
    };
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or(log_level)).init();

    let binary = match MachOBinary::from_path(&args.binary) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("grafted: {}: {e}", args.binary.display());
            return ExitCode::FAILURE;
        }
    };

    if args.inspect {
        print_info(&binary);
        return ExitCode::SUCCESS;
    }

    let entry_point = match grafted_loader::mapper::map_binary(&binary) {
        Ok(ep) => ep,
        Err(e) => {
            eprintln!("grafted: failed to map binary: {e}");
            return ExitCode::FAILURE;
        }
    };

    // Execute — this does not return (the binary's exit() terminates the process)
    grafted_loader::executor::execute(entry_point)
}

fn print_info(binary: &MachOBinary) {
    println!("Mach-O Binary: {}", binary.path);
    println!(
        "  Type: {:#x} ({})",
        binary.file_type,
        match binary.file_type {
            0x2 => "MH_EXECUTE",
            0x6 => "MH_DYLIB",
            0x8 => "MH_BUNDLE",
            _ => "unknown",
        }
    );
    println!("  CPU type: {:#x}", binary.cpu_type);
    println!(
        "  Entry point: {:#x} ({})",
        binary.entry_point,
        if binary.entry_is_offset {
            "offset from __TEXT"
        } else {
            "absolute"
        }
    );

    println!("  Segments:");
    for seg in &binary.segments {
        println!(
            "    {:<16} vmaddr={:#014x} vmsize={:#010x} fileoff={:#010x} filesize={:#010x}",
            seg.name, seg.vmaddr, seg.vmsize, seg.fileoff, seg.filesize
        );
    }

    if !binary.dylib_deps.is_empty() {
        println!("  Dependencies:");
        for dep in &binary.dylib_deps {
            println!("    {}", dep.name);
        }
    }
}
