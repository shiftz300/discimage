use std::path::{Path, PathBuf};

use anyhow::{Context, Result, ensure};
use clap::{CommandFactory, Parser, Subcommand};
use discimage::{
    author, burn, calibrate, console, device,
    layout::Plan,
    package::{self, Package},
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
    after_help = "Design: preview PROJECT -> template PROJECT -o DIR\nPrepare: build PROJECT [--iso] [--raw]\nRecord on macOS: burn PACKAGE"
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
    /// Write or continue a prepared recording job (macOS).
    Burn {
        /// Recording job directory created by build.
        #[arg(value_name = "JOB")]
        directory: PathBuf,
        /// Drive ID; it must match the drive saved in the job.
        #[arg(long)]
        device: Option<String>,
        /// Write speed multiplier; frozen once recording starts. Omit for automatic.
        #[arg(long, value_parser = clap::value_parser!(u32).range(1..=64))]
        speed: Option<u32>,
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
    /// Build a flat recording package without a drive; optionally export ISO/RAW.
    #[command(group(clap::ArgGroup::new("images").multiple(true)))]
    Build {
        /// Album TOML file.
        project: PathBuf,
        /// Output directory; defaults to output/<project name> beside the TOML.
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Export the 2048-byte data-session ISO.
        #[arg(long, group = "images")]
        iso: bool,
        /// Export 2352-byte Mode 1 RAW data.
        #[arg(long, group = "images")]
        raw: bool,
        /// Use a measured session 2 start; otherwise exports are estimates.
        #[arg(long, requires = "images")]
        session_start: Option<u32>,
    },
    /// Verify an offline bundle; optionally export its encoded-payload preview.
    Verify {
        directory: PathBuf,
        /// Migrate a verified completed legacy job into the flat layout.
        #[arg(long)]
        compact: bool,
        /// Export a preview from the local ISO, not the physical disc.
        #[arg(short, long)]
        output: Option<PathBuf>,
        #[arg(long, requires = "output", default_value_t = 1024)]
        size: u32,
        #[arg(long, requires = "output", default_value_t = 32)]
        samples: u32,
    },
    /// Show album timing and artwork space; optionally export a PNG/SVG preview.
    Preview {
        /// Album TOML file.
        project: PathBuf,
        #[arg(long)]
        session_start: Option<u32>,
        /// Export PNG or SVG; omit to print the layout summary.
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Print the layout estimate as JSON without accessing a drive.
        #[arg(long)]
        json: bool,
        /// Resolution of the raster disc preview (256–4096).
        #[arg(long, requires = "output", default_value_t = 1024)]
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
            completions(shell, &mut std::io::stdout())?;
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
        Commands::Burn {
            directory,
            device,
            speed,
        } => resume_job(&directory, device.as_deref(), speed)?,
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
            iso,
            raw,
            session_start,
        } => {
            let output = output
                .map(Ok)
                .unwrap_or_else(|| package::default_output(&project))?;
            let images = match (iso, raw) {
                (true, true) => Some(author::Images::Both),
                (true, false) => Some(author::Images::Iso),
                (false, true) => Some(author::Images::Raw),
                _ => None,
            };
            package::build(&project, &output, images, session_start)?;
            console::success(format_args!(
                "Built {} without accessing a drive",
                output.display()
            ));
            println!("Preview: {}", output.join("preview.svg").display());
            println!("To record: discimage burn {}", output.display());
            if images.is_some() && session_start.is_none() {
                console::warning(format_args!(
                    "Exported data image uses an estimated session start; recording will read the actual NWA"
                ));
            }
        }
        Commands::Verify {
            directory,
            output,
            size,
            samples,
            compact,
        } => {
            if compact {
                package::migrate(&directory)?;
            }
            verify_bundle(&directory, output.as_deref(), size, samples)?;
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
            json,
            size,
        } => preview_project(&project, session_start, output.as_deref(), size, json)?,
    }
    Ok(())
}

fn preview_project(
    project: &Path,
    session_start: Option<u32>,
    output: Option<&Path>,
    size: u32,
    json: bool,
) -> Result<()> {
    let (plan, artwork) = prepare(project, session_start)?;
    if let Some(output) = output {
        preview::save(&plan, artwork.as_ref(), output, size)?;
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
            plan.artwork_sectors, plan.artwork_inner_radius_mm, plan.artwork_outer_radius_mm
        );
        println!(
            "{}. This command does not access the optical drive.",
            plan.geometry_label()
        );
        if let Some(output) = output {
            println!(
                "Saved {} — design preview only, not a burn image.",
                output.display()
            );
        }
    }
    Ok(())
}

