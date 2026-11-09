# Timing and photo phase

[简体中文](zhcn/timing.md) · [Guides](README.md) · [Calibration steps](calibration.md)

## What each measurement determines

| Measurement | Refines | Does not establish |
| --- | --- | --- |
| Read timing | Spiral geometry and angle differences across encoder positions | CD read-origin phase or label orientation |
| Three-band photo | Physical radius scale, preserving timing angles | Angular phase |
| Known angular features in a photo | Remaining along-track phase offset | Absolute physical accuracy from a low residual alone |

The constant-pitch model uses `u`, `v` and channel position `n`:
`S = sqrt(u² + v*n/π)`, `Theta(n) = 2*n/(S+u)`.
Single-sector read completion is modeled at `(LBA+1)*57624 + 108*588` bits.
The automatic preset assumes that 108-frame origin and a 58 mm outer radius;
photos can refine the scale or phase through separate measurements.

## Uncertainty and repeatability

Each round uses per-target phase standard errors and weighted residuals.
The [DiscProbe solver](https://github.com/Ashley-QwQ/disc-probe/blob/fa64fd60498c3eac8a6683828384d62082f0af65/discprobe/geo.py)
provides the underlying geometry method. DiscImage reconstructs data-only
covariance, marginalizes phase intercept and excludes geometry priors.
It inflates covariance by `max(1, chi2/(n−3))` and propagates it with the shared
spiral Jacobian. RMS alone never decides export.

Two independent fits are compared with `Q = deltaᵀ(C1+C2)⁻¹delta`.
The nominal 99% chi-square(2) limit is 9.21034; 3–8 explicit rounds use Bonferroni
correction over all pairs. All pairs must agree. The smallest propagated band
selects the exported round, with coverage restricted to the measured intersection.
This is a linearized, model-dependent statistical check, not a universal physical
accuracy guarantee. See [uncertainty propagation](https://www.nist.gov/pml/nist-technical-note-1297/nist-tn-1297-appendix-law-propagation-uncertainty)
and [chi-square values](https://itl.nist.gov/div898/handbook/eda/section3/eda3674.htm).

The CLI reports RMS, chi2/dof, measured-span sigma, maximum nominal 99% angle band,
displacement at 50 mm, read scatter in degrees/microseconds, and pair agreement.
The same timing jitter spans more degrees at higher spindle speed. Both operating
systems use the same calculation. [Measured results and manual comparison](validation.md).

Export still rejects changed/incomplete receipts, wrong targets, copied reports,
invalid uncertainties, unverified media reads, unchecked/changing rotation,
insufficient coverage, unidentifiable covariance and ambiguous winding.
The sampler keeps its safe-ladder and whole-turn checks. Systematic model bias
and physical artwork alignment require independent inspection.

## Reuse reports offline

```sh
discimage calibrate -o calibrations/review --name my-batch \
  --bundle calibrations/my-batch/record \
  --timing calibrations/my-batch/timing-previous-001/summary.json \
  --timing calibrations/my-batch/timing/summary.json
```

Keep each report's `targets.csv` and completed `status.json` beside its summary.
Fresh acquisition requires a finalized single-session, single-track Mode 1 CD.
The probe requests 1411 kB/s and takes 32 reference/target pairs per point.
`calibrate probe --check` inspects metadata without a timing sweep.

## Photo phase refinement

Use known angular features and the exact package that generated their design.
The CLI needs externally extracted phase correspondences, not a photo alone:
at least 12 samples per curve and multiple curves for held-out validation.
This is different from radial `points.json`.

```sh
discimage calibrate phase --timing measurement/summary.json \
  --bundle record --points phase.json --evidence photo.jpg --output comparison

discimage calibrate import album.toml \
  --timing measurement/summary.json --phase comparison/report.json \
  --bundle record --output candidate
```

Use new output directories and the original album geometry. Open
`comparison/index.html`, inspect the fits, then use `candidate/candidate.toml`
with `preview`/`build`. The website **时序对比** tab displays the candidate files.
Import requires verified photo linkage and held-out checks; it exports a separate
candidate, never automatic production-preset acceptance.

<details>
<summary>Phase measurement format</summary>

```json
{
  "format": "discimage-phase-v1",
  "bundle_sha256": "SHA256 of the exact manifest.toml",
  "evidence_sha256": "SHA256 of the photograph or rectified array",
  "convention": "photo-minus-design-clockwise",
  "samples": [{"curve": "edge-a", "sector": 180000.5, "phase_deg": 12.8}]
}
```

Expand the sample list for each curve. `sector` is the fractional encoder coordinate,
not a file offset or a photo radius. `phase_deg` is continuously unwrapped
photo-minus-design angle, clockwise. Match the same feature within each curve.
Independent curve rotations are allowed. Review image correspondences and
rectification sensitivity; repeated AI estimates are not independent photos.

</details>
