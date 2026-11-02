//! sf-cli: the SectorForge engine as a command-line tool. With `--json` it
//! speaks the NDJSON contract in `docs/cli-contract.md`, which is how the GUI
//! drives it.

mod recover;

use std::ffi::OsString;
use std::fs::OpenOptions;
use std::io::{self, Write as _};
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::RwLock;
use std::time::{Duration, Instant};

use clap::{Parser, Subcommand};
use serde_json::{json, Value};
use sf_core::device::Access;
use sf_core::partitions;
use sf_core::{BlockDevice, Confidence, FsKind, Imager, ImagerConfig, MapFile, PartitionSource, Progress, Status, TableKind};

/// Bumped whenever the JSON contract changes incompatibly.
const CONTRACT_VERSION: u32 = 2;
const CHECKPOINT_INTERVAL: Duration = Duration::from_secs(5);
const PROGRESS_INTERVAL: Duration = Duration::from_millis(200);

/// --access and --timeout, set once in `run`. A static rather than an
/// argument because every command opens devices, some from worker threads.
static IO: RwLock<(Access, Duration)> = RwLock::new((Access::Os, Duration::from_secs(10)));

#[derive(Parser)]
#[command(name = "sf-cli", about = "SectorForge command-line imaging engine", version)]
struct Cli {
    /// Emit newline-delimited JSON events on stdout (see docs/cli-contract.md).
    #[arg(long, global = true)]
    json: bool,

    /// How physical drives are read: os (through the OS disk driver), scsi
    /// (SCSI READ pass-through) or ata (ATA READ DMA EXT pass-through). The
    /// pass-through modes skip the driver's own retries. Ignored for image files.
    #[arg(long, global = true, default_value = "os", value_parser = parse_access)]
    access: Access,

    /// Seconds before a single pass-through read gives up (scsi/ata only).
    #[arg(long, global = true, default_value_t = 10, value_parser = clap::value_parser!(u64).range(1..=3600))]
    timeout: u64,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Clone a device sector-by-sector into an image file, retrying and
    /// isolating bad sectors instead of giving up on the whole run.
    Image {
        /// Source: \\.\PhysicalDrive1 (Windows) or /dev/sdb (Linux) for
        /// direct hardware access, or any file path for image-file mode.
        #[arg(long)]
        source: String,
        /// Destination image file.
        #[arg(long)]
        dest: PathBuf,
        /// Mapfile tracking progress — created if missing, reused to resume.
        #[arg(long)]
        map: PathBuf,
        /// Read size (bytes) on the fast first pass.
        #[arg(long, default_value_t = 1024 * 1024)]
        chunk_size: u64,
        /// How far to jump past a chunk that failed to read.
        #[arg(long, default_value_t = 1024 * 1024)]
        skip_size: u64,
        /// Attempts per sector before giving up on it as bad.
        #[arg(long, default_value_t = 3)]
        retries: u32,
        /// Skip the fast first pass; only retry known-damaged regions.
        #[arg(long)]
        retry_only: bool,
        /// Run the fast first pass from the end of the drive backwards.
        #[arg(long)]
        reverse: bool,
    },
    /// Print a summary of a mapfile's current state.
    Status {
        #[arg(long)]
        map: PathBuf,
    },
    /// Detect partitions and filesystems on a device or image.
    Scan {
        #[arg(long)]
        source: String,
        /// Also scan the whole device for filesystem signatures, to find
        /// partitions missing from (or deleted from) the partition table.
        #[arg(long)]
        deep: bool,
    },
    /// Quick partition-table scan of several drives or images in parallel.
    Survey {
        /// Repeat for each source.
        #[arg(long, required = true)]
        source: Vec<String>,
    },
    /// SHA-256 of a whole device or image, to check a copy matches its source.
    Hash {
        #[arg(long)]
        source: String,
    },
    /// Print the version and the JSON contract version.
    Version,
    /// List physical drives.
    Devices,
    /// Recover files by their header signatures (no filesystem needed).
    Carve {
        /// Usually the cloned image; a raw device path also works.
        #[arg(long, required_unless_present = "list_types")]
        source: Option<String>,
        /// Folder to write recovered files into (created if missing).
        #[arg(long, required_unless_present = "list_types")]
        out: Option<PathBuf>,
        /// Comma-separated type ids (see --list-types). Default: all.
        #[arg(long, value_delimiter = ',')]
        types: Vec<String>,
        /// First byte to scan.
        #[arg(long)]
        start: Option<u64>,
        /// Stop scanning at this byte (exclusive).
        #[arg(long)]
        end: Option<u64>,
        /// Imaging mapfile for this source: unrescued ranges count as unreadable.
        #[arg(long)]
        map: Option<PathBuf>,
        /// List the supported file types and exit.
        #[arg(long)]
        list_types: bool,
    },
    /// List files and folders (including deleted ones) on an NTFS, FAT, exFAT or ext partition.
    Files {
        #[arg(long)]
        source: String,
        /// Partition start, in 512-byte sectors (`start_lba` from `scan`).
        #[arg(long)]
        partition_lba: u64,
        #[arg(long)]
        deleted_only: bool,
        #[arg(long)]
        map: Option<PathBuf>,
    },
    /// Copy files out of a partition by id (from `files`).
    Extract {
        #[arg(long)]
        source: String,
        #[arg(long)]
        partition_lba: u64,
        /// Comma-separated ids; a folder id extracts everything under it.
        #[arg(long, value_delimiter = ',', required_unless_present = "ids_file", conflicts_with = "ids_file")]
        ids: Vec<u64>,
        /// File with one id per line (for large selections).
        #[arg(long)]
        ids_file: Option<PathBuf>,
        /// Folder to recreate the selected files' folder structure in.
        #[arg(long)]
        out: PathBuf,
        #[arg(long)]
        map: Option<PathBuf>,
    },
}