// Static generators do not consistently honor clap's hidden items. Project
// the visible CLI from the same command definitions instead of maintaining a list.
fn completion_command(command: &clap::Command) -> clap::Command {
    let mut visible = clap::Command::new(command.get_name().to_owned())
        .disable_help_flag(command.is_disable_help_flag_set())
        .disable_version_flag(command.is_disable_version_flag_set())
        .disable_help_subcommand(command.is_disable_help_subcommand_set())
        .visible_aliases(command.get_visible_aliases().map(str::to_owned))
        .groups(command.get_groups().cloned())
        .args(
            command
                .get_arguments()
                .filter(|arg| !arg.is_hide_set())
                .cloned(),
        )
        .subcommands(
            command
                .get_subcommands()
                .filter(|sub| !sub.is_hide_set())
                .map(completion_command),
        );
    if let Some(about) = command.get_about() {
        visible = visible.about(about.clone());
    }
    if let Some(version) = command.get_version() {
        visible = visible.version(version.to_owned());
    }
    visible
}

fn completions(shell: clap_complete::Shell, output: &mut impl std::io::Write) -> Result<()> {
    if shell == clap_complete::Shell::Zsh {
        // clap_complete guards helper definitions, so sourcing a newer script
        // otherwise leaves the old command lists in the current zsh process.
        output.write_all(
            br#"#compdef discimage
() {
    local name
    for name in ${(k)functions}; do
        case "$name" in
            _discimage|_discimage_commands|_discimage__subcmd__*) unfunction "$name" ;;
        esac
    done
}

"#,
        )?;
    }
    clap_complete::generate(
        shell,
        &mut completion_command(&Cli::command()),
        "discimage",
        output,
    );
    Ok(())
}

fn resume_job(directory: &Path, id: Option<&str>, speed: Option<u32>) -> Result<()> {
    let flat = directory.join("manifest.toml").try_exists()?;
    let saved = if flat {
        Package::load(directory)?.journal
    } else {
        Some(burn::Journal::load(directory).with_context(|| {
            format!(
                "burn expects a built package; run 'discimage build PROJECT' first ({})",
                directory.display()
            )
        })?)
    };
    if let Some(saved) = &saved {
        ensure!(
            matches!(
                saved.stage,
                burn::Stage::Prepared | burn::Stage::MusicWritten | burn::Stage::DataReady
            ),
            "stage {:?} cannot be resumed automatically; no write attempted",
            saved.stage
        );
    }
    let selected = if let Some(saved) = &saved {
        if let Some(id) = id {
            ensure!(
                device::resolve_id(id).context(
                    "--device does not match the recording job's saved drive or is not connected"
                )? == saved.device,
                "--device does not match the recording's saved drive"
            );
        }
        saved.device.clone()
    } else {
        let selector = selection::preferred(id)?;
        let drives = device::list()?;
        if let Some(selector) = selector {
            device::resolve(&drives, &selector)?.id.clone()
        } else {
            burn::select_blank(&drives)?.id.clone()
        }
    };
    let mut recorder = device::recorder(&selected)?;
    device::install_cancel_handler()?;
    let journal = burn::record(&mut *recorder, directory, speed)?;
    recording_complete(&journal, directory);
    Ok(())
}

