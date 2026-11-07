use std::path::{Path, PathBuf};

use anyhow::{Context, Result, ensure};
use clap::{Args, CommandFactory, Parser, Subcommand};
use discimage::{
    author, burn, calibrate, console, device,
    layout::Plan,
    phase, preview, probe,
    project::{Artwork, Project},
    template, timing,
};

mod guide;
mod selection;

#[derive(Parser)]
#[command(
    version,
    about = "Music-first enhanced CD authoring, previews and macOS recording",
    after_help = "Offline: plan -> template / preview -> build -> verify\nRecord on macOS: devices -> burn PROJECT --dry-run -o JOB -> burn JOB"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Print a shell completion script (no drive access).
    Completions {
        #[arg(value_enum)]
        shell: clap_complete::Shell,
    },
    /// Show, set or clear the default optical drive (no disc writes).
    Select {
        /// Hardware ID, local connection ID or unique model from devices.
        #[arg(value_name = "DRIVE", conflicts_with = "clear")]
        device: Option<String>,
        /// Clear the saved default without accessing the drive.
        #[arg(long)]
        clear: bool,
    },
    /// Prepare a recording job from a blank CD without reserving or writing it (macOS).
    #[command(hide = true)]
    Prepare(Recording),
    /// Erase a CD-RW, confirm the target and verify blank media (macOS).
    Erase {
        /// Drive ID or unique model; overrides select, otherwise use the only available CD-RW.
        #[arg(long)]
        device: Option<String>,
        /// Erase only the disc metadata instead of the complete recording area.
        #[arg(long)]
        quick: bool,
        /// Confirm destruction of all disc contents without an interactive prompt.
        #[arg(long, conflicts_with = "dry_run")]
        yes: bool,
        /// Check device selection without reserving or erasing media.
        #[arg(long)]
        dry_run: bool,
    },
    /// Record a project, or write/continue an existing job (macOS).
    Burn {
        #[command(flatten)]
        recording: Recording,
        /// Reserve a blank CD-R of this exact capacity, selecting the drive automatically.
        #[arg(long, conflicts_with = "dry_run", value_parser = clap::value_parser!(u32).range(1..=360_000))]
        blank_cdr: Option<u32>,
        /// Prepare a project and its preview without writing the disc.
        #[arg(long)]
        dry_run: bool,
    },
    /// Write or continue a job using its saved drive and speed (macOS).
    #[command(hide = true)]
    Resume {
        directory: PathBuf,
        /// Drive selector; it must resolve to the drive saved in the job.
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
    /// Verify an offline bundle; optionally export its encoded-payload preview.
    Verify {
        directory: PathBuf,
        /// Export a preview from the local ISO, not the physical disc.
        #[arg(short, long)]
        output: Option<PathBuf>,
        #[arg(long, requires = "output", default_value_t = 1024)]
        size: u32,
        #[arg(long, requires = "output", default_value_t = 32)]
        samples: u32,
    },
    /// Verify and project the encoded drawing payload using the authoring geometry.
    #[command(hide = true)]
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
        #[arg(long, hide = true)]
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
    /// Album TOML file, or existing job directory when continuing with burn.
    #[arg(value_name = "PROJECT_OR_JOB")]
    project: PathBuf,
    /// Drive ID or unique model; overrides select, otherwise use the only eligible blank drive.
    #[arg(long, value_name = "DRIVE")]
    device: Option<String>,
    /// New job directory (required for a project; omit for an existing job).
    #[arg(short, long)]
    output: Option<PathBuf>,
    /// Requested CD speed multiplier; omit for the drive-selected maximum.
    #[arg(long, value_parser = clap::value_parser!(u32).range(1..=64))]
    speed: Option<u32>,
}