/// Where output goes: human text, or one JSON object per stdout line.
#[derive(Clone, Copy)]
struct Out {
    json: bool,
}

impl Out {
    fn event(self, value: Value) {
        if self.json {
            let mut stdout = io::stdout().lock();
            let _ = writeln!(stdout, "{value}");
            let _ = stdout.flush();
        }
    }

    /// What the CLI is doing right now, for steps with no progress numbers.
    fn phase(self, text: impl AsRef<str>) {
        if self.json {
            self.event(json!({"event": "phase", "message": text.as_ref()}));
        } else {
            eprintln!("{}", text.as_ref());
        }
    }

    /// Human-readable line; goes to stderr in JSON mode so stdout stays
    /// pure NDJSON.
    fn say(self, text: impl AsRef<str>) {
        if self.json {
            eprintln!("{}", text.as_ref());
        } else {
            println!("{}", text.as_ref());
        }
    }
}

struct Throttle {
    last: Option<Instant>,
}

impl Throttle {
    fn new() -> Self {
        Self { last: None }
    }

    fn ready(&mut self) -> bool {
        if self.last.is_some_and(|t| t.elapsed() < PROGRESS_INTERVAL) {
            return false;
        }
        self.last = Some(Instant::now());
        true
    }
}

/// `\\.\…` (Windows) or `/dev/…` (Linux) means direct hardware access;
/// anything else is opened as a plain file (image-file mode).
fn is_raw_device_path(path: &str) -> bool {
    path.starts_with(r"\\.\") || path.starts_with("/dev/")
}

fn open_device(path: &str) -> anyhow::Result<Box<dyn BlockDevice>> {
    open_device_inner(path).map_err(|e| {
        if e.kind() == io::ErrorKind::PermissionDenied && is_raw_device_path(path) {
            anyhow::anyhow!(
                "access denied opening {path}: direct hardware access needs Administrator (Windows) or root (Linux)"
            )
        } else {
            anyhow::anyhow!("cannot open source {path}: {e}")
        }
    })
}

fn parse_access(s: &str) -> Result<Access, String> {
    match s.to_ascii_lowercase().as_str() {
        "os" => Ok(Access::Os),
        "scsi" => Ok(Access::Scsi),
        "ata" => Ok(Access::Ata),
        _ => Err(format!("unknown access mode '{s}': use os, scsi or ata")),
    }
}

fn access_name(a: Access) -> &'static str {
    match a {
        Access::Os => "os",
        Access::Scsi => "scsi",
        Access::Ata => "ata",
    }
}

