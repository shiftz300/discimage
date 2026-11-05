use std::path::{Path, PathBuf};

use anyhow::Result;
use clap::{Parser, Subcommand};
use discimage::{
    author, burn, device,
    layout::Plan,
    preview,
    project::{Artwork, Project},
    template,
};

#[derive(Parser)]
#[command(
    version,
    about = "Music-first enhanced CD authoring, previews and macOS recording"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Prepare and record a music-first disc on a blank CD-R (macOS).
    Burn {
        project: PathBuf,
        #[arg(long)]
        device: String,
        #[arg(short, long)]
        output: PathBuf,
        /// Requested CD speed multiplier.
        #[arg(long, default_value_t = 8)]
        speed: u32,
        /// Probe and prepare the complete job without reserving or writing the disc.
        #[arg(long)]
        dry_run: bool,
    },
    /// Continue a prepared job or append after a confirmed music write.
    Resume {
        directory: PathBuf,
        #[arg(long)]
        device: String,
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
    /// Verify and render the actual encoded drawing payload from a bundle.
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
}

fn main() -> std::process::ExitCode {
    match run(Cli::parse()) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error:#}");
            std::process::ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<()> {
    match cli.command {
        Commands::Burn {
            project,
            device: id,
            output,
            speed,
            dry_run,
        } => {
            let mut recorder = device::recorder(&id)?;
            device::install_cancel_handler()?;
            let result = burn::start(&mut *recorder, &project, &output, speed, dry_run)?;
            if dry_run {
                println!(
                    "Prepared {} without writing media. Review planned/ and journal.json before resume.",
                    output.display()
                );
            } else {
                println!(
                    "Disc finalized and verified. Measured session bundle: {}",
                    result.actual(&output).display()
                );
            }
        }
        Commands::Resume {
            directory,
            device: id,
        } => {
            let mut recorder = device::recorder(&id)?;
            device::install_cancel_handler()?;
            let result = burn::resume(&mut *recorder, &directory)?;
            println!(
                "Disc finalized and verified. Measured session bundle: {}",
                result.actual(&directory).display()
            );
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
                "Saved {} — sampled payload readback, not an optical simulation.",
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
                        println!("  CD-R: {} · SAO: {} · TAO: {}", c.cdr, c.sao, c.tao);
                    }
                    if let Some(m) = drive.media {
                        println!(
                            "  Media: {} · blank: {} · appendable: {} · free sectors: {:?} · NWA: {:?}",
                            m.kind, m.blank, m.appendable, m.free_sectors, m.nwa
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
                println!("Uncalibrated estimate. This command does not access the optical drive.");
            }
        }
    }
    Ok(())
}

fn prepare(path: &Path, session_start: Option<u32>) -> Result<(Plan, Option<Artwork>)> {
    let project = Project::load(path)?;
    Ok((
        Plan::from_project(&project, session_start)?,
        project.artwork,
    ))
}
