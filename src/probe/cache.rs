// SPDX-License-Identifier: GPL-3.0-or-later
//! CD cache strategy adapted from DiscProbe timing.py and geo.py.
use anyhow::{Context, Result, ensure};
use serde::Serialize;
use serde_json::{Value, json};

use super::{
    Command, Probe, READ_CHUNK, SECTOR_BYTES, check_cancel, console,
    transport::{READ_SPEED_KBPS, Reply},
};

const CACHE_HIT_US: f64 = 3000.0;
const CACHE_RATIO: f64 = 3.0;
const CACHE_READS: usize = 40;
const EVICT_SECTORS: u32 = 256;
const EVICT_REGION: u32 = 32_768;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Cache {
    Plain,
    Fua,
    Flush,
    Evict,
}

impl Cache {
    fn name(self) -> &'static str {
        match self {
            Self::Plain => "plain",
            Self::Fua => "fua",
            Self::Flush => "flush",
            Self::Evict => "evict",
        }
    }
}

#[derive(Serialize)]
struct Stats {
    n: usize,
    failed: usize,
    p50_us: f64,
    p95_us: f64,
    p99_us: f64,
    min_us: f64,
    max_us: f64,
}

fn stats(replies: &[Reply]) -> Stats {
    let mut durations: Vec<_> = replies.iter().map(|reply| reply.duration * 1e6).collect();
    durations.sort_by(f64::total_cmp);
    let percentile = |p: f64| {
        let i = (p * (durations.len() - 1) as f64).round() as usize;
        (durations[i] * 10.0).round() / 10.0
    };
    Stats {
        n: durations.len(),
        failed: replies.iter().filter(|r| r.status != 0).count(),
        p50_us: percentile(0.5),
        p95_us: percentile(0.95),
        p99_us: percentile(0.99),
        min_us: percentile(0.0),
        max_us: percentile(1.0),
    }
}

impl Probe<'_> {
    fn align(&self, percent: u32) -> u32 {
        self.bounds.start + (self.bounds.end - self.bounds.start - 1) * percent / 100
    }

    fn read(&mut self, lba: u32, count: u8, fua: bool) -> Result<Reply> {
        self.check_read(lba, count)?;
        let reply = self
            .link
            .command(Command::Read {
                lba,
                count,
                fua,
                read12: self.read12,
            })
            .with_context(|| format!("could not read LBA {lba}"))?;
        let reply = active_media(reply)?;
        ensure!(
            reply.status != 0 || reply.data.len() == usize::from(count) * SECTOR_BYTES,
            "short data transfer at LBA {lba}"
        );
        Ok(reply)
    }

    fn flush(&mut self, lba: u32) -> Result<Reply> {
        self.check_read(lba, 1)?;
        active_media(self.link.command(Command::Flush(lba))?)
    }

    pub(super) fn setup_reads(&mut self) -> Result<Value> {
        let lba = self.align(50);
        let initial = self.read(lba, 1, false)?;
        if initial.status != 0 && initial.key() == 5 && matches!(initial.asc().0, 0x20 | 0x24) {
            self.read12 = false;
        } else {
            initial.good()?;
        }
        self.read(lba, 1, false)?.good()?;
        check_cancel()?;
        let speed = active_media(self.link.command(Command::Speed)?)?;
        if speed.status != 0 {
            console::warning(format_args!(
                "Drive rejected the timing read-speed request ({READ_SPEED_KBPS} kB/s); using current read speed"
            ));
        }
        for percent in [0, 25, 50, 75, 100, 5] {
            self.read(self.align(percent), 1, false)?.good()?;
        }
        Ok(json!({
            "requested_kBps": READ_SPEED_KBPS, "policy": "conservative", "pure_cav": false,
            "status": speed.status, "sense": speed.sense,
        }))
    }

    pub(super) fn check_cache(&mut self) -> Result<Value> {
        console::phase("probe", format_args!("Testing drive read cache"));
        let lba = self.align(50);
        let mut repeated = |fua| -> Result<Stats> {
            let reads = (0..CACHE_READS)
                .map(|_| self.read(lba, 1, fua))
                .collect::<Result<Vec<_>>>()?;
            Ok(stats(&reads))
        };
        let plain = repeated(false)?;
        let fua = repeated(true)?;
        let fua_media = fua.p50_us >= CACHE_HIT_US && fua.failed == 0;
        // Slow USB bridges can take >3 ms even for cached sectors. Compare FUA
        // with plain reads, as DiscProbe does, before declaring them uncached.
        let plain_cached =
            plain.p50_us < CACHE_HIT_US || (fua_media && fua.p50_us >= CACHE_RATIO * plain.p50_us);
        let plain_good = plain.failed == 0;
        let mut out = json!({
            "lba": lba, "sectors": 1, "plain": plain, "fua": fua,
            "plain_served_from_cache": plain_cached, "fua_reads_media": fua_media,
            "flush_then_read_media": false,
        });
        self.cache = if !plain_cached {
            ensure!(plain_good, "ordinary reads failed during the cache test");
            Cache::Plain
        } else if fua_media {
            Cache::Fua
        } else {
            let mut reads = Vec::with_capacity(CACHE_READS);
            let mut flush_failed = 0;
            for _ in 0..CACHE_READS {
                flush_failed += usize::from(self.flush(lba)?.status != 0);
                reads.push(self.read(lba, 1, false)?);
            }
            let flushed = stats(&reads);
            let flush_media =
                flush_failed == 0 && flushed.failed == 0 && flushed.p50_us >= CACHE_HIT_US;
            out["after_flush"] = json!(flushed);
            out["flush_failed"] = json!(flush_failed);
            out["flush_then_read_media"] = json!(flush_media);
            if flush_media {
                Cache::Flush
            } else {
                Cache::Evict
            }
        };
        out["strategy"] = json!(self.cache.name());
        if self.cache == Cache::Evict {
            out["evict_sectors"] = json!(EVICT_SECTORS);
        }
        Ok(out)
    }

    fn evict(&mut self) -> Result<()> {
        let start = self.align(60);
        let region = EVICT_REGION.min(self.bounds.end - start);
        let mut remaining = EVICT_SECTORS;
        while remaining != 0 {
            let count = remaining
                .min(u32::from(READ_CHUNK))
                .min(region - self.evict_cursor);
            self.read(start + self.evict_cursor, count as u8, false)?
                .good()?;
            self.evict_cursor = (self.evict_cursor + count) % region;
            remaining -= count;
        }
        Ok(())
    }

    pub(super) fn sample(&mut self, lba: u32) -> Result<Option<f64>> {
        self.check_read(lba, 1)?;
        match self.cache {
            Cache::Flush => {
                self.flush(lba)?.good()?;
            }
            Cache::Evict => self.evict()?,
            _ => {}
        }
        let reply = self.read(lba, 1, self.cache == Cache::Fua)?;
        let indirect = matches!(self.cache, Cache::Flush | Cache::Evict);
        Ok(
            (reply.status == 0 && (!indirect || reply.duration * 1e6 >= CACHE_HIT_US))
                .then_some(reply.end),
        )
    }
}

fn active_media(reply: Reply) -> Result<Reply> {
    // A changed or unavailable medium invalidates the bounds established at open.
    if reply.status != 0 && matches!(reply.key(), 2 | 3 | 4 | 6) {
        return reply
            .good()
            .context("recorded CD is no longer ready or cannot be read reliably");
    }
    Ok(reply)
}