fn open_device_inner(path: &str) -> io::Result<Box<dyn BlockDevice>> {
    if is_raw_device_path(path) {
        let (access, timeout) = *IO.read().unwrap_or_else(|e| e.into_inner());
        #[cfg(windows)]
        return Ok(Box::new(sf_core::device::WindowsPhysicalDrive::open_with(path, access, timeout)?));
        #[cfg(target_os = "linux")]
        {
            // ponytail: Linux has no pass-through yet; SG_IO (SCSI READ / ATA PASS-THROUGH(16)) would add it.
            let _ = timeout;
            if access != Access::Os {
                return Err(io::Error::new(io::ErrorKind::Unsupported, "SCSI/ATA pass-through isn't available on Linux yet; use --access os"));
            }
            return Ok(Box::new(sf_core::device::LinuxBlockDevice::open(path)?));
        }
        #[cfg(not(any(windows, target_os = "linux")))]
        return Err(io::Error::new(io::ErrorKind::Unsupported, "no raw device backend for this platform yet"));
    }
    Ok(Box::new(sf_core::device::FileBlockDevice::open(path, 512)?))
}

fn mode_name(path: &str) -> &'static str {
    if is_raw_device_path(path) {
        "device"
    } else {
        "file"
    }
}

fn status_name(s: Status) -> &'static str {
    match s {
        Status::NonTried => "non_tried",
        Status::NonTrimmed => "non_trimmed",
        Status::NonScraped => "non_scraped",
        Status::BadSector => "bad_sector",
        Status::Finished => "finished",
    }
}

fn fs_name(f: FsKind) -> &'static str {
    match f {
        FsKind::Ntfs => "ntfs",
        FsKind::Fat12 => "fat12",
        FsKind::Fat16 => "fat16",
        FsKind::Fat32 => "fat32",
        FsKind::ExFat => "exfat",
        FsKind::Ext => "ext",
        FsKind::Unknown => "unknown",
    }
}

fn confidence_name(c: Confidence) -> &'static str {
    match c {
        Confidence::Confident => "confident",
        Confidence::Rebuilt => "rebuilt",
        Confidence::LowConfidence => "low_confidence",
    }
}

fn source_name(s: PartitionSource) -> &'static str {
    match s {
        PartitionSource::Mbr => "mbr",
        PartitionSource::Gpt => "gpt",
        PartitionSource::SignatureScan => "signature_scan",
    }
}

fn table_name(t: TableKind) -> &'static str {
    match t {
        TableKind::Mbr => "mbr",
        TableKind::Gpt => "gpt",
        TableKind::None => "none",
    }
}

fn totals_json(m: &MapFile) -> Value {
    json!({
        "finished": m.bytes_with_status(Status::Finished),
        "bad_sector": m.bytes_with_status(Status::BadSector),
        "non_trimmed": m.bytes_with_status(Status::NonTrimmed),
        "non_scraped": m.bytes_with_status(Status::NonScraped),
        "non_tried": m.bytes_with_status(Status::NonTried),
    })
}

fn format_bytes(b: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = b as f64;
    let mut u = 0;
    while v >= 1024.0 && u < UNITS.len() - 1 {
        v /= 1024.0;
        u += 1;
    }
    format!("{v:.2} {}", UNITS[u])
}

fn report_progress(out: Out, pass: u32, p: Progress) {
    if out.json {
        out.event(json!({
            "event": "progress",
            "pass": pass,
            "current_pos": p.current_pos,
            "bytes_total": p.bytes_total,
            "bytes_rescued": p.bytes_rescued,
            "bytes_bad": p.bytes_bad,
            "bytes_pending": p.bytes_pending,
        }));
    } else {
        let pct = if p.bytes_total > 0 {
            100.0 * (p.bytes_rescued + p.bytes_bad) as f64 / p.bytes_total as f64
        } else {
            0.0
        };
        print!(
            "\r{pct:>6.2}%  rescued {:<12} bad {:<10} pending {:<12} pos 0x{:X}    ",
            format_bytes(p.bytes_rescued),
            format_bytes(p.bytes_bad),
            format_bytes(p.bytes_pending),
            p.current_pos
        );
        let _ = io::stdout().flush();
    }
}

