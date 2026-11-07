//! Resumable calibration prompts using the existing recording journal.
use std::{
    io::{self, IsTerminal, Write},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, ensure};
use clap::Args;
use discimage::{
    author,
    burn::{self, Journal, Stage},
    calibrate, device,
    package::{self, Package},
};

#[derive(Args)]
pub struct Options {
    /// Working directory. Reuse it to continue from the last step.
    #[arg(short, long)]
    output: PathBuf,
    /// Drive ID or unique model; overrides the default saved by select.
    #[arg(long)]
    device: Option<String>,
    /// Requested CD speed multiplier; omit for the drive-selected maximum.
    #[arg(long, value_parser = clap::value_parser!(u32).range(1..=64))]
    speed: Option<u32>,
    /// Proceed with recording without the interactive confirmation.
    #[arg(long, conflicts_with = "bundle")]
    write: bool,
    /// Use the verified target bundle corresponding to an externally recorded disc.
    #[arg(long, conflicts_with = "device")]
    bundle: Option<PathBuf>,
    /// Photo path, or place photo.jpg, photo.jpeg or photo.png in the working directory.
    #[arg(long)]
    photo: Option<PathBuf>,
    /// Exported measurements, or place points.json in the working directory.
    #[arg(long)]
    points: Option<PathBuf>,
}

pub fn run(options: Options) -> Result<()> {
    let directory = &options.output;
    std::fs::create_dir_all(directory)
        .with_context(|| format!("无法创建工作目录 {}", directory.display()))?;
    let directory = &std::fs::canonicalize(directory)?;
    let profile = directory.join("calibration.toml");
    if profile.try_exists()? {
        println!(
            "标定完成。把 {} 中的 [disc.calibration] 段加入专辑配置。",
            profile.display()
        );
        return Ok(());
    }
    let bundle = match options.bundle {
        Some(ref bundle) => bundle.clone(),
        None => match record(directory, &options)? {
            Some(bundle) => bundle,
            None => return Ok(()),
        },
    };
    let points = options
        .points
        .as_ref()
        .cloned()
        .unwrap_or_else(|| directory.join("points.json"));
    if options.points.is_some() || points.try_exists()? {
        let result = calibrate::fit(&bundle, &points, &profile)?;
        println!(
            "[3/3] 标定完成。拟合半径 {:.3}–{:.3} mm，RMS {:.3} mm。",
            result.inner_radius_mm, result.outer_radius_mm, result.rms_mm
        );
        println!(
            "把 {} 中的 [disc.calibration] 段加入专辑配置。",
            profile.display()
        );
        return Ok(());
    }
    let photo = match options.photo {
        Some(path) => Some(path),
        None => find_photo(directory)?,
    };
    let Some(photo) = photo else {
        println!(
            "[2/3] 从记录面垂直拍下整张标靶盘。把照片放到 {}，命名为 photo.jpg 或 photo.png，再运行同一条命令。",
            directory.display()
        );
        return Ok(());
    };
    let target_digest = calibrate::digest(&bundle)?;
    let target = directory.join(format!("mark-{}.json", &target_digest[..12]));
    if !target.try_exists()? {
        calibrate::mark(&bundle, &target)?;
    }
    println!(
        "[2/3] 打开 discimage-website 的「标定取点」，导入 {} 和 {}，取点后导出 points.json。",
        target.display(),
        photo.display()
    );
    println!(
        "把 points.json 移到 {}，再运行同一条命令。",
        directory.display()
    );
    Ok(())
}

fn find_photo(directory: &Path) -> Result<Option<PathBuf>> {
    let mut photo = None;
    for name in ["photo.jpg", "photo.jpeg", "photo.png"] {
        let path = directory.join(name);
        if path.try_exists()? {
            ensure!(
                photo.is_none(),
                "找到多张照片，请用 --photo 指定要取点的那一张"
            );
            photo = Some(path);
        }
    }
    Ok(photo)
}

