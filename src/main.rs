use std::path::{Path, PathBuf};

use anyhow::{Context, Result, ensure};
use clap::{Args, Parser, Subcommand};
use discimage::{
    author, burn, calibrate, console, device,
    layout::Plan,
    preview,
    project::{Artwork, Project},
    template, timing,
};

mod guide;

#[derive(Parser)]
#[command(
    version,
    about = "Music-first enhanced CD authoring, previews and macOS recording",
    after_help = "Offline: plan -> template / preview -> build -> verify\nRecord on macOS: devices -> prepare -> resume"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Prepare a recording job from a blank CD without reserving or writing it (macOS).
    Prepare(Recording),
    /// Prepare and immediately write a blank CD (macOS).
    Burn {
        #[command(flatten)]
        recording: Recording,
        /// Reserve a blank CD-R of this exact capacity, selecting the drive automatically.
        #[arg(long, conflicts_with = "dry_run", value_parser = clap::value_parser!(u32).range(1..=360_000))]
        blank_cdr: Option<u32>,
        /// Prepare only; equivalent to the prepare command.
        #[arg(long)]
        dry_run: bool,
    },
    /// Write or continue a job using its saved drive and speed (macOS).
    Resume {
        directory: PathBuf,
        /// Assert the drive ID; it must match the one saved in the job.
        #[arg(long)]
        device: Option<String>,
    },
    /// Export disc/strip artwork templates from the album's actual WAV lengths.
    Template {
        project: PathBuf,
        #[arg(short, long)]
        output: PathBuf,
        /// Disc canvas size; strip dimensions match its physical pixel density.
        #[arg(long, default_value_t = 1600)]
        size: u32,
        #[arg(long)]
        session_start: Option<u32>,
    },
    /// Author audio, a session-offset ISO/RAW, previews and a checksummed manifest.
    Build {
        project: PathBuf,
        #[arg(short, long)]
        output: PathBuf,
        /// Override session 2 INDEX 01 with an observed next writable LBA.
        #[arg(long)]
        session_start: Option<u32>,
    },
    /// Check asset/file hashes and every Mode 1 header, EDC and ECC byte.
    Verify { directory: PathBuf },
    /// Verify and project the encoded drawing payload using the authoring geometry.
    Readback {
        directory: PathBuf,
        #[arg(short, long)]
        output: PathBuf,
        #[arg(long, default_value_t = 1024)]
        size: u32,
        #[arg(long, default_value_t = 32)]
        samples: u32,
    },
    /// Inspect actual PCM WAV lengths and estimate the free artwork annulus.
    Plan {
        project: PathBuf,
        #[arg(long)]
        session_start: Option<u32>,
        /// Print a machine-readable estimate, including burn_ready=false.
        #[arg(long)]
        json: bool,
        /// Save a labeled SVG layout and artwork report; never overwrite.
        #[arg(long)]
        svg: Option<PathBuf>,
    },
    /// Export a PNG disc composition or self-contained SVG preview report.
    Preview {
        project: PathBuf,
        #[arg(long)]
        session_start: Option<u32>,
        #[arg(short, long)]
        output: PathBuf,
        /// Resolution of the raster disc preview (256–4096).
        #[arg(long, default_value_t = 1024)]
        size: u32,
    },
    /// Query the OS for optical drives (read-only, platform-specific output).
    Devices {
        #[arg(long)]
        json: bool,
    },
    /// Create a CD calibration target, mark a photo, or fit radial geometry.
    #[command(subcommand)]
    Calibrate(Calibration),
}

#[derive(Args)]
struct Recording {
    project: PathBuf,
    /// Exact ID from devices; prepare can select the only eligible drive.
    #[arg(long, value_name = "DRIVE_ID")]
    device: Option<String>,
    /// New directory for the prepared bundle and recording journal.
    #[arg(short, long)]
    output: PathBuf,
    /// Requested CD speed multiplier, saved with the job.
    #[arg(long, default_value_t = 8, value_parser = clap::value_parser!(u32).range(1..=64))]
    speed: u32,
}