fn print_status_human(m: &MapFile) {
    println!("Map size: {}", format_bytes(m.size()));
    for s in [Status::Finished, Status::BadSector, Status::NonTrimmed, Status::NonScraped, Status::NonTried] {
        println!("  {:<12} {}", status_name(s), format_bytes(m.bytes_with_status(s)));
    }
}

struct ImageArgs {
    source: String,
    dest: PathBuf,
    map: PathBuf,
    chunk_size: u64,
    skip_size: u64,
    retries: u32,
    retry_only: bool,
    reverse: bool,
}

fn run_image(out: Out, a: ImageArgs) -> anyhow::Result<()> {
    recover::guard_outputs(&a.source, &[("--dest", &a.dest), ("--map", &a.map)])?;
    let device = open_device(&a.source)?;
    let size = device.size_bytes();
    let sector_size = device.sector_size();

    let resumed = a.map.exists();
    let map = if resumed {
        let m = MapFile::load(&a.map)?;
        anyhow::ensure!(
            m.size() == size,
            "mapfile {} is for a {}-byte device, but the source is {} bytes — wrong map for this source?",
            a.map.display(),
            m.size(),
            size
        );
        // The map is only meaningful alongside the image it describes: regions
        // it calls finished are never re-read, so a missing or different
        // destination would silently end up with holes.
        let already = m.bytes_with_status(Status::Finished);
        if already > 0 {
            let dest_len = std::fs::metadata(&a.dest).map(|d| d.len()).ok();
            anyhow::ensure!(
                dest_len == Some(size),
                "the map says {} was already copied, but the destination {} {}. Resuming would leave those parts of the image empty. \
                 Use the destination file from the original run, or delete the map to start over.",
                format_bytes(already),
                a.dest.display(),
                if dest_len.is_none() { "doesn't exist" } else { "is a different size" }
            );
        }
        out.say(format!("Resuming from existing map {}", a.map.display()));
        m
    } else {
        out.say(format!("Starting new map for {} ({})", a.source, format_bytes(size)));
        let m = MapFile::new_non_tried(size);
        m.save(&a.map)?;
        m
    };

    out.event(json!({
        "event": "start",
        "source": a.source,
        "mode": mode_name(&a.source),
        "access": if is_raw_device_path(&a.source) { access_name(IO.read().unwrap_or_else(|e| e.into_inner()).0) } else { "file" },
        "size": size,
        "sector_size": sector_size,
        "resumed": resumed,
    }));

    out.phase(format!("Setting aside {} for the destination image", format_bytes(size)));
    let dest_file = OpenOptions::new().write(true).create(true).truncate(false).open(&a.dest)?;
    if a.reverse {
        // Without this, the first write (at the end) waits while Windows zero-fills the whole image.
        // Best effort: FAT/exFAT can't do sparse files, and then the run is only slow to start.
        let _ = sf_core::device::make_sparse(&dest_file);
    }
    dest_file.set_len(size)?;

    let config = ImagerConfig {
        initial_chunk: a.chunk_size,
        skip_size: a.skip_size,
        max_retries: a.retries,
        reverse: a.reverse,
    };
    let mut imager = Imager::new(device, dest_file, map, config).with_checkpoint(&a.map, CHECKPOINT_INTERVAL);

    let passes: &[(u32, &str)] = if a.retry_only { &[(2, "retry")] } else { &[(1, "copy"), (2, "retry")] };
    for &(pass, name) in passes {
        out.event(json!({"event": "pass_start", "pass": pass, "name": name}));
        if !out.json {
            println!("\nPass {pass}: {}", if pass == 1 { "fast copy, skipping past damage" } else { "retrying damaged regions sector-by-sector" });
        }
        let mut throttle = Throttle::new();
        let on_progress = |p: Progress| {
            if throttle.ready() {
                report_progress(out, pass, p);
            }
        };
        if pass == 1 {
            imager.run_copy_pass(on_progress)?;
        } else {
            imager.run_retry_pass(on_progress)?;
        }
        report_progress(out, pass, imager.progress());
        out.event(json!({"event": "pass_complete", "pass": pass}));
        if !out.json {
            println!();
        }
    }

    out.event(json!({"event": "done", "totals": totals_json(imager.map())}));
    if !out.json {
        print_status_human(imager.map());
    }
    Ok(())
}