#[derive(Subcommand)]
enum Calibration {
    /// Record a finalized single-session Mode 1 calibration target on a blank CD-R.
    Record {
        #[arg(short, long)]
        output: PathBuf,
        /// Drive ID or unique model; overrides the default saved by select.
        #[arg(long)]
        device: Option<String>,
        /// Requested write speed. By default, let the drive choose its supported maximum.
        #[arg(long, value_parser = clap::value_parser!(u32).range(1..=64))]
        speed: Option<u32>,
        /// Verify selection and reservation without creating files or writing the disc.
        #[arg(long)]
        dry_run: bool,
    },
    /// Measure a finalized Mode 1 CD with native read-only commands (macOS/Windows).
    Probe {
        /// Drive ID or unique model; defaults to select or the only connected drive.
        #[arg(long)]
        device: Option<String>,
        /// Check the disc and report its readable LBA range without timing reads.
        #[arg(long)]
        check: bool,
        /// First absolute LBA. With no explicit range, measure 3–85% of the track.
        #[arg(long, requires = "end", conflicts_with = "check")]
        first: Option<u32>,
        /// End of the absolute LBA range, exclusive.
        #[arg(long, requires = "first", conflicts_with = "check")]
        end: Option<u32>,
        #[arg(
            short,
            long,
            required_unless_present = "check",
            conflicts_with = "check"
        )]
        output: Option<PathBuf>,
    },
    /// Compare measured CD timing with angle samples from a photo.
    Phase {
        #[arg(long)]
        timing: PathBuf,
        #[arg(long)]
        bundle: PathBuf,
        #[arg(long)]
        points: PathBuf,
        /// Original photo or rectified array; required for candidate import.
        #[arg(long)]
        evidence: Option<PathBuf>,
        /// Independently measured forward offset in channel frames.
        #[arg(long, requires = "offset_evidence", allow_negative_numbers = true)]
        offset_frames: Option<f64>,
        #[arg(long, requires = "offset_frames")]
        offset_evidence: Option<PathBuf>,
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
    /// Export a target descriptor for the website's photo point picker.
    Mark {
        bundle: PathBuf,
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
        Commands::Completions { shell } => {
            clap_complete::generate(
                shell,
                &mut Cli::command(),
                "discimage",
                &mut std::io::stdout(),
            );
        }
        Commands::Calibrate(command) => match command {
            Calibration::Record {
                output,
                device,
                speed,
                dry_run,
            } => {
                let device = selection::preferred(device.as_deref())?;
                calibrate::record_single(&output, device.as_deref(), speed, dry_run)?;
            }
            Calibration::Probe {
                device,
                check,
                first,
                end,
                output,
            } => {
                let device = match selection::preferred(device.as_deref())? {
                    Some(device) => device,
                    None => {
                        let drives = device::list()?;
                        ensure!(
                            drives.len() == 1,
                            "expected one optical drive; run 'discimage select ID' or pass --device ID"
                        );
                        drives[0].id.clone()
                    }
                };
                if check {
                    let bounds = probe::inspect(&device)?;
                    println!(
                        "Recorded Mode 1 range: {}..{} (end exclusive)",
                        bounds.start, bounds.end
                    );
                } else {
                    let range = first.zip(end).map(|(a, b)| a..b);
                    probe::acquire(
                        &device,
                        range,
                        &output.context("missing probe output directory")?,
                    )?;
                }
            }
            Calibration::Phase {
                timing,
                bundle,
                points,
                evidence,
                offset_frames,
                offset_evidence,
                output,
            } => phase::compare(
                &timing,
                &bundle,
                &points,
                evidence.as_deref(),
                offset_frames.zip(offset_evidence.as_deref()),
                &output,
            )?,
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
                    "Candidate: {}. Import {} in the website's timing comparison. Physical validation pending.",
                    output.join("candidate.toml").display(),
                    output.display()
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
            Calibration::Mark { bundle, output } => {
                calibrate::mark(&bundle, &output)?;
                println!(
                    "Saved {} for the website's photo point picker.",
                    output.display()
                );
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
        } => {
            if recording.project.is_dir() {
                ensure!(
                    recording.output.is_none()
                        && recording.speed.is_none()
                        && blank_cdr.is_none()
                        && !dry_run,
                    "an existing job uses its saved inputs and speed; omit --output, --speed, --blank-cdr and --dry-run"
                );
                resume_job(&recording.project, recording.device.as_deref())?;
            } else {
                record(recording, blank_cdr, dry_run)?;
            }
        }
        Commands::Resume { directory, device } => resume_job(&directory, device.as_deref())?,
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
        Commands::Verify {
            directory,
            output,
            size,
            samples,
        } => {
            verify_bundle(&directory, output.as_deref(), size, samples)?;
        }
        Commands::Readback {
            directory,
            output,
            size,
            samples,
        } => {
            verify_bundle(&directory, Some(&output), size, samples)?;
        }
        Commands::Select { device, clear } => selection::run(device.as_deref(), clear)?,
        Commands::Devices { json } => {
            let devices = device::list()?;
            if json {
                let descriptions = devices
                    .iter()
                    .map(|drive| {
                        let mut value = serde_json::to_value(drive)?;
                        value["selector"] = serde_json::json!(drive.selector());
                        value["hardware_id"] = serde_json::json!(drive.hardware_id());
                        value["connection_selector"] =
                            serde_json::json!(drive.connection_selector());
                        Ok(value)
                    })
                    .collect::<Result<Vec<_>>>()?;
                println!("{}", serde_json::to_string_pretty(&descriptions)?);
            } else {
                print!("{}", console::devices(&devices));
            }
        }
        Commands::Erase {
            device,
            quick,
            yes,
            dry_run,
        } => {
            let device = selection::preferred(device.as_deref())?;
            device::erase(device.as_deref(), quick, dry_run, |drive| {
                use std::io::{self, IsTerminal, Write};
                print!("{}", console::devices(std::slice::from_ref(drive)));
                println!(
                    "{} erase will destroy all contents on this CD-RW.",
                    if quick { "Quick" } else { "Full" }
                );
                if yes {
                    return Ok(true);
                }
                ensure!(
                    io::stdin().is_terminal(),
                    "interactive confirmation required; use --yes only to explicitly approve erasing this CD-RW"
                );
                print!("Type ERASE to continue: ");
                io::stdout().flush()?;
                let mut answer = String::new();
                io::stdin().read_line(&mut answer)?;
                Ok(answer.trim() == "ERASE")
            })?;
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

fn resume_job(directory: &Path, id: Option<&str>) -> Result<()> {
    let saved = burn::Journal::load(directory).with_context(|| {
        format!("cannot read {}; burn expects an album TOML file with --output, or an existing job directory", directory.join("journal.json").display())
    })?;
    let id = id.map(|id| {
        device::resolve_id(id).with_context(|| format!("--device does not match the recording job's saved drive {} or is not connected", saved.device))
    }).transpose()?;
    ensure!(
        id.as_deref().is_none_or(|id| id == saved.device),
        "--device does not match the recording job's saved drive: {}",
        saved.device
    );
    let mut recorder = device::recorder(&saved.device)?;
    device::install_cancel_handler()?;
    console::phase(
        "burn",
        format_args!("{} | {}", directory.display(), console::speed(saved.speed)),
    );
    let result = burn::resume(&mut *recorder, directory)?;
    recording_complete(&result, directory);
    Ok(())
}

fn verify_bundle(directory: &Path, output: Option<&Path>, size: u32, samples: u32) -> Result<()> {
    let manifest = author::verify(directory)?;
    println!(
        "Verified {} assets, {} ISO files, {} Mode 1 sectors.",
        manifest.assets.len(),
        manifest.files.len(),
        manifest.data_sectors
    );
    if let Some(output) = output {
        preview::readback(directory, &manifest, output, size, samples)?;
        println!(
            "Saved {} — encoded payload projected from the local ISO, not a physical-disc readback.",
            output.display()
        );
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
    let output = output.context("a new recording project requires --output with a new job directory; use --dry-run to prepare without writing")?;
    burn::check_destination(&output)?;
    let id = selection::preferred(id.as_deref())?;
    let id = id.as_deref().map(device::resolve_id).transpose()?;
    device::install_cancel_handler()?;
    let mut recorder = match (blank_cdr, id.as_deref()) {
        (Some(capacity), id) => device::blank_cdr_recorder(id, capacity)?,
        (None, id) => {
            let drives;
            let id = match id {
                Some(id) => id,
                None => {
                    ensure!(
                        cfg!(target_os = "macos"),
                        "Recording is implemented for macOS only"
                    );
                    drives = device::list()?;
                    &burn::select_blank(&drives)?.id
                }
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
        println!("To write the job: discimage burn {}", output.display());
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merged_commands_keep_legacy_entry_points_hidden() {
        let command = Cli::command();
        for name in ["prepare", "resume", "readback"] {
            assert!(command.find_subcommand(name).unwrap().is_hide_set());
        }
        for args in [
            vec!["discimage", "select"],
            vec!["discimage", "select", "GX40N"],
            vec!["discimage", "select", "--clear"],
            vec!["discimage", "calibrate", "probe", "--check"],
            vec!["discimage", "burn", "album.toml", "-o", "job", "--dry-run"],
            vec!["discimage", "burn", "job"],
            vec!["discimage", "verify", "bundle"],
            vec![
                "discimage",
                "verify",
                "bundle",
                "-o",
                "payload.png",
                "--samples",
                "128",
            ],
            vec!["discimage", "prepare", "album.toml", "-o", "job"],
            vec!["discimage", "resume", "job"],
            vec!["discimage", "readback", "bundle", "-o", "payload.png"],
        ] {
            Cli::try_parse_from(args).unwrap();
        }
        assert!(
            Cli::try_parse_from(["discimage", "verify", "bundle", "--samples", "128"]).is_err()
        );
        assert!(Cli::try_parse_from(["discimage", "select", "GX40N", "--clear"]).is_err());
    }

    #[test]
    fn job_options_cannot_override_frozen_recording_inputs() {
        let directory =
            std::env::temp_dir().join(format!("discimage-job-options-{}", std::process::id()));
        std::fs::create_dir(&directory).unwrap();
        for flags in [
            vec!["--speed", "10"],
            vec!["--output", "new"],
            vec!["--dry-run"],
            vec!["--blank-cdr", "360000"],
        ] {
            let args = ["discimage", "burn", directory.to_str().unwrap()];
            let error =
                run(Cli::try_parse_from(args.into_iter().chain(flags)).unwrap()).unwrap_err();
            assert!(error.to_string().contains("saved inputs and speed"));
        }
        std::fs::remove_dir(directory).unwrap();
    }

    #[test]
    fn completion_scripts_include_current_commands_and_flags() {
        for shell in [
            clap_complete::Shell::Zsh,
            clap_complete::Shell::Bash,
            clap_complete::Shell::Fish,
            clap_complete::Shell::PowerShell,
            clap_complete::Shell::Elvish,
        ] {
            let mut bytes = Vec::new();
            clap_complete::generate(shell, &mut Cli::command(), "discimage", &mut bytes);
            let script = String::from_utf8(bytes).unwrap();
            for token in ["burn", "verify", "erase", "select", "completions", "speed"] {
                assert!(script.contains(token), "{shell}: missing {token}");
            }
        }
    }

    #[test]
    fn recording_speed_defaults_to_auto_and_validates_explicit_values() {
        for command in ["prepare", "burn"] {
            let args = ["discimage", command, "album.toml", "--output", "job"];
            let parsed = Cli::try_parse_from(args).unwrap();
            let recording = match parsed.command {
                Commands::Prepare(recording) | Commands::Burn { recording, .. } => recording,
                _ => unreachable!(),
            };
            assert_eq!(recording.speed, None);
            for speed in ["0", "65"] {
                assert!(Cli::try_parse_from(args.into_iter().chain(["--speed", speed])).is_err());
            }
            let parsed = Cli::try_parse_from(args.into_iter().chain(["--speed", "10"])).unwrap();
            let recording = match parsed.command {
                Commands::Prepare(recording) | Commands::Burn { recording, .. } => recording,
                _ => unreachable!(),
            };
            assert_eq!(recording.speed, Some(10));
        }
    }
}
