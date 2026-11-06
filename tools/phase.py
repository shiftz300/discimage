#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Offline CD timing/photo comparison; Python 3.9+, standard library only.

Uses the constant-pitch u/v model described by Ashley-QwQ/disc-probe and the
curve-validation approach in DiscImageStudio issue #49. This is an analysis
tool, not a drive backend, photo rectifier or production-preset exporter.
"""

import argparse
import csv
import hashlib
import html
import json
import math
from pathlib import Path

FRAME_BITS = 588
SECTOR_BITS = 98 * FRAME_BITS
CIRC_ORIGIN = 111  # Forward CIRC frame minus discimage inverse-map frame.


def require(condition, message):
    if not condition:
        raise ValueError(message)


def number(value):
    require(isinstance(value, (int, float)) and not isinstance(value, bool)
            and math.isfinite(value), "expected a finite number")
    return float(value)


def mean(values):
    return math.fsum(values) / len(values)


def rms(values):
    return math.sqrt(mean([x * x for x in values]))


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def load_timing(path):
    """Normalize measured coverage without confusing it with readable capacity.

    Keep the input summary unchanged; report hashes always refer to that input.
    Both the Mac CD branch and the earlier CD Extra experiment emit targets.csv.
    """
    summary = json.loads(path.read_text(encoding="utf-8"))
    with path.with_name("targets.csv").open(encoding="utf-8", newline="") as stream:
        rows = list(csv.DictReader(stream))
    sectors = [int(row["lba"]) for row in rows if int(row["used"]) > 0]
    require(len(set(sectors)) >= 8, "need at least eight measured timing targets")
    sectors.append(int(summary["fit"]["reference_lba"]))
    measured = [min(sectors), max(sectors) + 1]
    for key in ("experiment_range", "readable_lba_range"):
        bounds = summary["setup"].get(key)
        if bounds is not None:
            require(len(bounds) == 2 and bounds[0] <= measured[0] < measured[1] <= bounds[1],
                    f"targets outside {key}")
    summary["setup"]["experiment_range"] = measured
    return summary


class Model:
    def __init__(self, summary, manifest):
        fit = summary["fit"]
        require(fit["media"] == "cd" and fit["sector_bits"] == SECTOR_BITS,
                "only CD timing geometry is supported, not DVD")
        require(fit["read_tail_frames"] == 108,
                "unknown read-completion origin; do not silently mix conventions")
        self.u, self.v = number(fit["u"]), number(fit["v"])
        require(self.u > 0 and self.v > 0, "invalid timing geometry")
        self.range = list(map(number, summary["setup"]["experiment_range"]))
        require(len(self.range) == 2 and 0 <= self.range[0] < self.range[1],
                "missing measured LBA range")
        require(manifest["format"] == "discimage-extra-v1", "unknown bundle format")
        plan = manifest["plan"]
        self.art_range = [plan["estimated_art_start_sector"], plan["estimated_art_end_sector"]]
        disc = plan["disc"]
        geometry = disc.get("calibration") or disc
        self.inner = number(geometry["inner_radius_mm"])
        outer = number(geometry["outer_radius_mm"])
        self.capacity = number(geometry.get("reference_sectors", disc["capacity_sectors"]))
        self.velocity = number(disc["velocity"])
        require(0 < self.inner < outer and self.capacity > 0 and self.velocity > 0,
                "invalid generating geometry")
        self.span = outer * outer - self.inner * self.inner
        self.anchor = mean(self.range)

    def drift(self, sector, shift):
        """Photo minus design, clockwise; shift is in 588-bit channel frames.

        Use angle differences directly, avoiding subtraction of huge cumulative
        angles. One free photo rotation is removed separately for each curve.
        Neither the global disc.angle nor the photo orientation is calibrated.
        """
        n, ref = sector * SECTOR_BITS, self.anchor * SECTOR_BITS
        base = self.u * self.u + self.v * shift * FRAME_BITS / math.pi
        require(base > 0, "shift lies outside the constant-pitch model")
        measured = 2 * (n - ref) / (
            math.sqrt(base + self.v * n / math.pi)
            + math.sqrt(base + self.v * ref / math.pi))
        generated = (2 * self.velocity / 75 * (sector - self.anchor)
                     / (math.sqrt(self.inner**2 + sector / self.capacity * self.span)
                        + math.sqrt(self.inner**2 + self.anchor / self.capacity * self.span)))
        return math.degrees(measured - generated)


def curves(points, model, manifest_hash):
    require(points["format"] == "discimage-phase-v1", "unknown phase points format")
    require(points["bundle_sha256"] == manifest_hash, "points belong to a different bundle")
    require(points["convention"] == "photo-minus-design-clockwise",
            "unknown photo angle convention")
    photo = points["evidence_sha256"]
    require(isinstance(photo, str) and len(photo) == 64
            and all(c in "0123456789abcdef" for c in photo), "missing photo/rectification evidence hash")
    groups = {}
    for point in points["samples"]:
        name = point["curve"]
        require(isinstance(name, str) and name.strip(), "missing curve identifier")
        sector, phase = number(point["sector"]), number(point["phase_deg"])
        require(model.range[0] <= sector <= model.range[1], "point outside measured timing range")
        require(model.art_range[0] <= sector < model.art_range[1], "point outside artwork range")
        groups.setdefault(name, []).append((sector, phase))
    require(groups, "no phase points")
    for samples in groups.values():
        samples.sort()
        require(len(samples) >= 12 and samples[-1][0] - samples[0][0] >= 1000,
                "each curve needs at least 12 points spanning 1000 sectors")
        require(all(b[0] > a[0] and abs(b[1] - a[1]) < 180
                    for a, b in zip(samples, samples[1:])),
                "duplicate sectors or wrapped/discontinuous phase; unwrap the same edge first")
    return groups


def evaluate(model, samples, shift):
    raw = [model.drift(s, shift) for s, _ in samples]
    rotation = mean([observed - predicted for (_, observed), predicted in zip(samples, raw)])
    predicted = [value + rotation for value in raw]
    errors = [prediction - observation for prediction, (_, observation) in zip(predicted, samples)]
    return {"rms_deg": rms(errors), "max_deg": max(map(abs, errors)),
            "rotation_deg": rotation, "predicted_deg": predicted}


def fit(model, groups):
    """Fit a single effective along-track shift; one nuisance rotation per curve.

    A finite grid checks for an interior, unique minimum before golden-section
    refinement. This tool deliberately does not fit higher-order pitch terms.
    """
    def cost(shift):
        return mean([evaluate(model, samples, shift)["rms_deg"] ** 2
                     for samples in groups.values()])
    grid = [-1024 + i * 32 for i in range(65)]
    costs = [cost(x) for x in grid]
    minima = [i for i in range(1, 64) if costs[i] < costs[i-1] and costs[i] < costs[i+1]]
    require(len(minima) == 1, "no unique interior shift fit; inspect points and conventions")
    index = minima[0]
    require(costs[index] < min(costs[0], costs[-1]), "best shift is outside the search range")
    lo, hi = grid[index-1], grid[index+1]
    ratio = (math.sqrt(5) - 1) / 2
    for _ in range(70):
        a, b = hi - ratio * (hi-lo), lo + ratio * (hi-lo)
        if cost(a) < cost(b):
            hi = b
        else:
            lo = a
    shift = (lo + hi) / 2
    return {"shift_frames": shift, "forward_offset_frames": CIRC_ORIGIN - shift,
            "rms_deg": math.sqrt(cost(shift))}


def analyze(model, groups, offset=None):
    joint = fit(model, groups) if offset is None else {
        "shift_frames": CIRC_ORIGIN - number(offset), "forward_offset_frames": number(offset),
        "rms_deg": rms([evaluate(model, data, CIRC_ORIGIN - offset)["rms_deg"]
                        for data in groups.values()])}
    individual = {name: fit(model, {name: samples}) for name, samples in groups.items()}
    validation = []
    if len(groups) > 1:
        for held, samples in groups.items():
            trained = fit(model, {name: data for name, data in groups.items() if name != held}) if offset is None else joint
            checked = evaluate(model, samples, trained["shift_frames"])
            validation.append({"held_out_curve": held, "shift_frames": trained["shift_frames"],
                               "rms_deg": checked["rms_deg"], "max_deg": checked["max_deg"],
                               "photo_rotation_refitted": True, "shift_refitted": False})
    return {"format": "discimage-phase-report-v1", "joint": joint,
            "offset_source": "curve-fit" if offset is None else "external-measurement",
            "individual": individual, "holdouts": validation,
            "origin_frames": CIRC_ORIGIN, "measured_lba_range": model.range,
            "profile_applied": False, "physical_validation": False,
            "note": "Effective offset includes read completion and encoding conventions, not a measured hardware delay. "
                    "Curve holdouts share one photograph and do not test perspective systematics.",
            "curves": {name: {"samples": len(data), "lba_range": [data[0][0], data[-1][0]],
                              "naive_origin": evaluate(model, data, 0),
                              "forward_origin": evaluate(model, data, CIRC_ORIGIN),
                              "fitted": evaluate(model, data, joint["shift_frames"])}
                       for name, data in groups.items()}}


def page(report, groups):
    charts = []
    for name, data in groups.items():
        result = report["curves"][name]
        observed = [p for _, p in data]
        values = observed + result["forward_origin"]["predicted_deg"] + result["fitted"]["predicted_deg"]
        low, high = min(values) - .1, max(values) + .1
        first, last = data[0][0], data[-1][0]
        def x(s):
            return 60 + (s-first)/(last-first)*680
        def y(p):
            return 235 - (p-low)/(high-low)*195
        paths = []
        for key, color in [("forward_origin", "#ffb454"), ("fitted", "#66d9c3")]:
            coords = " ".join(f"{x(s):.2f},{y(p):.2f}" for (s, _), p in zip(data, result[key]["predicted_deg"]))
            paths.append(f'<polyline points="{coords}" fill="none" stroke="{color}" stroke-width="2"/>')
        dots = "".join(f'<circle cx="{x(s):.2f}" cy="{y(p):.2f}" r="2.5" fill="#eee"/>' for s, p in data)
        charts.append(f'''<section><h2>{html.escape(name)}</h2>
<p>111 帧原点换算 RMS {result['forward_origin']['rms_deg']:.3f}° → 联合拟合 RMS {result['fitted']['rms_deg']:.3f}°</p>
<svg viewBox="0 0 800 290" role="img" aria-label="照片角度与模型预测曲线">
<path d="M60 30V235H750" fill="none" stroke="#8892a6"/>
<text x="5" y="40">{high:.2f}°</text><text x="5" y="235">{low:.2f}°</text>
<text x="60" y="260">{first:.0f}</text><text x="680" y="260">{last:.0f}</text>
<text x="330" y="282">编码坐标 / 绝对扇区</text>{''.join(paths)}{dots}</svg></section>''')
    fit = report["joint"]
    fits = "".join(f"<li>{html.escape(name)} 单独拟合的上游沿轨偏移：{value['forward_offset_frames']:.3f} 帧</li>"
                   for name, value in report["individual"].items())
    evidence = html.escape(str(report.get("evidence", {}).get("original_photo", "见 report.json 中的来源哈希")))
    holds = "".join(f"<li>未参与拟合的 {html.escape(v['held_out_curve'])}：RMS {v['rms_deg']:.3f}°，最大 {v['max_deg']:.3f}°</li>"
                    for v in report["holdouts"])
    return f'''<!doctype html><html lang="zh-CN"><meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1"><title>CD 计时与照片核对</title>
<style>body{{margin:32px auto;padding:0 20px;max-width:960px;background:#111723;color:#e8edf6;font:16px/1.65 system-ui}}
section{{background:#1b2434;border-radius:12px;padding:20px;margin:20px 0}}h1,h2{{line-height:1.3}}svg{{width:100%;font-size:13px}}text{{fill:#bac6da}}a{{color:#66d9c3}}</style>
<h1>用已有盘片，核对剩余角度偏差</h1><p>离线分析 · 没有擦除或刻录 · 没有修改生产配置</p>
<section><b>有效偏移：{fit['shift_frames']:.3f} 帧</b>
<p>偏移来源：{html.escape(report.get('offset_source', 'curve-fit'))}。外部偏移仅在这些照片曲线上核对，不重新拟合。</p>
<p>现有编码与正向 CIRC 的原点差为 111 帧；换算到上游沿轨偏移约定：{fit['forward_offset_frames']:.3f} 帧。
这个数包含位置模型与读完成约定，不能解释为光驱固有延迟。</p>
<p>白点：照片观测；<span style="color:#ffb454">橙线：只统一原点</span>；<span style="color:#66d9c3">绿线：拟合一个剩余偏移</span>。
每条曲线只扣除整体旋转，不单独拟合斜率。下图是角度曲线比较，并非实物照片叠加验证。</p><ul>{fits}{holds}</ul></section>
{''.join(charts)}<p>不同曲线来自同一张照片，仍共享透视、定位和取点误差。低残差不等于整盘验证通过。可用 calibrate import 生成独立候选；本报告不自动应用配置。</p>
<p>照片来源：{evidence}</p>
<p>参考：<a href="https://github.com/JiaFeiMiao-K-Cat/DiscImageStudio/issues/49">Issue #49</a> ·
<a href="https://github.com/JiaFeiMiao-K-Cat/DiscImageStudio/discussions/143">CD 沿轨道偏移 #143</a></p></html>'''


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--timing", type=Path, required=True)
    parser.add_argument("--bundle", type=Path, required=True)
    parser.add_argument("--points", type=Path, required=True)
    parser.add_argument("--evidence", type=Path, help="Verify the original photo or rectified-array hash")
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--offset-frames", type=float, help="Externally measured forward offset D; validate without refitting it")
    parser.add_argument("--offset-evidence", type=Path, help="Required evidence file for an external offset")
    args = parser.parse_args()
    manifest = args.bundle / "manifest.json"
    load = lambda path: json.loads(path.read_text(encoding="utf-8"))
    model = Model(load_timing(args.timing), load(manifest))
    points = load(args.points)
    groups = curves(points, model, digest(manifest))
    if args.evidence:
        require(digest(args.evidence) == points["evidence_sha256"], "photo/rectification evidence checksum mismatch")
    require((args.offset_frames is None) == (args.offset_evidence is None),
            "supply both --offset-frames and --offset-evidence")
    if args.offset_frames is not None:
        require(-1024 <= CIRC_ORIGIN - number(args.offset_frames) <= 1024,
                "offset is outside the supported range")
    report = analyze(model, groups, args.offset_frames)
    report["sources"] = {"timing_sha256": digest(args.timing), "manifest_sha256": digest(manifest),
                         "points_sha256": digest(args.points), "evidence_sha256": points["evidence_sha256"]}
    report["sources"]["evidence_hash_verified"] = args.evidence is not None
    report["sources"]["targets_sha256"] = digest(args.timing.with_name("targets.csv"))
    if args.offset_evidence:
        report["sources"]["offset_evidence_sha256"] = digest(args.offset_evidence)
    report["evidence"] = {key: points[key] for key in ("evidence_kind", "original_photo", "rectification") if key in points}
    rendered = page(report, groups)
    args.output.mkdir(parents=True, exist_ok=False)
    (args.output / "report.json").write_text(json.dumps(report, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    (args.output / "index.html").write_text(rendered, encoding="utf-8")
    print(json.dumps(report["joint"], indent=2))
    print(f"Open {args.output / 'index.html'}; no profile applied.")


if __name__ == "__main__":
    try:
        main()
    except (ValueError, KeyError, OSError) as error:
        raise SystemExit(f"error: {error}")