fn partitions_json(result: &partitions::ScanResult) -> Vec<Value> {
    result
        .partitions
        .iter()
        .map(|p| {
            json!({
                "start_lba": p.start_bytes / 512,
                "size_sectors": p.size_bytes.map(|s| s / 512),
                "filesystem": fs_name(p.filesystem),
                "confidence": confidence_name(p.confidence),
                "source": source_name(p.source),
                "label": p.label,
            })
        })
        .collect()
}

/// Quick (partition table only) scan of several sources at once, one thread each, so a
/// slow or failing drive delays only its own result. Never fails as a whole.
fn run_survey(out: Out, sources: &[String]) -> anyhow::Result<()> {
    std::thread::scope(|s| {
        for source in sources {
            s.spawn(move || {
                let scanned = open_device(source).and_then(|mut dev| {
                    let size = dev.size_bytes();
                    Ok((size, partitions::scan(&mut dev, false, |_, _| {})?))
                });
                match scanned {
                    Ok((size, r)) if out.json => out.event(json!({
                        "event": "survey_result", "source": source, "size": size,
                        "table_kind": table_name(r.table_kind), "partitions": partitions_json(&r),
                    })),
                    Ok((size, r)) => println!("{source}: {} bytes, {} table, {} partitions", size, table_name(r.table_kind), r.partitions.len()),
                    Err(e) if out.json => out.event(json!({"event": "survey_error", "source": source, "message": format!("{e:#}")})),
                    Err(e) => eprintln!("{source}: {e:#}"),
                }
            });
        }
    });
    out.event(json!({"event": "done"}));
    Ok(())
}