#[derive(Subcommand)]
enum Calibration {
    /// Record a finalized single-session Mode 1 calibration target on a blank CD-R.
    Record {
        #[arg(short, long)]
        output: PathBuf,
        /// Select a drive when more than one blank CD-R is eligible.
        #[arg(long)]
        device: Option<String>,
        /// Requested write speed. By default, let the drive choose its supported maximum.
        #[arg(long, value_parser = clap::value_parser!(u32).range(1..=64))]
        speed: Option<u32>,
        /// Verify selection and reservation without creating files or writing the disc.
        #[arg(long)]
        dry_run: bool,
    },
    /// Run an external discprobe-macos checkout with live read-only progress.
    Probe {
        #[arg(long)]
        probe_dir: PathBuf,
        #[arg(long, default_value = "python3")]
        python: PathBuf,
        #[arg(long)]
        device: String,
        /// Absolute LBA interval, end exclusive; use a recorded Mode 1 data track.
        #[arg(long)]
        first: u32,
        #[arg(long)]
        end: u32,
        #[arg(short, long)]
        output: PathBuf,
    },
    /// Import external CD timing and a photo phase report into a separate candidate.
    Import {
        project: PathBuf,
        #[arg(long)]
        timing: PathBuf,
        #[arg(long)]
        phase: PathBuf,
        #[arg(long)]
        bundle: PathBuf,
        #[arg(short, long)]
        output: PathBuf,
        #[arg(long, default_value_t = 1024)]
        size: u32,
    },
    /// Walk through target recording, photo measurements and fitting in one directory.
    Guide(guide::Options),
    /// Generate a standalone three-band calibration project with a silent music track.
    Target {
        #[arg(short, long)]
        output: PathBuf,
        #[arg(long, default_value_t = 360_000)]
        capacity_sectors: u32,
    },
    /// Export a local photo point picker for the bundle that was actually written.
    Mark {
        bundle: PathBuf,
        photo: PathBuf,
        #[arg(short, long)]
        output: PathBuf,
    },
    /// Fit a [disc.calibration] TOML section using the six marked band edges.
    Fit {
        bundle: PathBuf,
        points: PathBuf,
        #[arg(short, long)]
        output: PathBuf,
    },
}

