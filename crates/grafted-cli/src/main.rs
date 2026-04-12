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

    // Per-thread selectors are now managed automatically by executor + shims.

    // Tell executor where the binary's __TEXT is mapped (affects SUD range selection)
    if let Some(text_seg) = binary.segments.iter().find(|s| s.name == "__TEXT") {
        grafted_loader::executor::set_darwin_text_base(text_seg.vmaddr);
    }

    // Set up TLV init image from __thread_data section so thread-locals get
    // correct initial values (not just zeros).
    if let Some((data_addr, data_size, total_size)) = binary.tlv_init_image() {
        log::debug!("TLV init image: addr={data_addr:#x} data_size={data_size:#x} total={total_size:#x}");
        grafted_dyld::shims::set_tlv_init_image(
            data_addr as *const u8,
            data_size,
            total_size,
        );
    }

    let mut linker = grafted_dyld::Linker::new();
    
    if let Err(e) = linker.load_dependencies(&binary) {
        eprintln!("grafted: failed to load dependencies: {e}");
        return ExitCode::FAILURE;
    }

    if binary.file_type == 0x2 { // MH_EXECUTE
        log::debug!("binary is MH_EXECUTE, running linker");
        match linker.bind(&binary.data) {
            Ok(n) => {
                if n > 0 {
                    log::info!("resolved {n} dynamic imports");
                }
            }
            Err(e) => {
                eprintln!("grafted: link error: {e}");
                return ExitCode::FAILURE;
            }
        }

        if binary.chained_fixups.is_some() {
            if let Err(e) = grafted_loader::chained_fixups::apply_fixups(&binary, |dylib, name| {
                linker.resolve_external(dylib, name)
            }) {
                eprintln!("grafted: fixup error: {e}");
                return ExitCode::FAILURE;
            }
        }

        // Resolve __nl_symbol_ptr entries for binaries that DON'T use chained fixups
        // (e.g., Go binaries). Don't overwrite entries already resolved by apply_fixups.
        if binary.chained_fixups.is_none() {
            match linker.bind_nl_symbol_ptrs(&binary) {
                Ok(n) if n > 0 => log::info!("resolved {n} nl_symbol_ptr entries"),
                Err(e) => log::warn!("nl_symbol_ptr binding: {e}"),
                _ => {}
            }
        }

        // Translate Swift metadata layout (Mach-O → Linux compatible)
        // Must run AFTER fixups (which resolve absolute pointers) but BEFORE
        // section registration (which hands metadata to the runtime).
        grafted_frameworks::swift_metadata_translate::translate_swift_metadata(&binary.data);

        // Register Swift metadata sections with the Linux Swift runtime
        grafted_frameworks::swift_sections::register_swift_sections(&binary.data);

        if let Err(e) = linker.run_all_initializers(&binary) {
            eprintln!("grafted: failed to run initializers: {e}");
            return ExitCode::FAILURE;
        }
    }

    // Patch Go runtime.settls if this is a Go binary (sets GS base for TLS)
    grafted_loader::executor::patch_go_settls(&binary);

    let mut argv = vec![args.binary.display().to_string()];
    argv.extend(args.args.iter().cloned());
    grafted_dyld::shims::set_process_info(&argv[0], &argv);
    grafted_loader::executor::execute(entry_point, &argv, &argv[0], binary.entry_is_offset, |base, size| {
        grafted_dyld::shims::set_stack_bounds(base, size);
    })
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