fn run_hash(out: Out, source: &str) -> anyhow::Result<()> {
    use sha2::{Digest, Sha256};
    const CHUNK: u64 = 4 << 20;

    let mut device = open_device(source)?;
    let size = device.size_bytes();
    out.event(json!({
        "event": "start",
        "source": source,
        "mode": mode_name(source),
        "size": size,
        "sector_size": device.sector_size(),
    }));
    out.phase(format!("Reading all {} of {source} to compute its SHA-256", format_bytes(size)));

    let mut buf = sf_core::aligned_buf::AlignedBuf::new(CHUNK as usize, device.sector_size().max(4096) as usize);
    let mut hasher = Sha256::new();
    let mut throttle = Throttle::new();
    let mut pos = 0u64;
    while pos < size {
        let n = CHUNK.min(size - pos) as usize;
        device.read_at(pos, &mut buf[..n]).map_err(|e| {
            anyhow::anyhow!("can't read {source} at byte {pos}: {e}. A drive with unreadable sectors has no single hash; image it first and hash the image")
        })?;
        hasher.update(&buf[..n]);
        pos += n as u64;
        if pos == size || throttle.ready() {
            if out.json {
                out.event(json!({"event": "hash_progress", "bytes_done": pos, "bytes_total": size}));
            } else {
                eprint!("
Hashed {:>6.2}%", 100.0 * pos as f64 / size.max(1) as f64);
            }
        }
    }
    let hex = format!("{:x}", hasher.finalize());
    out.event(json!({"event": "done", "sha256": hex}));
    if !out.json {
        eprintln!();
        println!("{hex}  {source}");
    }
    Ok(())
}

fn run_status(out: Out, map_path: &PathBuf) -> anyhow::Result<()> {
    let m = MapFile::load(map_path)?;
    if out.json {
        let blocks: Vec<Value> = m
            .blocks()
            .iter()
            .map(|b| json!({"pos": b.pos, "size": b.size, "status": status_name(b.status)}))
            .collect();
        out.event(json!({
            "event": "status",
            "size": m.size(),
            "current_pos": m.current_pos,
            "totals": totals_json(&m),
            "blocks": blocks,
        }));
    } else {
        print_status_human(&m);
    }
    Ok(())
}

fn run_scan(out: Out, source: &str, deep: bool) -> anyhow::Result<()> {
    let mut device = open_device(source)?;
    let size = device.size_bytes();
    out.event(json!({
        "event": "start",
        "source": source,
        "mode": mode_name(source),
        "size": size,
        "sector_size": device.sector_size(),
    }));

    out.phase(if deep {
        format!("Reading the partition table of {source}, then searching every sector for filesystems")
    } else {
        format!("Reading the partition table of {source}")
    });
    let mut throttle = Throttle::new();
    let result = partitions::scan(&mut device, deep, |done, total| {
        if done == total || throttle.ready() {
            if out.json {
                out.event(json!({"event": "scan_progress", "bytes_scanned": done, "bytes_total": total}));
            } else {
                eprint!("\rSignature scan: {:>6.2}%", 100.0 * done as f64 / total.max(1) as f64);
            }
        }
    })?;

    if out.json {
        out.event(json!({"event": "result", "table_kind": table_name(result.table_kind), "partitions": partitions_json(&result)}));
    } else {
        eprintln!();
        println!("Partition table: {}", table_name(result.table_kind));
        if result.partitions.is_empty() {
            println!("No partitions or filesystems found.");
        }
        for (i, p) in result.partitions.iter().enumerate() {
            println!(
                "  #{i}  {:<8} start LBA {:<12} size {:<12} {:<15} via {}{}",
                fs_name(p.filesystem),
                p.start_bytes / 512,
                p.size_bytes.map(format_bytes).unwrap_or_else(|| "unknown".into()),
                confidence_name(p.confidence),
                source_name(p.source),
                p.label.as_ref().map(|l| format!("  \"{l}\"")).unwrap_or_default(),
            );
        }
    }
    Ok(())
}

/// Runs sf-cli with `args` (the first item is the program name, as in `std::env::args_os`).
/// Used by the `sf-cli` binary and by the Windows GUI, which embeds the engine and
/// runs itself in CLI mode so it can ship as a single exe.
pub fn run<I, T>(args: I) -> ExitCode
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let cli = match Cli::try_parse_from(args) {
        Ok(cli) => cli,
        Err(e) => {
            // Usage errors print to stderr and exit 2; --help / --version print and exit 0.
            let _ = e.print();
            return ExitCode::from(e.exit_code().clamp(0, 255) as u8);
        }
    };
    let out = Out { json: cli.json };
    *IO.write().unwrap_or_else(|e| e.into_inner()) = (cli.access, Duration::from_secs(cli.timeout));
    let result = match cli.command {
        Command::Image {
            source,
            dest,
            map,
            chunk_size,
            skip_size,
            retries,
            retry_only,
            reverse,
        } => run_image(
            out,
            ImageArgs {
                source,
                dest,
                map,
                chunk_size,
                skip_size,
                retries,
                retry_only,
                reverse,
            },
        ),
        Command::Status { map } => run_status(out, &map),
        Command::Scan { source, deep } => run_scan(out, &source, deep),
        Command::Hash { source } => run_hash(out, &source),
        Command::Survey { source } => run_survey(out, &source),
        Command::Version => {
            let v = env!("CARGO_PKG_VERSION");
            out.event(json!({"event": "version", "version": v, "contract": CONTRACT_VERSION}));
            if !out.json {
                println!("sf-cli {v} (JSON contract v{CONTRACT_VERSION})");
            }
            Ok(())
        }
        Command::Devices => recover::run_devices(out),
        Command::Carve { list_types: true, .. } => {
            recover::run_list_types(out);
            Ok(())
        }
        Command::Carve { source, out: out_dir, types, start, end, map, .. } => recover::run_carve(
            out,
            recover::CarveArgs {
                source: source.expect("clap requires --source"),
                out_dir: out_dir.expect("clap requires --out"),
                types,
                start,
                end,
                map,
            },
        ),
        Command::Files { source, partition_lba, deleted_only, map } => {
            recover::run_files(out, &source, partition_lba, deleted_only, map.as_ref())
        }
        Command::Extract { source, partition_lba, ids, ids_file, out: out_dir, map } => recover::run_extract(
            out,
            recover::ExtractArgs { source, partition_lba, ids, ids_file, out_dir, map },
        ),
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            if out.json {
                out.event(json!({"event": "error", "message": format!("{e:#}")}));
            } else {
                eprintln!("\nerror: {e:#}");
            }
            ExitCode::from(1)
        }
    }
}