fn main() -> std::process::ExitCode {
    match run(Cli::parse()) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            console::error(format_args!("{error:#}"));
            std::process::ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<()> {
    match cli.command {
        Commands::Calibrate(command) => match command {
            Calibration::Record {
                output,
                device,
                speed,
                dry_run,
            } => {
                calibrate::record_single(&output, device.as_deref(), speed, dry_run)?;
            }
            Calibration::Probe {
                probe_dir,
                python,
                device,
                first,
                end,
                output,
            } => {
                timing::probe(&probe_dir, &python, &device, first, end, &output)?;
            }
            Calibration::Import {
                project,
                timing: summary,
                phase,
                bundle,
                output,
                size,
            } => {
                timing::import(&project, &summary, &phase, &bundle, &output, size)?;
                println!(
                    "Open {}. Separate candidate: {}. No disc accessed; physical validation pending.",
                    output.join("index.html").display(),
                    output.join("candidate.toml").display()
                );
            }
            Calibration::Guide(options) => guide::run(options)?,
            Calibration::Target {
                output,
                capacity_sectors,
            } => {
                calibrate::create(&output, capacity_sectors)?;
                println!("Created {}", output.join("target.toml").display());
            }
            Calibration::Mark {
                bundle,
                photo,
                output,
            } => {
                calibrate::mark(&bundle, &photo, &output)?;
                println!("Open {} in a browser to mark the photo.", output.display());
            }
            Calibration::Fit {
                bundle,
                points,
                output,
            } => {
                let profile = calibrate::fit(&bundle, &points, &output)?;
                println!(
                    "Saved {} · radii {:.4}–{:.4} mm at {} sectors · RMS {:.3} mm",
                    output.display(),
                    profile.inner_radius_mm,
                    profile.outer_radius_mm,
                    profile.reference_sectors,
                    profile.rms_mm
                );
            }
        },
        Commands::Prepare(recording) => record(recording, None, true)?,
        Commands::Burn {
            recording,
            blank_cdr,
            dry_run,
        } => record(recording, blank_cdr, dry_run)?,
        Commands::Resume {
            directory,
            device: id,
        } => {
            let saved = burn::Journal::load(&directory).with_context(|| {
                format!("cannot read {}", directory.join("journal.json").display())
            })?;
            ensure!(
                id.as_deref().is_none_or(|id| id == saved.device),
                "--device does not match the recording job's saved drive: {}",
                saved.device
            );
            let mut recorder = device::recorder(&saved.device)?;
            device::install_cancel_handler()?;
            console::phase(
                "resume",
                format_args!("{} | requested {}x", directory.display(), saved.speed),
            );
            let result = burn::resume(&mut *recorder, &directory)?;
            recording_complete(&result, &directory);
        }
        Commands::Template {
            project,
            output,
            size,
            session_start,
        } => {
            let (plan, _) = prepare(&project, session_start)?;
            template::save(&plan, &output, size)?;
            println!(
                "Saved disc and strip templates in {}. Estimated artwork radius {:.2}–{:.2} mm.",
                output.display(),
                plan.artwork_inner_radius_mm,
                plan.artwork_outer_radius_mm
            );
        }
        Commands::Build {
            project,
            output,
            session_start,
        } => {
            let manifest = author::build(&project, &output, session_start)?;
            println!(
                "Built {}: session 2 starts at LBA {}, {} data sectors. Offline bundle only.",
                output.display(),
                manifest.session_start,
                manifest.data_sectors
            );
        }
        Commands::Verify { directory } => {
            let manifest = author::verify(&directory)?;
            println!(
                "Verified {} assets, {} ISO files, {} Mode 1 sectors.",
                manifest.assets.len(),
                manifest.files.len(),
                manifest.data_sectors
            );
        }
        Commands::Readback {
            directory,
            output,
            size,
            samples,
        } => {
            let manifest = author::verify(&directory)?;
            preview::readback(&directory, &manifest, &output, size, samples)?;
            println!(
                "Saved {} — payload projected with the authoring model. Verify the physical artwork by inspecting the disc.",
                output.display()
            );
        }
        Commands::Devices { json } => {
            let devices = device::list()?;
            if json {
                println!("{}", serde_json::to_string_pretty(&devices)?);
            } else if devices.is_empty() {
                println!("No optical drives reported.");
            } else {
                for drive in devices {
                    println!("{} [{}]\n  id: {}", drive.name, drive.backend, drive.id);
                    if let Some(c) = drive.capabilities {
                        println!(
                            "  CD-R: {} · CD-RW: {} · SAO: {} · TAO: {} · CD-Text: {}",
                            c.cdr, c.cdrw, c.sao, c.tao, c.cd_text
                        );
                    }
                    if let Some(m) = drive.media {
                        let state = if m.blank {
                            "blank"
                        } else if m.appendable {
                            "appendable"
                        } else {
                            "closed"
                        };
                        println!(
                            "  Media: {} ({state}) | {} sessions, {} tracks",
                            m.kind, m.sessions, m.tracks
                        );
                        println!(
                            "  Writable sectors: {} | Next writable LBA: {}",
                            m.free_sectors.map_or_else(|| "-".into(), |n| n.to_string()),
                            m.nwa.map_or_else(|| "-".into(), |n| n.to_string())
                        );
                    } else if drive.backend == "macos-discrecording" {
                        println!("  No ready media");
                    }
                }
            }
        }
        Commands::Preview {
            project,
            session_start,
            output,
            size,
        } => {
            let (plan, artwork) = prepare(&project, session_start)?;
            preview::save(&plan, artwork.as_ref(), &output, size)?;
            println!(
                "Saved {} — design preview only, not a burn image.",
                output.display()
            );
        }
        Commands::Plan {
            project,
            session_start,
            json,
            svg: svg_path,
        } => {
            let (plan, artwork) = prepare(&project, session_start)?;
            if let Some(path) = svg_path {
                preview::save(&plan, artwork.as_ref(), &path, 1024)?;
            }
            if json {
                println!("{}", serde_json::to_string_pretty(&plan)?);
            } else {
                println!("{}", plan.title);
                println!(
                    "Music: {} tracks, {:.3} seconds (sector-aligned)",
                    plan.tracks.len(),
                    f64::from(plan.music_sectors) / 75.0
                );
                println!(
                    "Artwork budget: {} sectors, estimated radius {:.2}–{:.2} mm",
                    plan.artwork_sectors,
                    plan.artwork_inner_radius_mm,
                    plan.artwork_outer_radius_mm
                );
                println!(
                    "{}. This command does not access the optical drive.",
                    plan.geometry_label()
                );
            }
        }
    }
    Ok(())
}

fn record(recording: Recording, blank_cdr: Option<u32>, dry_run: bool) -> Result<()> {
    let Recording {
        project,
        device: id,
        output,
        speed,
    } = recording;
    device::install_cancel_handler()?;
    let mut recorder = match (blank_cdr, id.as_deref()) {
        (Some(capacity), id) => device::blank_cdr_recorder(id, capacity)?,
        (None, id) => {
            let drives;
            let id = match id {
                Some(id) => id,
                None if dry_run => {
                    ensure!(
                        cfg!(target_os = "macos"),
                        "Recording is implemented for macOS only"
                    );
                    drives = device::list()?;
                    &burn::select_blank(&drives)?.id
                }
                None => anyhow::bail!(
                    "direct recording requires --device or --blank-cdr; use 'discimage prepare' to review the job first"
                ),
            };
            device::recorder(id)?
        }
    };
    let result = burn::start(&mut *recorder, &project, &output, speed, dry_run)?;
    if dry_run {
        console::success(format_args!(
            "Prepared {} without writing the disc",
            output.display()
        ));
        println!("Preview: {}", output.join("planned/preview.svg").display());
        println!("To write the job, pass this directory to discimage resume.");
    } else {
        recording_complete(&result, &output);
    }
    Ok(())
}

fn recording_complete(journal: &burn::Journal, directory: &Path) {
    console::success(format_args!(
        "Audio and data verified; both sessions are finalized"
    ));
    println!("Recorded bundle: {}", journal.actual(directory).display());
}

fn prepare(path: &Path, session_start: Option<u32>) -> Result<(Plan, Option<Artwork>)> {
    let project = Project::load(path)?;
    Ok((
        Plan::from_project(&project, session_start)?,
        project.artwork,
    ))
}