fn record(directory: &Path, options: &Options) -> Result<Option<PathBuf>> {
    let job = directory.join("record");
    let target = directory.join("target");
    let config = target.join("target.toml");
    let journal = if job.join("manifest.toml").try_exists()? {
        let package = Package::load(&job)?;
        ensure!(
            package.project.disc.calibration_target,
            "现有成品不是标靶工程"
        );
        if let Some(journal) = &package.journal {
            if journal.stage == Stage::Complete {
                return Ok(Some(job));
            }
            ensure!(
                matches!(
                    journal.stage,
                    Stage::Prepared | Stage::MusicWritten | Stage::DataReady
                ),
                "当前刻录阶段无法自动继续"
            );
        }
        package.journal
    } else if job.try_exists()? {
        let journal = Journal::load(&job)?;
        if journal.stage == Stage::Complete {
            return Ok(Some(journal.actual(&job)));
        }
        ensure!(
            matches!(
                journal.stage,
                Stage::Prepared | Stage::MusicWritten | Stage::DataReady
            ),
            "当前刻录阶段无法自动继续"
        );
        ensure!(
            author::load(&job.join("planned"))?
                .plan
                .disc
                .calibration_target,
            "现有作业不是标靶工程"
        );
        Some(journal)
    } else {
        if !target.try_exists()? {
            calibrate::create(&target, 360_000)?;
        }
        package::build(&config, &job, None, None)?;
        println!(
            "[1/3] 标靶成品已准备。预览 {}",
            job.join("preview.svg").display()
        );
        None
    };
    ensure!(
        options.photo.is_none() && options.points.is_none(),
        "先完成标靶刻录，或用 --bundle 指定已刻制品，再提交照片或点位"
    );
    if !cfg!(target_os = "macos") {
        println!(
            "[1/3] 标靶工程已生成于 {}。可离线 build，外部刻录后用 --bundle 继续取点。",
            config.display()
        );
        return Ok(None);
    }
    let drives = device::list()?;
    let selector = if journal.is_some() {
        options.device.clone()
    } else {
        crate::selection::preferred(options.device.as_deref())?
    };
    let requested = selector
        .as_deref()
        .map(|id| device::resolve(&drives, id))
        .transpose()?;
    let id = journal
        .as_ref()
        .map(|j| j.device.as_str())
        .or(requested.map(|drive| drive.id.as_str()));
    let drive = match (id, drives.as_slice()) {
        (_, []) => {
            println!("[1/3] 等待光驱。请连接刻录机并放入盘片，然后重跑同一条命令。");
            return Ok(None);
        }
        (Some(id), _) => drives
            .iter()
            .find(|d| d.id == id)
            .context("作业指定的光驱未连接")?,
        (None, [drive]) => drive,
        (None, _) => {
            println!("[1/3] 找到多个光驱，请用 --device 指定一个。");
            for drive in drives {
                println!("  {}  {}", drive.selector(), drive.name);
            }
            return Ok(None);
        }
    };
    if let Some(requested) = requested {
        ensure!(drive.id == requested.id, "当前作业已绑定到另一台光驱");
    }
    let Some(media) = &drive.media else {
        println!(
            "[1/3] 已找到 {}。请插入盘片，再重跑同一条命令。",
            drive.name
        );
        return Ok(None);
    };
    let needs_blank = journal.as_ref().is_none_or(|j| j.stage == Stage::Prepared);
    if needs_blank && !media.blank {
        println!(
            "[1/3] {} 中的 {} 不是空白盘。请换入空白盘，或保存现有内容后手动擦除 CD-RW。",
            drive.name, media.kind
        );
        return Ok(None);
    }

    let mut recorder = device::recorder(&drive.id)?;
    device::install_cancel_handler()?;
    let prepared = journal.as_ref().is_none_or(|j| j.stage == Stage::Prepared);
    println!("[1/3] 光驱 {}，介质 {}。", drive.name, media.kind);
    if prepared {
        let preview = if journal
            .as_ref()
            .is_some_and(|j| j.format == "discimage-burn-v1")
        {
            job.join("planned/preview.svg")
        } else {
            job.join("preview.svg")
        };
        println!("成品已准备。预览 {}", preview.display());
    }
    if !options.write {
        if !io::stdin().is_terminal() {
            println!("作业已预演。请在终端重跑确认，或明确指定 --write。");
            return Ok(None);
        }
        let action = if prepared {
            "现在写入标靶"
        } else {
            "继续写入标靶的数据会话"
        };
        print!("{action}？输入 y 继续，直接回车暂停 [y/N] ");
        io::stdout().flush()?;
        let mut answer = String::new();
        io::stdin().read_line(&mut answer)?;
        if !answer.trim().eq_ignore_ascii_case("y") {
            return Ok(None);
        }
    }
    let complete = burn::record(&mut *recorder, &job, options.speed)?;
    Ok(Some(complete.actual(&job)))
}
