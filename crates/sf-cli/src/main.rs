use std::fs::OpenOptions;
use std::io::{self, Write as _};
use std::path::PathBuf;

use clap::{Parser, Subcommand};
use sf_core::{BlockDevice, Imager, ImagerConfig, MapFile, Progress, Status};

#[derive(Parser)]
#[command(name = "sf-cli", about = "SectorForge command-line imaging engine", version)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Clone a raw device sector-by-sector into an image file, retrying and
    /// isolating bad sectors instead of giving up on the whole run.
    Image {
        /// Source device path, e.g. \\.\PhysicalDrive1 (Windows) or /dev/sdb (Linux)
        #[arg(long)]
        source: String,
        /// Destination image file.
        #[arg(long)]
        dest: PathBuf,
        /// Mapfile tracking progress — created if missing, reused to resume
        /// an interrupted run.
        #[arg(long)]
        map: PathBuf,
        /// Read size (bytes) on the fast first pass.
        #[arg(long, default_value_t = 1024 * 1024)]
        chunk_size: u64,
        /// How far to jump past a chunk that failed to read, before
        /// continuing the first pass.
        #[arg(long, default_value_t = 1024 * 1024)]
        skip_size: u64,
        /// Retries per sector before giving up on it as bad.
        #[arg(long, default_value_t = 3)]
        retries: u32,
        /// Skip the fast first pass and go straight to retrying
        /// already-known damaged regions from a previous run.
        #[arg(long)]
        retry_only: bool,
    },
    /// Print a summary of a mapfile's current state.
    Status {
        #[arg(long)]
        map: PathBuf,
    },
}

/// `\\.\PhysicalDrive1`, `\\.\PhysicalDrive1\` etc. (Windows) or `/dev/sdb`
/// (Linux) mean "direct hardware access"; anything else is treated as
/// "image file" mode — a plain file, usually an existing disk image.
fn looks_like_raw_device_path(path: &str) -> bool {
    path.starts_with(r"\\.\") || path.starts_with("/dev/")
}

fn open_device(path: &str) -> io::Result<Box<dyn BlockDevice>> {
    if looks_like_raw_device_path(path) {
        #[cfg(windows)]
        {
            return Ok(Box::new(sf_core::device::WindowsPhysicalDrive::open(path)?));
        }
        #[cfg(target_os = "linux")]
        {
            return Ok(Box::new(sf_core::device::LinuxBlockDevice::open(path)?));
        }
        #[cfg(not(any(windows, target_os = "linux")))]
        {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "no raw device backend for this platform yet",
            ));
        }
    }
    Ok(Box::new(sf_core::device::FileBlockDevice::open(path, 512)?))
}

fn format_bytes(b: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = b as f64;
    let mut u = 0;
    while v >= 1024.0 && u < UNITS.len() - 1 {
        v /= 1024.0;
        u += 1;
    }
    format!("{:.2} {}", v, UNITS[u])
}

fn print_progress(p: Progress) {
    let pct = if p.bytes_total > 0 {
        100.0 * (p.bytes_rescued + p.bytes_bad) as f64 / p.bytes_total as f64
    } else {
        0.0
    };
    print!(
        "\r{pct:>6.2}%  rescued {:<12} bad {:<10} pending {:<12} pos 0x{:X}        ",
        format_bytes(p.bytes_rescued),
        format_bytes(p.bytes_bad),
        format_bytes(p.bytes_pending),
        p.current_pos
    );
    let _ = io::stdout().flush();
}

fn print_status(m: &MapFile) {
    println!("Map size: {}", format_bytes(m.size()));
    println!("  Finished:     {}", format_bytes(m.bytes_with_status(Status::Finished)));
    println!("  Bad sectors:  {}", format_bytes(m.bytes_with_status(Status::BadSector)));
    println!("  Non-trimmed:  {}", format_bytes(m.bytes_with_status(Status::NonTrimmed)));
    println!("  Non-scraped:  {}", format_bytes(m.bytes_with_status(Status::NonScraped)));
    println!("  Non-tried:    {}", format_bytes(m.bytes_with_status(Status::NonTried)));
}

fn run_image(
    source: &str,
    dest: &PathBuf,
    map_path: &PathBuf,
    chunk_size: u64,
    skip_size: u64,
    retries: u32,
    retry_only: bool,
) -> anyhow::Result<()> {
    let device = open_device(source)?;
    let size = device.size_bytes();

    let map = if map_path.exists() {
        println!("Resuming from existing map {}", map_path.display());
        MapFile::load(map_path)?
    } else {
        println!("Starting new map for {source} ({})", format_bytes(size));
        let m = MapFile::new_non_tried(size);
        m.save(map_path)?;
        m
    };

    let dest_file = OpenOptions::new().write(true).create(true).open(dest)?;
    dest_file.set_len(size)?;

    let config = ImagerConfig {
        initial_chunk: chunk_size,
        skip_size,
        max_retries: retries,
    };
    let mut imager = Imager::new(device, dest_file, map, config);

    if !retry_only {
        println!("\nPass 1: fast copy, skipping past damage");
        imager.run_copy_pass(print_progress)?;
        imager.save_map(map_path)?;
        println!();
    }

    println!("Pass 2: retrying damaged regions sector-by-sector");
    imager.run_retry_pass(print_progress)?;
    imager.save_map(map_path)?;
    println!();

    print_status(imager.map());
    Ok(())
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Image {
            source,
            dest,
            map,
            chunk_size,
            skip_size,
            retries,
            retry_only,
        } => run_image(&source, &dest, &map, chunk_size, skip_size, retries, retry_only)?,
        Command::Status { map } => {
            let m = MapFile::load(&map)?;
            print_status(&m);
        }
    }
    Ok(())
}