fn verify_bundle(directory: &Path, output: Option<&Path>, size: u32, samples: u32) -> Result<()> {
    if directory.join("manifest.toml").try_exists()? {
        let package = Package::load(directory)?;
        package.verify(directory)?;
        println!(
            "Verified {} frozen inputs{}.",
            package.inputs.len(),
            if package.image.is_some() {
                " and data image"
            } else {
                "; no data image generated yet"
            }
        );
        if let Some(output) = output {
            let image = package.image.context(
                "no encoded image yet; record the disc or export with build --iso/--raw first",
            )?;
            preview::readback(directory, &image, output, size, samples)?;
        }
        return Ok(());
    }
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
    fn public_workflow_is_unambiguous_and_build_never_takes_device_options() {
        for name in ["plan", "prepare", "resume", "readback"] {
            assert!(Cli::command().find_subcommand(name).is_none());
        }
        for args in [
            vec!["discimage", "select"],
            vec!["discimage", "select", "GX40N"],
            vec!["discimage", "build", "album.toml"],
            vec!["discimage", "build", "album.toml", "--iso"],
            vec!["discimage", "build", "album.toml", "--raw"],
            vec!["discimage", "build", "album.toml", "--iso", "--raw"],
            vec![
                "discimage",
                "build",
                "album.toml",
                "--iso",
                "--session-start",
                "12000",
            ],
            vec!["discimage", "preview", "album.toml"],
            vec!["discimage", "preview", "album.toml", "--json"],
            vec!["discimage", "preview", "album.toml", "-o", "preview.svg"],
            vec!["discimage", "burn", "output/album"],
            vec!["discimage", "verify", "output/album"],
        ] {
            Cli::try_parse_from(args).unwrap();
        }
        for command in ["build", "burn"] {
            for flags in [vec!["--offline"], vec!["--dry-run"]] {
                assert!(
                    Cli::try_parse_from(
                        ["discimage", command, "album.toml"]
                            .into_iter()
                            .chain(flags)
                    )
                    .is_err()
                );
            }
        }
        assert!(
            Cli::try_parse_from(["discimage", "build", "album.toml", "--device", "GX40N"]).is_err()
        );
        assert!(
            Cli::try_parse_from(["discimage", "build", "album.toml", "--speed", "10"]).is_err()
        );
        assert!(Cli::try_parse_from(["discimage", "burn", "job", "-o", "other"]).is_err());
        assert!(
            Cli::try_parse_from([
                "discimage",
                "build",
                "album.toml",
                "--session-start",
                "12000"
            ])
            .is_err()
        );
        assert!(
            Cli::try_parse_from(["discimage", "preview", "album.toml", "--size", "2048"]).is_err()
        );
        assert!(Cli::try_parse_from(["discimage", "select", "GX40N", "--clear"]).is_err());
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
            completions(shell, &mut bytes).unwrap();
            let script = String::from_utf8(bytes).unwrap();
            for token in ["burn", "verify", "erase", "select", "completions", "speed"] {
                assert!(script.contains(token), "{shell}: missing {token}");
            }
            let visible = completion_command(&Cli::command());
            for name in ["prepare", "resume", "readback", "plan"] {
                assert!(
                    visible.find_subcommand(name).is_none(),
                    "{shell}: hidden command {name}"
                );
            }
            assert!(!script.contains("--svg"));
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn zsh_reload_replaces_stale_helpers_without_removing_custom_functions() {
        use std::{
            io::Write,
            process::{Command, Stdio},
        };
        let mut script = b"autoload -Uz compinit\ncompinit -D\n_describe() { print -rl -- \"${commands[@]}\"; }\n_discimage_custom() { print keep; }\n".to_vec();
        for _ in 0..2 {
            script.extend_from_slice(b"_discimage_commands() { print stale; }\n_discimage__subcmd__calibrate_commands() { print stale; }\n");
            completions(clap_complete::Shell::Zsh, &mut script).unwrap();
            script.extend_from_slice(
                br#"
[[ $(_discimage_commands) == *select:* ]] || exit 11
[[ $(_discimage_commands) != *stale* ]] || exit 12
[[ $(_discimage_commands) != *resume:* ]] || exit 13
[[ $(_discimage__subcmd__calibrate_commands) == *probe:* ]] || exit 14
[[ $(_discimage_custom) == keep ]] || exit 15
[[ $_comps[discimage] == _discimage ]] || exit 16
"#,
            );
        }
        let mut child = Command::new("zsh")
            .args(["-f", "-s"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(&script).unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "zsh reload failed: {:?}\n{}",
            output.status.code(),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn recording_speed_defaults_to_auto_and_validates_explicit_values() {
        for (value, valid) in [("0", false), ("65", false), ("10", true)] {
            assert_eq!(
                Cli::try_parse_from(["discimage", "burn", "job", "--speed", value]).is_ok(),
                valid
            );
        }
        let Cli {
            command: Commands::Burn { speed, .. },
        } = Cli::try_parse_from(["discimage", "burn", "job"]).unwrap()
        else {
            unreachable!()
        };
        assert_eq!(speed, None);
    }
}
